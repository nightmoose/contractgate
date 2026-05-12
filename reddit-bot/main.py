"""
main.py — ContractGate Reddit Bot orchestrator.

Runs three concurrent async workers:
  1. monitor  — PRAW streaming, keyword match, enqueue
  2. answer   — Anthropic API, confidence scoring, update outcome
  3. poster   — auto-post to Reddit or route to Slack

Usage:
    python main.py

Environment variables: see .env.example
"""

import asyncio
import logging
import os
import sqlite3
from pathlib import Path

from dotenv import load_dotenv

load_dotenv()

from answer import run_answer
from monitor import run_monitor
from poster import run_poster

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
)
logger = logging.getLogger("main")

DB_PATH = Path(__file__).parent / "db" / "bot.db"
SCHEMA_PATH = Path(__file__).parent / "db" / "schema.sql"


def _init_db() -> sqlite3.Connection:
    DB_PATH.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(DB_PATH), check_same_thread=False)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.executescript(SCHEMA_PATH.read_text())
    conn.commit()
    logger.info("Database ready at %s", DB_PATH)
    return conn


async def main() -> None:
    _validate_env()
    db = _init_db()
    stop = asyncio.Event()

    try:
        async with asyncio.TaskGroup() as tg:
            tg.create_task(run_monitor(db, stop), name="monitor")
            tg.create_task(run_answer(db, stop), name="answer")
            tg.create_task(run_poster(db, stop), name="poster")
    except* KeyboardInterrupt:
        logger.info("Shutdown requested — stopping workers.")
        stop.set()
    except* Exception as eg:
        for exc in eg.exceptions:
            logger.exception("Worker crashed: %s", exc)
        stop.set()
    finally:
        db.close()
        logger.info("ContractGateBot exited.")


def _validate_env() -> None:
    required = [
        "REDDIT_CLIENT_ID",
        "REDDIT_CLIENT_SECRET",
        "REDDIT_USERNAME",
        "REDDIT_PASSWORD",
        "ANTHROPIC_API_KEY",
    ]
    missing = [k for k in required if not os.environ.get(k)]
    if missing:
        raise RuntimeError(f"Missing required environment variables: {', '.join(missing)}")


if __name__ == "__main__":
    asyncio.run(main())
