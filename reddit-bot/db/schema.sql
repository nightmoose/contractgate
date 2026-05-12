-- ContractGate Reddit Bot — SQLite schema
-- Stores every candidate comment and the bot's action on it.

CREATE TABLE IF NOT EXISTS queue (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    comment_id      TEXT    NOT NULL UNIQUE,   -- Reddit fullname (t1_xxxxx)
    subreddit       TEXT    NOT NULL,
    permalink       TEXT    NOT NULL,
    source_text     TEXT    NOT NULL,          -- Original Reddit comment/post body
    generated_reply TEXT,                      -- Anthropic response (NULL if discarded)
    confidence      REAL,                      -- 0.0 – 1.0
    outcome         TEXT    CHECK(outcome IN ('auto_posted', 'sent_to_slack', 'discarded', 'pending')),
    reddit_reply_id TEXT,                      -- Fullname of the reply we posted (if any)
    created_at      TEXT    NOT NULL DEFAULT (datetime('now')),
    actioned_at     TEXT                       -- When outcome was set
);

CREATE INDEX IF NOT EXISTS idx_queue_outcome    ON queue(outcome);
CREATE INDEX IF NOT EXISTS idx_queue_created_at ON queue(created_at);
