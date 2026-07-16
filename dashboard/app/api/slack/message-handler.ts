/**
 * ContractGate Slack Bot — message handler
 *
 * Handles incoming Slack messages (app_mentions and DMs) by:
 *   1. Loading/creating per-thread conversation state from Supabase
 *   2. Detecting intake intent and running the 5-question intake flow
 *   3. Calling the configured LLM for general Q&A
 *      (`LLM_PROVIDER=xai|anthropic`, default anthropic)
 *   4. Posting the reply back to Slack and persisting the updated state
 */

import Anthropic from "@anthropic-ai/sdk";
import { createClient as createServiceClient } from "@supabase/supabase-js";

// ── Lazy singletons (build-time safe) ─────────────────────────────────────────

let _anthropic: Anthropic | null = null;
function getAnthropic(): Anthropic {
  if (!_anthropic) {
    const key = process.env.ANTHROPIC_API_KEY;
    if (!key) {
      throw new Error("ANTHROPIC_API_KEY is not set (required when LLM_PROVIDER=anthropic)");
    }
    _anthropic = new Anthropic({ apiKey: key });
  }
  return _anthropic;
}

type LlmProvider = "xai" | "anthropic";

/** `LLM_PROVIDER` env: `xai` | `anthropic` (default). Case-insensitive. */
function getLlmProvider(): LlmProvider {
  const raw = (process.env.LLM_PROVIDER ?? "anthropic").trim().toLowerCase();
  if (raw === "xai") return "xai";
  return "anthropic";
}

function getSupabaseAdmin() {
  return createServiceClient(
    process.env.NEXT_PUBLIC_SUPABASE_URL!,
    process.env.SUPABASE_SERVICE_ROLE_KEY!,
    { auth: { persistSession: false } }
  );
}

// ── Types ─────────────────────────────────────────────────────────────────────

interface SlackMessage {
  role: "user" | "assistant";
  content: string;
  /** How the assistant message was produced — used for rate limits (LLM only). */
  via?: "llm" | "intake" | "limit" | "block";
}

interface IntakeAnswers {
  name_company?: string;
  email?: string;
  stack?: string;
  use_case?: string;
  onboarding_pref?: string;
}

interface IntakeState {
  step: number; // 0-4 (which question we're waiting to receive an answer for)
  answers: IntakeAnswers;
}

interface ConversationRow {
  workspace: string;
  thread_ts: string;
  user_id: string;
  channel_id: string;
  messages: SlackMessage[];
  intake_state: IntakeState | null;
  lead_id: string | null;
}

export interface HandleMessageArgs {
  userId: string;
  workspaceId: string;
  channelId: string;
  threadTs: string; // Slack thread identifier (ts of the first message)
  text: string;     // user message text (mentions stripped by caller)
}

// ── Intake configuration ──────────────────────────────────────────────────────

const INTAKE_QUESTIONS = [
  "What's your name and what company are you with?",
  "What's your email so we can follow up with you?",
  "What does your current data stack look like? Rough is totally fine — Kafka, Kinesis, flat files, etc.",
  "What's the main pain point you're hoping ContractGate helps you solve?",
  "Would you prefer a self-serve pilot or a guided walkthrough with our team?",
] as const;

const INTAKE_ANSWER_KEYS: (keyof IntakeAnswers)[] = [
  "name_company",
  "email",
  "stack",
  "use_case",
  "onboarding_pref",
];

/** Patterns that trigger the intake flow */
const INTAKE_TRIGGERS = [
  /i'?m interested/i,
  /want to try/i,
  /sign me up/i,
  /tell me more about pricing/i,
  /tell me more about.*pilot/i,
  /interested in.*pilot/i,
  /how do i get (started|access|a demo)/i,
  /can i get (access|a demo|a trial)/i,
  /request (access|a demo|a pilot)/i,
];

function detectsIntakeIntent(text: string): boolean {
  return INTAKE_TRIGGERS.some((re) => re.test(text));
}

// ── System prompt ─────────────────────────────────────────────────────────────

const SYSTEM_PROMPT = `You are the ContractGate product assistant — not a general-purpose chatbot.

Your ONLY job is to help people understand ContractGate (data contracts at ingest,
quarantine, replay, pilot reports, plans, integrations). You are NOT Grok, ChatGPT,
or a free coding/writing tutor.

About ContractGate:
ContractGate is a data contract enforcement platform. It sits between your producers and consumers, validates every event against a schema contract, and routes violations to a quarantine queue for inspection and replay. Key features include:
- **Schema ingestion**: import contracts from YAML/JSON (ODCS format), GitHub, or the API
- **Multi-source ingestion**: Kafka, Kinesis, HTTP webhooks, flat file upload
- **Real-time quarantine**: invalid events are held, not dropped — inspect and replay once fixed
- **PII masking**: field-level transforms before events reach consumers
- **Egress validation**: validate outbound data too
- **Scoring & reporting**: per-provider data quality scorecards
- **Team collaboration**: org-scoped access, shared contract libraries, audit logs
- **Plan tiers**: free, growth, enterprise — with usage-based metering

Hard rules:
- If the user asks for something unrelated to ContractGate / data contracts / data quality
  pipelines (e.g. general coding, homework, recipes, other products, roleplay), refuse in
  1–2 short sentences and invite them to ask about ContractGate or say "I'm interested"
  for a pilot/advisory intake. Do NOT answer the off-topic request.
- Do not write long code, essays, or multi-step tutorials for unrelated tools.
- Never claim to be a general AI assistant or that you can help with anything.

Default answer style (important):
- Be concise by default: 2–5 short sentences OR a handful of tight bullets. Lead with the
  answer, not a preamble.
- End most answers with a brief offer to go deeper, e.g. "Want more detail on X, or how
  that fits your stack?" / "Happy to elaborate on any of those."
- Only give a longer, in-depth answer if the user explicitly asks to elaborate, expand,
  go deeper, or asks a follow-up that clearly needs more depth.
- Technical depth is fine for data engineers — still keep the first reply short.

If you don't know a pricing/SLA detail, say so and suggest the website or intake.`;

// ── Abuse / cost controls (env-overridable) ───────────────────────────────────

function envInt(name: string, fallback: number): number {
  const raw = process.env[name]?.trim();
  if (!raw) return fallback;
  const n = parseInt(raw, 10);
  return Number.isFinite(n) && n >= 0 ? n : fallback;
}

/** Max LLM replies per Slack user per rolling hour (default 10). */
function llmPerUserHour(): number {
  return envInt("SLACK_LLM_PER_USER_HOUR", 10);
}

/** Max LLM replies per Slack user per UTC day (default 30). */
function llmPerUserDay(): number {
  return envInt("SLACK_LLM_PER_USER_DAY", 30);
}

/** Max LLM replies in a single thread/DM (default 12). */
function llmPerThread(): number {
  return envInt("SLACK_LLM_PER_THREAD", 12);
}

/** Max completion tokens per LLM call (default 512). */
function llmMaxTokens(): number {
  return envInt("SLACK_LLM_MAX_TOKENS", 512);
}

/** How many prior messages to send the model (default 8 = ~4 turns). */
function llmHistoryLimit(): number {
  return envInt("SLACK_LLM_HISTORY_LIMIT", 8);
}

/**
 * Optional comma-separated Slack team IDs allowed to use the bot.
 * Empty = all workspaces where the app is installed.
 */
function allowedTeamIds(): Set<string> | null {
  const raw = process.env.SLACK_ALLOWED_TEAM_IDS?.trim();
  if (!raw) return null;
  return new Set(
    raw
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean)
  );
}

const RATE_LIMIT_REPLY =
  "You've hit the free Q&A limit for now (we cap this so the bot stays useful for " +
  "people evaluating ContractGate, not as a general chatbot).\n\n" +
  "• Say *I'm interested* to leave your details for a pilot/advisory seat\n" +
  "• Or try again later with a ContractGate-specific question";

const THREAD_LIMIT_REPLY =
  "This thread has reached the Q&A limit. Start a fresh DM or say *I'm interested* " +
  "if you'd like a pilot/advisory follow-up.";

const WORKSPACE_BLOCKED_REPLY =
  "This Slack workspace isn't enabled for the ContractGate bot. " +
  "DM us via the workspace where you installed the app, or visit the website.";

/** Count LLM assistant turns only (excludes intake / rate-limit notices). */
function countLlmAssistantTurns(messages: SlackMessage[]): number {
  return messages.filter(
    (m) =>
      m.role === "assistant" &&
      m.via !== "intake" &&
      m.via !== "limit" &&
      m.via !== "block"
  ).length;
}

/**
 * Count this user's assistant turns across recent conversations (durable,
 * works across serverless instances). Best-effort — on DB error, allow the call.
 */
async function countUserAssistantTurns(
  workspace: string,
  userId: string,
  sinceIso: string
): Promise<number> {
  const supabase = getSupabaseAdmin();
  const { data, error } = await supabase
    .from("slack_conversations")
    .select("messages")
    .eq("workspace", workspace)
    .eq("user_id", userId)
    .gte("updated_at", sinceIso);

  if (error || !data) {
    if (error) {
      console.warn("[SlackBot] rate-limit count failed:", error.message);
    }
    return 0;
  }

  let n = 0;
  for (const row of data) {
    const msgs = row.messages as SlackMessage[] | null;
    if (Array.isArray(msgs)) n += countLlmAssistantTurns(msgs);
  }
  return n;
}

function hoursAgoIso(hours: number): string {
  return new Date(Date.now() - hours * 3600 * 1000).toISOString();
}

function utcDayStartIso(): string {
  const d = new Date();
  d.setUTCHours(0, 0, 0, 0);
  return d.toISOString();
}

// ── Slack API helpers ─────────────────────────────────────────────────────────

async function postSlackMessage(
  channelId: string,
  text: string,
  threadTs?: string
): Promise<void> {
  const body: Record<string, string> = { channel: channelId, text };
  if (threadTs) body.thread_ts = threadTs;

  const res = await fetch("https://slack.com/api/chat.postMessage", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${process.env.SLACK_BOT_TOKEN}`,
    },
    body: JSON.stringify(body),
  });

  const json = (await res.json()) as { ok: boolean; error?: string };
  if (!json.ok) {
    console.error("[Slack] postMessage failed:", json.error);
  }
}

// ── Conversation persistence ───────────────────────────────────────────────────

const MAX_HISTORY_MESSAGES = 20; // keep last 10 turns (user + assistant)

async function loadConversation(
  workspace: string,
  threadTs: string,
  userId: string,
  channelId: string
): Promise<ConversationRow> {
  const supabase = getSupabaseAdmin();
  const { data, error } = await supabase
    .from("slack_conversations")
    .select("*")
    .eq("workspace", workspace)
    .eq("thread_ts", threadTs)
    .maybeSingle();

  if (error) {
    console.error("[SlackBot] loadConversation error:", error.message);
  }

  if (data) {
    return data as ConversationRow;
  }

  // New thread — insert a fresh row
  const fresh: ConversationRow = {
    workspace,
    thread_ts: threadTs,
    user_id: userId,
    channel_id: channelId,
    messages: [],
    intake_state: null,
    lead_id: null,
  };
  await supabase.from("slack_conversations").insert(fresh);
  return fresh;
}

async function saveConversation(conv: ConversationRow): Promise<void> {
  // Trim history to the most recent MAX_HISTORY_MESSAGES entries
  const trimmed = conv.messages.slice(-MAX_HISTORY_MESSAGES);
  const supabase = getSupabaseAdmin();
  const { error } = await supabase
    .from("slack_conversations")
    .update({
      messages: trimmed,
      intake_state: conv.intake_state,
      lead_id: conv.lead_id,
      updated_at: new Date().toISOString(),
    })
    .eq("workspace", conv.workspace)
    .eq("thread_ts", conv.thread_ts);

  if (error) {
    console.error("[SlackBot] saveConversation error:", error.message);
  }
}

// ── Intake flow ───────────────────────────────────────────────────────────────

async function saveLead(
  conv: ConversationRow,
  answers: IntakeAnswers
): Promise<string | null> {
  const supabase = getSupabaseAdmin();
  const { data, error } = await supabase
    .from("slack_leads")
    .insert({
      slack_user_id: conv.user_id,
      slack_workspace: conv.workspace,
      name: answers.name_company ?? null,
      email: answers.email ?? null,
      company: null, // parsed from name_company if needed downstream
      stack: answers.stack ?? null,
      use_case: answers.use_case ?? null,
      onboarding_pref: answers.onboarding_pref ?? null,
      status: "new",
    })
    .select("id")
    .single();

  if (error) {
    console.error("[SlackBot] saveLead error:", error.message);
    return null;
  }
  return (data as { id: string }).id;
}

/**
 * Handle a message while in intake mode.
 * Records the answer for the current step, advances to the next question,
 * and saves the lead when all 5 answers are collected.
 * Returns the bot's reply text.
 */
async function handleIntakeStep(
  conv: ConversationRow,
  userText: string
): Promise<string> {
  const state = conv.intake_state!;
  const key = INTAKE_ANSWER_KEYS[state.step];
  state.answers[key] = userText.trim();

  const nextStep = state.step + 1;

  if (nextStep < INTAKE_QUESTIONS.length) {
    // More questions to ask
    conv.intake_state = { step: nextStep, answers: state.answers };
    return INTAKE_QUESTIONS[nextStep];
  }

  // All answers collected — save lead and finish
  conv.intake_state = null;
  const leadId = await saveLead(conv, state.answers);
  conv.lead_id = leadId;

  return (
    "Thanks — I've got everything I need! 🎉\n\n" +
    "Someone from the ContractGate team will follow up with you shortly. " +
    "In the meantime, feel free to keep asking questions here — I'm happy to help."
  );
}

// ── LLM call (xAI or Anthropic) ───────────────────────────────────────────────

async function callAnthropic(
  history: SlackMessage[],
  userMessage: string
): Promise<string> {
  const messages: Array<{ role: "user" | "assistant"; content: string }> = [
    ...history,
    { role: "user", content: userMessage },
  ];

  const model = process.env.ANTHROPIC_MODEL?.trim() || "claude-sonnet-4-6";
  const response = await getAnthropic().messages.create({
    model,
    max_tokens: llmMaxTokens(),
    system: SYSTEM_PROMPT,
    messages,
  });

  const block = response.content[0];
  if (block.type === "text" && "text" in block && typeof block.text === "string") {
    return block.text;
  }
  return "Sorry, I couldn't generate a response. Please try again.";
}

/** xAI chat completions (OpenAI-compatible). Docs: https://docs.x.ai/docs */
async function callXai(
  history: SlackMessage[],
  userMessage: string
): Promise<string> {
  const apiKey = process.env.XAI_API_KEY;
  if (!apiKey) {
    throw new Error("XAI_API_KEY is not set (required when LLM_PROVIDER=xai)");
  }

  const model = process.env.XAI_MODEL?.trim() || "grok-4";
  const messages: Array<{ role: "system" | "user" | "assistant"; content: string }> = [
    { role: "system", content: SYSTEM_PROMPT },
    ...history.map((m) => ({ role: m.role, content: m.content })),
    { role: "user", content: userMessage },
  ];

  const res = await fetch("https://api.x.ai/v1/chat/completions", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${apiKey}`,
    },
    body: JSON.stringify({
      model,
      messages,
      max_tokens: llmMaxTokens(),
      temperature: 0.4,
    }),
  });

  if (!res.ok) {
    const errText = await res.text().catch(() => "");
    console.error("[SlackBot] xAI API error:", res.status, errText.slice(0, 500));
    throw new Error(`xAI API request failed (${res.status})`);
  }

  const json = (await res.json()) as {
    choices?: Array<{ message?: { content?: string | null } }>;
  };
  const text = json.choices?.[0]?.message?.content?.trim();
  if (text) return text;
  return "Sorry, I couldn't generate a response. Please try again.";
}

async function callLlm(
  history: SlackMessage[],
  userMessage: string
): Promise<string> {
  const provider = getLlmProvider();
  try {
    if (provider === "xai") {
      return await callXai(history, userMessage);
    }
    return await callAnthropic(history, userMessage);
  } catch (e) {
    console.error(`[SlackBot] LLM call failed (provider=${provider}):`, e);
    return (
      "Sorry — I'm having trouble reaching the language model right now. " +
      "Please try again in a moment, or start with “I'm interested” to leave your details."
    );
  }
}

// ── Main entry point ──────────────────────────────────────────────────────────

export async function handleSlackMessage(args: HandleMessageArgs): Promise<void> {
  const { userId, workspaceId, channelId, threadTs, text } = args;

  // Optional workspace allowlist (blocks freeloaders on random installs)
  const teams = allowedTeamIds();
  if (teams && !teams.has(workspaceId)) {
    await postSlackMessage(channelId, WORKSPACE_BLOCKED_REPLY, threadTs);
    return;
  }

  // Load (or create) conversation state
  const conv = await loadConversation(workspaceId, threadTs, userId, channelId);

  let replyText: string;
  let via: SlackMessage["via"] = "intake";

  if (conv.intake_state !== null) {
    // Intake is free (no LLM) — always allowed
    replyText = await handleIntakeStep(conv, text);
    via = "intake";
  } else if (detectsIntakeIntent(text)) {
    conv.intake_state = { step: 0, answers: {} };
    replyText =
      "Great, I'd love to learn more about your situation! Just a few quick questions.\n\n" +
      INTAKE_QUESTIONS[0];
    via = "intake";
  } else {
    // ── LLM gate: thread cap + per-user hour/day caps ───────────────────────
    const threadTurns = countLlmAssistantTurns(conv.messages);
    if (threadTurns >= llmPerThread()) {
      replyText = THREAD_LIMIT_REPLY;
      via = "limit";
    } else {
      const hourLimit = llmPerUserHour();
      const dayLimit = llmPerUserDay();
      const [hourCount, dayCount] = await Promise.all([
        countUserAssistantTurns(workspaceId, userId, hoursAgoIso(1)),
        countUserAssistantTurns(workspaceId, userId, utcDayStartIso()),
      ]);

      if (hourCount >= hourLimit || dayCount >= dayLimit) {
        console.info(
          `[SlackBot] rate limit user=${userId} hour=${hourCount}/${hourLimit} day=${dayCount}/${dayLimit}`
        );
        replyText = RATE_LIMIT_REPLY;
        via = "limit";
      } else {
        const history = conv.messages
          .filter((m) => m.via !== "limit" && m.via !== "block")
          .slice(-llmHistoryLimit());
        replyText = await callLlm(history, text);
        via = "llm";
      }
    }
  }

  conv.messages.push({ role: "user", content: text });
  conv.messages.push({ role: "assistant", content: replyText, via });

  await Promise.all([
    saveConversation(conv),
    postSlackMessage(channelId, replyText, threadTs),
  ]);

  if (via === "llm") {
    console.info(`[SlackBot] LLM reply user=${userId} workspace=${workspaceId}`);
  }
}
