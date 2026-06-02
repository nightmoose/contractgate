"""
monitor.py — PRAW streaming loop.

Watches target subreddits for comments/posts matching ContractGate keywords,
deduplicates via SQLite, and enqueues candidates for the answer worker.
"""

import asyncio
import logging
import os
import re
import sqlite3
import time
from typing import Optional

import praw

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

SUBREDDITS = [
    "dataengineering",
    "datascience",
    "apachekafka",
    "learnpython",
]

# Each tuple: (pattern, is_regex)
KEYWORD_PATTERNS: list[tuple[str, bool]] = [
    # Exact brand mentions
    (r"\bContractGate\b", True),
    (r"\bdatacontractgate\b", True),
    # Topical
    (r"\bdata\s+contract\b", True),
    (r"\bingestion\s+validation\b", True),
    (r"\bschema\s+enforcement\b", True),
    (r"bad data at ingest", False),
    (r"\bcontract\s+violation\b", True),
    (r"\bquarantine\s+queue\b", True),
    # Competitive
    (r"Great\s+Expectations\s+vs", True),
    (r"dbt tests not enough", False),
    (r"post.?hoc\s+validation", True),
]

_COMPILED = [
    (re.compile(p, re.IGNORECASE) if is_re else None, p, is_re)
    for p, is_re in KEYWORD_PATTERNS
]


def _matches(text: str) -> bool:
    """Return True if text contains any monitored keyword."""
    for compiled, raw, is_re in _COMPILED:
        if is_re:
            if compiled.search(text):
                return True
        else:
            if raw.lower() in text.lower():
                return True
    return False


def _already_seen(db: sqlite3.Connection, comment_id: str) -> bool:
    row = db.execute(
        "SELECT 1 FROM queue WHERE comment_id = ?", (comment_id,)
    ).fetchone()
    return row is not None


def _enqueue(db: sqlite3.Connection, comment_id: str, subreddit: str, permalink: str, text: str) -> None:
    db.execute(
        """
        INSERT OR IGNORE INTO queue
            (comment_id, subreddit, permalink, source_text, outcome)
        VALUES (?, ?, ?, ?, 'pending')
        """,
        (comment_id, subreddit, permalink, text),
    )
    db.commit()
    logger.info("Enqueued %s from r/%s", comment_id, subreddit)


def _make_reddit() -> praw.Reddit:
    return praw.Reddit(
        client_id=os.environ["REDDIT_CLIENT_ID"],
        client_secret=os.environ["REDDIT_CLIENT_SECRET"],
        username=os.environ["REDDIT_USERNAME"],
        password=os.environ["REDDIT_PASSWORD"],
        user_agent="ContractGateBot/1.0 (by u/ContractGateBot)",
    )


async def run_monitor(db: sqlite3.Connection, stop_event: asyncio.Event) -> None:
    """Stream comments from target subreddits and enqueue matches."""
    loop = asyncio.get_running_loop()

    def _blocking_stream() -> None:
        reddit = _make_reddit()
        subreddit = reddit.subreddit("+".join(SUBREDDITS))

        logger.info("Monitor started — watching r/%s", "+".join(SUBREDDITS))

        for comment in subreddit.stream.comments(skip_existing=True, pause_after=5):
            if stop_event.is_set():
                break

            if comment is None:
                # pause_after=5 yields None when no new items; check stop flag
                time.sleep(1)
                continue

            text = (comment.body or "").strip()
            if not text or text == "[deleted]" or text == "[removed]":
                continue

            if not _matches(text):
                continue

            comment_id = f"t1_{comment.id}"
            if _already_seen(db, comment_id):
                continue

            permalink = f"https://reddit.com{comment.permalink}"
            _enqueue(db, comment_id, str(comment.subreddit), permalink, text)

        logger.info("Monitor stream ended.")

    await loop.run_in_executor(None, _blocking_stream)
