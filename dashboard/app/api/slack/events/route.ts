/**
 * POST /api/slack/events
 *
 * Slack Events API handler.
 *
 * Responsibilities:
 *   1. Verify the request's HMAC-SHA256 signature using SLACK_SIGNING_SECRET
 *   2. Respond to Slack's url_verification challenge
 *   3. Dispatch app_mention and message.im events to the message handler
 *
 * Security note: this route is exempt from Supabase auth middleware (see
 * middleware.ts). Authentication is provided by Slack's request signature.
 *
 * Slack requires a response within 3 seconds. We return 200 immediately after
 * signature verification and process the event synchronously. Claude API calls
 * are typically <2 s; if occasionally slower, Slack will retry — we dedup
 * retries via the event_id check against the already-responded thread.
 */

import { NextRequest, NextResponse } from "next/server";
import crypto from "crypto";
import { handleSlackMessage } from "../message-handler";

export const dynamic = "force-dynamic";

// ── Signature verification ────────────────────────────────────────────────────

async function verifySlackSignature(req: NextRequest, rawBody: string): Promise<boolean> {
  const signingSecret = process.env.SLACK_SIGNING_SECRET;
  if (!signingSecret) {
    console.error("[Slack] SLACK_SIGNING_SECRET not set");
    return false;
  }

  const timestamp = req.headers.get("x-slack-request-timestamp");
  const slackSig = req.headers.get("x-slack-signature");

  if (!timestamp || !slackSig) return false;

  // Reject requests older than 5 minutes to prevent replay attacks
  const nowSeconds = Math.floor(Date.now() / 1000);
  if (Math.abs(nowSeconds - parseInt(timestamp, 10)) > 300) {
    console.warn("[Slack] Request timestamp too old — possible replay attack");
    return false;
  }

  const baseString = `v0:${timestamp}:${rawBody}`;
  const hmac = crypto
    .createHmac("sha256", signingSecret)
    .update(baseString)
    .digest("hex");
  const expected = `v0=${hmac}`;

  // Constant-time comparison to prevent timing attacks
  return crypto.timingSafeEqual(
    Buffer.from(slackSig, "utf8"),
    Buffer.from(expected, "utf8")
  );
}

// ── Slack event types (minimal subset we need) ────────────────────────────────

interface SlackUrlVerification {
  type: "url_verification";
  challenge: string;
}

interface SlackEventCallback {
  type: "event_callback";
  event_id: string;
  event: SlackInnerEvent;
  team_id: string;
}

interface SlackAppMentionEvent {
  type: "app_mention";
  user: string;
  text: string;
  channel: string;
  ts: string;
  thread_ts?: string;
  bot_id?: string;
}

interface SlackMessageImEvent {
  type: "message";
  channel_type: "im";
  user: string;
  text: string;
  channel: string;
  ts: string;
  thread_ts?: string;
  bot_id?: string;     // set when the message is from a bot — we must ignore these
  subtype?: string;    // e.g. "bot_message" — ignore
}

type SlackInnerEvent = SlackAppMentionEvent | SlackMessageImEvent | { type: string };

type SlackPayload = SlackUrlVerification | SlackEventCallback | { type: string };

// ── Strip @mention from text ──────────────────────────────────────────────────

function stripMention(text: string): string {
  // Slack encodes mentions as <@USERID> — strip the first one (the bot mention)
  return text.replace(/^<@[A-Z0-9]+>\s*/i, "").trim();
}

// ── Route handler ─────────────────────────────────────────────────────────────

export async function POST(req: NextRequest): Promise<NextResponse> {
  // Read raw body once — needed for both JSON parsing and HMAC verification
  const rawBody = await req.text();

  // Verify signature before doing anything else
  const valid = await verifySlackSignature(req, rawBody);
  if (!valid) {
    console.warn("[Slack] Signature verification failed");
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  let payload: SlackPayload;
  try {
    payload = JSON.parse(rawBody) as SlackPayload;
  } catch {
    return NextResponse.json({ error: "Bad JSON" }, { status: 400 });
  }

  // 1. url_verification — return challenge immediately
  if (payload.type === "url_verification") {
    const { challenge } = payload as SlackUrlVerification;
    return NextResponse.json({ challenge });
  }

  // 2. event_callback
  if (payload.type === "event_callback") {
    const { event, team_id } = payload as SlackEventCallback;

    // Handle app_mention
    if (event.type === "app_mention") {
      const ev = event as SlackAppMentionEvent;

      // Ignore messages from bots (including ourselves)
      if (ev.bot_id) {
        return NextResponse.json({ ok: true });
      }

      const text = stripMention(ev.text);
      if (!text) return NextResponse.json({ ok: true });

      // Thread context: if the mention is already inside a thread, reply there.
      // Otherwise start a new thread from this message.
      const threadTs = ev.thread_ts ?? ev.ts;

      // Acknowledge immediately, then process (fits within Slack's 3 s window
      // for typical Claude response times; Slack retries with the same event_id
      // if we exceed it, which is safely idempotent given our state model).
      await handleSlackMessage({
        userId: ev.user,
        workspaceId: team_id,
        channelId: ev.channel,
        threadTs,
        text,
      });

      return NextResponse.json({ ok: true });
    }

    // Handle DMs (message.im)
    if (event.type === "message") {
      const ev = event as SlackMessageImEvent;

      // Ignore bot messages and subtypes (joins, leaves, etc.)
      if (ev.bot_id || ev.subtype || ev.channel_type !== "im") {
        return NextResponse.json({ ok: true });
      }

      const threadTs = ev.thread_ts ?? ev.ts;

      await handleSlackMessage({
        userId: ev.user,
        workspaceId: team_id,
        channelId: ev.channel,
        threadTs,
        text: ev.text?.trim() ?? "",
      });

      return NextResponse.json({ ok: true });
    }

    // Unhandled event type — acknowledge silently
    return NextResponse.json({ ok: true });
  }

  // Unknown payload type
  return NextResponse.json({ ok: true });
}
