-- RFC-084: Slack bot — lead intake and conversation state.
--
-- slack_leads: one row per completed intake form submission.
-- slack_conversations: ephemeral per-thread state (messages + intake progress).
--   Used by the bot to maintain conversational context across Slack messages
--   within a thread (or DM). Serverless functions are stateless; this table
--   is the only durable store available.

-- ── slack_leads ───────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS public.slack_leads (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slack_user_id   TEXT NOT NULL,
    slack_workspace TEXT NOT NULL,
    -- Intake answers (populated one at a time during the conversational flow)
    name            TEXT,
    email           TEXT,
    company         TEXT,
    stack           TEXT,  -- current data stack description
    use_case        TEXT,  -- main pain point / goal
    onboarding_pref TEXT,  -- 'self-serve' | 'guided' | free-text
    -- Lifecycle
    status          TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new', 'contacted', 'qualified', 'closed')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS slack_leads_workspace_user_idx
    ON public.slack_leads (slack_workspace, slack_user_id);

CREATE INDEX IF NOT EXISTS slack_leads_status_idx
    ON public.slack_leads (status);

COMMENT ON TABLE public.slack_leads IS
    'RFC-084: Leads captured via the ContractGate Slack bot intake flow.';

-- ── slack_conversations ───────────────────────────────────────────────────────
-- Stores short conversation history + intake state per Slack thread.
-- Primary key: (workspace, thread_ts) — thread_ts is the timestamp of the
-- first message in the thread (Slack's canonical thread identifier). For DMs
-- with no explicit thread, the event ts is used as thread_ts.

CREATE TABLE IF NOT EXISTS public.slack_conversations (
    workspace   TEXT NOT NULL,
    thread_ts   TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    channel_id  TEXT NOT NULL,
    -- Last N messages as [{role, content}] JSON — kept small, trimmed on write.
    messages    JSONB NOT NULL DEFAULT '[]',
    -- Null when not in intake mode; populated when intake flow is active.
    -- Shape: { step: 0-4, answers: { name_company?, email?, stack?, use_case?, onboarding_pref? } }
    intake_state JSONB,
    -- lead_id is set once all 5 answers are collected and the row is inserted.
    lead_id     UUID REFERENCES public.slack_leads(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace, thread_ts)
);

COMMENT ON TABLE public.slack_conversations IS
    'RFC-084: Ephemeral per-thread Slack bot state (history + intake progress). Service-role only.';

-- No RLS policies: the Slack bot uses the service-role key (server-only).
-- Never expose these tables via the anon/publishable key.
--
-- Enable RLS with NO policy → deny-by-default for anon/authenticated; the bot's
-- service-role key bypasses RLS, so the bot still works. This matches the
-- idempotency_keys / stripe_* service-role-only pattern. Without it these tables
-- (which hold lead PII: name/email/company) are readable via PostgREST by anon
-- (advisor ERROR: rls_disabled_in_public).
ALTER TABLE public.slack_leads ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.slack_conversations ENABLE ROW LEVEL SECURITY;
