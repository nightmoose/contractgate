"""
answer.py — Anthropic API worker.

Reads 'pending' rows from the SQLite queue, calls claude-sonnet-4-6 to
generate a reply, scores confidence, and updates the row status.
"""

import asyncio
import logging
import os
import sqlite3
from pathlib import Path

import anthropic

logger = logging.getLogger(__name__)

SYSTEM_PROMPT_PATH = Path(__file__).parent / "prompts" / "system.txt"
DISCLOSURE = (
    "^(I'm ContractGate's AI technical advisor — a bot, not a human. "
    "Happy to help with any data contract questions.)\n\n"
)

AUTO_POST_THRESHOLD = float(os.environ.get("AUTO_POST_THRESHOLD", "0.85"))
REVIEW_THRESHOLD = float(os.environ.get("REVIEW_THRESHOLD", "0.60"))

_system_prompt: str | None = None


def _load_system_prompt() -> str:
    global _system_prompt
    if _system_prompt is None:
        _system_prompt = SYSTEM_PROMPT_PATH.read_text(encoding="utf-8")
    return _system_prompt


def _build_user_message(source_text: str, permalink: str) -> str:
    return (
        f"Reddit permalink: {permalink}\n\n"
        f"Comment to respond to:\n\"\"\"\n{source_text}\n\"\"\"\n\n"
        "Instructions:\n"
        "1. Decide if this comment is relevant enough to reply to (data contracts, "
        "schema validation, data quality, ingestion, ContractGate).\n"
        "2. If relevant, write a helpful, technically accurate reply. "
        "Do NOT include the disclosure line — it will be prepended automatically.\n"
        "3. Return your answer in this exact format (no extra keys):\n"
        "CONFIDENCE: <float 0.0-1.0>\n"
        "REPLY:\n<reply body, or empty if discarding>"
    )


def _parse_response(raw: str) -> tuple[float, str]:
    """Extract confidence score and reply body from model output."""
    confidence = 0.0
    reply = ""

    lines = raw.strip().splitlines()
    reply_lines: list[str] = []
    in_reply = False

    for line in lines:
        if line.startswith("CONFIDENCE:") and not in_reply:
            try:
                confidence = float(line.split(":", 1)[1].strip())
                confidence = max(0.0, min(1.0, confidence))
            except ValueError:
                pass
        elif line.startswith("REPLY:"):
            in_reply = True
        elif in_reply:
            reply_lines.append(line)

    reply = "\n".join(reply_lines).strip()
    return confidence, reply


async def _generate_reply(client: anthropic.AsyncAnthropic, source_text: str, permalink: str) -> tuple[float, str]:
    system = _load_system_prompt()
    user_msg = _build_user_message(source_text, permalink)

    message = await client.messages.create(
        model="claude-sonnet-4-6",
        max_tokens=1024,
        system=system,
        messages=[{"role": "user", "content": user_msg}],
    )

    raw = message.content[0].text if message.content else ""
    confidence, reply_body = _parse_response(raw)

    if reply_body:
        reply_body = DISCLOSURE + reply_body

    return confidence, reply_body


def _determine_outcome(confidence: float, reply: str) -> str:
    if not reply or confidence < REVIEW_THRESHOLD:
        return "discarded"
    if confidence >= AUTO_POST_THRESHOLD:
        return "auto_posted"
    return "sent_to_slack"


async def run_answer(db: sqlite3.Connection, stop_event: asyncio.Event) -> None:
    """Poll for pending queue rows and generate replies."""
    client = anthropic.AsyncAnthropic(api_key=os.environ["ANTHROPIC_API_KEY"])
    logger.info("Answer worker started.")

    while not stop_event.is_set():
        rows = db.execute(
            "SELECT id, comment_id, permalink, source_text FROM queue WHERE outcome = 'pending' LIMIT 5"
        ).fetchall()

        if not rows:
            await asyncio.sleep(5)
            continue

        for row_id, comment_id, permalink, source_text in rows:
            if stop_event.is_set():
                break

            logger.info("Generating reply for %s", comment_id)
            try:
                confidence, reply = await _generate_reply(client, source_text, permalink)
            except Exception:
                logger.exception("Anthropic API error for %s", comment_id)
                await asyncio.sleep(10)
                continue

            outcome = _determine_outcome(confidence, reply)
            db.execute(
                """
                UPDATE queue
                SET generated_reply = ?,
                    confidence      = ?,
                    outcome         = ?,
                    actioned_at     = datetime('now')
                WHERE id = ?
                """,
                (reply if reply else None, confidence, outcome, row_id),
            )
            db.commit()
            logger.info(
                "comment=%s confidence=%.2f outcome=%s", comment_id, confidence, outcome
            )

        await asyncio.sleep(2)

    logger.info("Answer worker stopped.")
