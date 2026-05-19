# RFC-027: Disclosed Reddit Bot for Technical Community Engagement

**Status:** Accepted  
**Date:** 2026-05-11  
**Author:** ContractGate Engineering

---

## Summary

Build a Claude-powered Reddit bot (`u/ContractGateBot`) that monitors data engineering subreddits for relevant discussions and replies with technically accurate, fully-disclosed AI responses. The bot never impersonates a human and every reply opens with a clear bot disclosure.

---

## Motivation

ContractGate is technical software used by data engineers. The communities on Reddit (r/dataengineering, r/datascience, r/apachekafka, r/learnpython) regularly surface questions about data contracts, schema enforcement, ingestion validation, and quarantine patterns — exactly the problems ContractGate solves. A disclosed, technically accurate bot presence increases awareness, provides genuine value, and creates a feedback loop for product positioning.

---

## Design

### Components

| File | Role |
|------|------|
| `main.py` | Async orchestrator — starts monitor, answer, and poster loops |
| `monitor.py` | PRAW streaming; keyword match; writes to SQLite queue |
| `answer.py` | Anthropic API call, confidence scoring, routes High/Medium/Low |
| `poster.py` | Reads queue; auto-posts after 2-min delay or routes to Slack |
| `prompts/system.txt` | Full ContractGate technical system prompt |
| `db/schema.sql` | SQLite schema |
| `.env.example` | Required environment variables |
| `Dockerfile` | Single-process container (1 vCPU / 512 MB) |

### Keyword Strategy

**Exact:** `ContractGate`, `datacontractgate`  
**Topical:** `data contract`, `ingestion validation`, `schema enforcement`, `bad data at ingest`, `contract violation`, `quarantine queue`  
**Competitive:** `Great Expectations vs`, `dbt tests not enough`, `post-hoc validation`

**Target subreddits:** r/dataengineering, r/datascience, r/apachekafka, r/learnpython

### Confidence Routing

| Score | Action |
|-------|--------|
| ≥ 0.85 | Auto-post after 2-minute delay |
| 0.60 – 0.84 | Human review via Slack webhook |
| < 0.60 | Discard silently |

### Disclosure

Every reply opens with:

```
^(I'm ContractGate's AI technical advisor — a bot, not a human. Happy to help with any data contract questions.)
```

### Constraints

- Bot may only **reply to comments/posts**. No votes, awards, or other actions.
- Deduplication: SQLite `comment_id` unique constraint prevents double-replies.
- All interactions logged with source, response, confidence score, and outcome.

---

## Security / Compliance

- Credentials stored in `.env`, never committed.
- Bot account password scoped to Reddit OAuth only.
- Anthropic API key rotated quarterly.
- Slack webhook delivers only the generated response text and Reddit permalink.

---

## Alternatives Considered

- **Human moderator posting manually:** Too slow, doesn't scale.
- **Undisclosed bot:** Rejected — violates Reddit policy and ContractGate values.
- **Twitter/X engagement:** Out of scope for this RFC.

---

## Implementation

See `reddit-bot/` directory. Deploy via `docker compose` or Fly.io (`fly deploy`).
