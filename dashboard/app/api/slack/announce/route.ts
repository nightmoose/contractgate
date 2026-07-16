/**
 * POST /api/slack/announce
 *
 * Posts a message to a Slack channel. Intended for scheduled announcements
 * triggered by an external cron or script.
 *
 * Request body: { channel: string, message: string }
 * Authorization: Bearer <SLACK_ANNOUNCE_SECRET>  (Authorization header)
 *
 * This endpoint is NOT authenticated via Supabase session — it uses a shared
 * secret instead. Keep SLACK_ANNOUNCE_SECRET out of client-side code.
 */

import { NextRequest, NextResponse } from "next/server";
import crypto from "crypto";

export const dynamic = "force-dynamic";

interface AnnounceBody {
  channel: string;
  message: string;
}

function timingSafeEqual(a: string, b: string): boolean {
  // Pad to the same length before comparison to prevent length-timing leaks
  const aBuf = Buffer.from(a.padEnd(Math.max(a.length, b.length), "\0"), "utf8");
  const bBuf = Buffer.from(b.padEnd(Math.max(a.length, b.length), "\0"), "utf8");
  try {
    return crypto.timingSafeEqual(aBuf, bBuf);
  } catch {
    return false;
  }
}

export async function POST(req: NextRequest): Promise<NextResponse> {
  const announceSecret = process.env.SLACK_ANNOUNCE_SECRET;
  if (!announceSecret) {
    console.error("[Slack/Announce] SLACK_ANNOUNCE_SECRET not configured");
    return NextResponse.json({ error: "Service misconfigured" }, { status: 500 });
  }

  // Verify shared secret from Authorization header
  const authHeader = req.headers.get("authorization") ?? "";
  const token = authHeader.startsWith("Bearer ") ? authHeader.slice(7) : "";
  if (!token || !timingSafeEqual(token, announceSecret)) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  let body: AnnounceBody;
  try {
    body = (await req.json()) as AnnounceBody;
  } catch {
    return NextResponse.json({ error: "Invalid JSON" }, { status: 400 });
  }

  const { channel, message } = body;
  if (!channel || !message) {
    return NextResponse.json(
      { error: "Both 'channel' and 'message' are required" },
      { status: 422 }
    );
  }

  const slackToken = process.env.SLACK_BOT_TOKEN;
  if (!slackToken) {
    console.error("[Slack/Announce] SLACK_BOT_TOKEN not configured");
    return NextResponse.json({ error: "Service misconfigured" }, { status: 500 });
  }

  const res = await fetch("https://slack.com/api/chat.postMessage", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${slackToken}`,
    },
    body: JSON.stringify({ channel, text: message }),
  });

  const json = (await res.json()) as { ok: boolean; error?: string; ts?: string };

  if (!json.ok) {
    console.error("[Slack/Announce] chat.postMessage failed:", json.error);
    return NextResponse.json(
      { error: `Slack API error: ${json.error}` },
      { status: 502 }
    );
  }

  return NextResponse.json({ ok: true, ts: json.ts });
}
