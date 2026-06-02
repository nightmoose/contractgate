# ContractGate Reddit Bot

A Claude-powered Reddit bot (`u/ContractGateBot`) that monitors data engineering subreddits for discussions about data contracts, schema enforcement, and ingestion validation, then replies with technically accurate, **fully disclosed** AI responses.

Every reply opens with:
> *I'm ContractGate's AI technical advisor — a bot, not a human. Happy to help with any data contract questions.*

---

## Architecture

```
monitor.py  ──► SQLite queue ──► answer.py ──► poster.py ──► Reddit
                                                     └──► Slack (review queue)
```

| Component | Role |
|-----------|------|
| `monitor.py` | PRAW streaming — keyword match, deduplication, enqueue |
| `answer.py` | Anthropic API (`claude-sonnet-4-6`), confidence scoring |
| `poster.py` | 2-minute delay auto-post or Slack routing |
| `main.py` | Async orchestrator (TaskGroup) |
| `prompts/system.txt` | Full ContractGate technical system prompt |
| `db/schema.sql` | SQLite schema |

### Confidence routing

| Score | Action |
|-------|--------|
| ≥ 0.85 | Auto-post to Reddit after 2-minute delay |
| 0.60 – 0.84 | Send to Slack for human review |
| < 0.60 | Discard |

---

## Quick start

### Local

```bash
cd reddit-bot
python -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt
cp .env.example .env   # fill in credentials
python main.py
```

### Docker

```bash
docker build -t contractgate-bot .
docker run --env-file .env contractgate-bot
```

---

## Configuration

All configuration is via environment variables. See `.env.example` for the full list.

| Variable | Required | Description |
|----------|----------|-------------|
| `REDDIT_CLIENT_ID` | ✓ | Reddit OAuth app client ID |
| `REDDIT_CLIENT_SECRET` | ✓ | Reddit OAuth app client secret |
| `REDDIT_USERNAME` | ✓ | Bot account username |
| `REDDIT_PASSWORD` | ✓ | Bot account password |
| `ANTHROPIC_API_KEY` | ✓ | Anthropic API key |
| `SLACK_WEBHOOK_URL` | | Slack incoming webhook (needed for medium-confidence review) |
| `AUTO_POST_THRESHOLD` | | Float ≥ 0–1 (default `0.85`) |
| `REVIEW_THRESHOLD` | | Float ≥ 0–1 (default `0.60`) |
| `POST_DELAY_SECONDS` | | Seconds before auto-posting (default `120`) |

### Reddit app setup

1. Log into Reddit as `u/ContractGateBot`
2. Go to https://www.reddit.com/prefs/apps → **create another app**
3. Type: **script**
4. Redirect URI: `http://localhost:8080`
5. Copy `client_id` (under the app name) and `client_secret`

---

## Monitored keywords

**Exact:** `ContractGate`, `datacontractgate`  
**Topical:** `data contract`, `ingestion validation`, `schema enforcement`, `bad data at ingest`, `contract violation`, `quarantine queue`  
**Competitive:** `Great Expectations vs`, `dbt tests not enough`, `post-hoc validation`

**Subreddits:** r/dataengineering · r/datascience · r/apachekafka · r/learnpython

---

## Moderation policy

- The bot **only replies** — it never votes, awards, or takes any other Reddit action.
- Every interaction is logged to `db/bot.db` with source text, generated reply, confidence score, and outcome.
- Medium-confidence replies go to Slack for human review before posting.
- Deduplication: SQLite `comment_id` unique constraint prevents double-replies.

---

## Database

SQLite at `db/bot.db`. See `db/schema.sql` for the full schema.

```sql
SELECT comment_id, subreddit, confidence, outcome, created_at
FROM queue
ORDER BY created_at DESC
LIMIT 20;
```

---

## Related

- RFC-027: `docs/rfcs/027-reddit-bot.md`
