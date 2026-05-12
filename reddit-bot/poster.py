"""
poster.py — Reddit posting and Slack routing worker.

Reads rows with outcome='auto_posted' or 'sent_to_slack' and either:
  - Posts the reply to Reddit (after a 2-minute delay), or
  - Sends to the Slack webhook for human review.

Auto-posted rows have their reddit_reply_id updated on success.
"""

import asyncio
import json
import logging
import os
import sqlite3
import time
from datetime import datetime, timezone

import praw
import requests

logger = logging.getLogger(__name__)

POST_DELAY_SECONDS = int(os.environ.get("POST_DELAY_SECONDS", "120"))  # 2 min default
SLACK_WEBHOOK_URL: str | None = os.environ.get("SLACK_WEBHOOK_URL")


def _make_reddit() -> praw.Reddit:
    return praw.Reddit(
        client_id=os.environ["REDDIT_CLIENT_ID"],
        client_secret=os.environ["REDDIT_CLIENT_SECRET"],
        username=os.environ["REDDIT_USERNAME"],
        password=os.environ["REDDIT_PASSWORD"],
        user_agent="ContractGateBot/1.0 (by u/ContractGateBot)",
    )


def _send_to_slack(row_id: int, comment_id: str, permalink: str, reply: str, confidence: float) -> bool:
    """POST to Slack webhook for human review. Returns True on success."""
    if not SLACK_WEBHOOK_URL:
        logger.warning("SLACK_WEBHOOK_URL not set — cannot send row %d to Slack", row_id)
        return False

    payload = {
        "text": (
            f":speech_balloon: *ContractGate Bot — Human Review Needed*\n"
            f"*Comment:* <{permalink}|Reddit link>\n"
            f"*Confidence:* {confidence:.2f}\n"
            f"*Proposed reply:*\n```{reply[:2000]}```"
        )
    }
    try:
        resp = requests.post(SLACK_WEBHOOK_URL, json=payload, timeout=10)
        resp.raise_for_status()
        return True
    except Exception:
        logger.exception("Slack webhook failed for row %d", row_id)
        return False


async def run_poster(db: sqlite3.Connection, stop_event: asyncio.Event) -> None:
    """Poll queue and act on auto_posted / sent_to_slack rows."""
    loop = asyncio.get_running_loop()
    reddit = _make_reddit()
    logger.info("Poster worker started.")

    # Track when each row was first seen so we can enforce the 2-min delay
    first_seen: dict[int, float] = {}

    while not stop_event.is_set():
        rows = db.execute(
            """
            SELECT id, comment_id, subreddit, permalink, generated_reply, confidence, outcome
            FROM queue
            WHERE outcome IN ('auto_posted', 'sent_to_slack')
              AND reddit_reply_id IS NULL
              AND actioned_at IS NOT NULL
            LIMIT 20
            """
        ).fetchall()

        now = time.monotonic()
        for row in rows:
            row_id, comment_id, subreddit, permalink, reply, confidence, outcome = row

            if row_id not in first_seen:
                first_seen[row_id] = now

            if outcome == "auto_posted":
                elapsed = now - first_seen[row_id]
                if elapsed < POST_DELAY_SECONDS:
                    continue  # Not ready yet

                # Post to Reddit
                logger.info("Posting reply to %s", comment_id)
                try:
                    # comment_id is stored as t1_xxxxx fullname
                    bare_id = comment_id.replace("t1_", "")
                    comment = await loop.run_in_executor(
                        None, lambda: reddit.comment(bare_id)
                    )
                    new_reply = await loop.run_in_executor(
                        None, lambda: comment.reply(reply)
                    )
                    reply_fullname = f"t1_{new_reply.id}"
                    db.execute(
                        """
                        UPDATE queue
                        SET reddit_reply_id = ?,
                            actioned_at     = datetime('now')
                        WHERE id = ?
                        """,
                        (reply_fullname, row_id),
                    )
                    db.commit()
                    first_seen.pop(row_id, None)
                    logger.info("Posted reply %s to %s", reply_fullname, comment_id)
                except Exception:
                    logger.exception("Failed to post reply for row %d (%s)", row_id, comment_id)

            elif outcome == "sent_to_slack":
                success = _send_to_slack(row_id, comment_id, permalink, reply, confidence)
                if success:
                    # Mark as handled so we don't resend
                    db.execute(
                        "UPDATE queue SET reddit_reply_id = 'slack_sent' WHERE id = ?",
                        (row_id,),
                    )
                    db.commit()
                    first_seen.pop(row_id, None)

        await asyncio.sleep(10)

    logger.info("Poster worker stopped.")
