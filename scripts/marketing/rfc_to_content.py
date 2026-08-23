#!/usr/bin/env python3
"""
rfc_to_content.py — Turn a shipped ContractGate RFC into publishable drafts.

Reads docs/rfcs/<N>-*.md, looks up its ship status in docs/STATUS.md, and asks
Claude to produce four drafts (blog post, Show HN post, X thread, Reddit post)
in ContractGate's voice per docs/marketing/marketing-plan.md §7.

Never publishes anything. Writes to docs/marketing/drafts/rfc-<N>/ for human
review before posting.

Usage:
    python scripts/marketing/rfc_to_content.py 89
    python scripts/marketing/rfc_to_content.py 89 --dry-run       # no API call
    python scripts/marketing/rfc_to_content.py 89 --out /tmp/out
    python scripts/marketing/rfc_to_content.py 89 --model claude-sonnet-4-6

Requires: ANTHROPIC_API_KEY in env; `pip install anthropic` (unless --dry-run).
"""
from __future__ import annotations

import argparse
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

REPO_ROOT = Path(__file__).resolve().parents[2]
RFCS_DIR = REPO_ROOT / "docs" / "rfcs"
STATUS_FILE = REPO_ROOT / "docs" / "STATUS.md"
DEFAULT_OUT_ROOT = REPO_ROOT / "docs" / "marketing" / "drafts"
DEFAULT_MODEL = "claude-opus-4-7"

ARTIFACTS = ("blog", "hn", "x-thread", "reddit-dataengineering")

# ── Voice rules (mirrors docs/marketing/marketing-plan.md §7) ─────────────────
# Kept inline so the script is self-contained; if you change the plan, update
# this too. The doc is the source of truth; this is the machine copy.
SYSTEM_PROMPT = """\
You are ContractGate's marketing writer. ContractGate is the runtime enforcement
gate for streaming data contracts: users write a semantic YAML contract, we
validate every event at ingress in sub-millisecond (Rust core, 86k+ events/s/core),
and route bad events to quarantine before consumers see them. Patent pending.

Positioning: "A schema is not a contract. We enforce the contract." Direct nod
to Stéphane Derosiaux's Dec 2025 thesis. Not a Schema Registry replacement; not
a shift-left change-management workflow like Gable. The runtime choke point.

Audience: platform, data, and MLOps engineers who own a Kafka / Kinesis / HTTP
ingest path and have been burned by a producer shipping a semantically broken
event that was schema-valid.

Voice + style rules (non-negotiable):
- First-person plural for blog + HN ("we hit", "we shipped"). First-person
  singular for Reddit ("I hit", "I ran into"). Never open a post with "you".
- Lead with the problem, not the product. Show a broken event, then the fix.
- Concrete beats abstract every time. Use real numbers (latency, event counts,
  throughput). If the RFC has them, cite them; if it doesn't, don't invent.
- Zero marketing adjectives. Do not use: "powerful", "seamless", "robust",
  "enterprise-grade", "cutting-edge", "revolutionary", "game-changing". If you
  would cringe reading it in someone else's post, cut it.
- Every long-form artifact (blog, reddit) includes at least one code block or
  YAML snippet a reader could copy.
- Every long-form artifact links to https://app.datacontractgate.com/llm-integration.md
  in the closing paragraph, not the opener.
- Never claim shipped-in-prod behavior the RFC does not describe. If the RFC is
  Draft, frame the post as design/plans, not release.
- No emojis. No hashtag spam. On X, ≤2 hashtags total, at the end of the last
  tweet only.

The user will paste an RFC. Your job is to produce four separate artifacts, one
per user message. Follow the specific format in each user message exactly.
"""

# ── Per-artifact prompts ──────────────────────────────────────────────────────
ARTIFACT_PROMPTS = {
    "blog": """\
Write a dev.to-ready blog post based on the RFC below. Requirements:
- 800-1200 words.
- dev.to frontmatter at top:
    ---
    title: "<title>"
    description: "<140-160 char SEO description>"
    tags: [dataengineering, kafka, rust, datacontracts]     # pick 3-4 real tags
    published: false
    ---
- Structure: (1) problem in the wild (1-2 paras), (2) why existing tools don't
  solve it, (3) what we shipped and how it works (with 1+ code or YAML block),
  (4) the interesting engineering trade-off, (5) close with a "try it" line
  linking to https://app.datacontractgate.com/llm-integration.md.
- Use "we" throughout.
- Output the blog post ONLY. No preamble, no explanation, no trailing "Hope
  this helps!" — just the frontmatter + body ready to paste into dev.to.
""",
    "hn": """\
Write a Show HN submission based on the RFC below. Format the output as:

Title: Show HN: <title, ≤80 chars, must start with "Show HN:">
URL: <if the RFC ships a public URL, use it; else use https://github.com/nightmoose/contractgate>

First comment (paste as first reply after posting):
<3-5 paragraphs of author context. Tell the story of the problem, the constraint
that made it hard, and one specific technical detail HN readers will latch onto.
End with an ask: "Would love feedback on X." No sales pitch, no CTA to sign up.
Link to /llm-integration.md is fine mid-comment; do not put a link in the last
sentence — that's the "sales close" pattern HN hates.>

Output the three sections above ONLY. No preamble.
""",
    "x-thread": """\
Write a 5-7 tweet thread for @contractgate (X) based on the RFC below.

Format each tweet on its own numbered line as:

[1/N] <tweet text>   (chars: NN)
[2/N] <tweet text>   (chars: NN)
...

Rules:
- Each tweet ≤280 chars including the "[k/N] " prefix.
- Tweet 1 hooks with the problem, not the product. No emoji, no all-caps, no
  "🧵" thread emoji.
- Middle tweets show the solution, ideally including a screenshot/gist idea
  in [brackets] where a visual would land — e.g. "[screenshot: quarantine tab
  with 3 bad events]".
- Last tweet: link to the blog post (placeholder: [BLOG_URL]) + one soft CTA.
- ≤2 hashtags total, at the end of the last tweet only, and only if genuinely
  used by the target audience (#dataengineering, #kafka OK; #tech no).
- Count characters honestly. If a tweet is 291 chars, tighten it, don't lie.

Output the numbered thread ONLY. No preamble.
""",
    "reddit-dataengineering": """\
Write an r/dataengineering post based on the RFC below.

Format:

Subreddit: r/dataengineering
Title: <plain-English title, ≤300 chars, NO "Show HN"-style framing, NO "I built
       X" opener — frame as a problem you hit or a lesson learned>

Body:
<600-900 words, first-person singular ("I ran into"). Practitioner voice. The
first two paragraphs must NOT mention ContractGate; describe the problem
generically. Mention ContractGate only in the second half, and only once by
name — the rest of the time refer to it as "the gate we built" or similar.
Include at least one code or YAML block. Close with: "Repo:
https://github.com/nightmoose/contractgate — playbook for wiring it into an
existing stack: https://app.datacontractgate.com/llm-integration.md. Happy to
answer questions." No signup CTA. No "check us out".>

Output the three sections above ONLY. No preamble.
""",
}


# ── RFC + status parsing ─────────────────────────────────────────────────────
@dataclass
class RfcContext:
    number: str  # zero-padded 3-digit, e.g. "089"
    path: Path
    title: str
    status: Optional[str]
    shipped_branch: Optional[str]
    body: str


def zpad_rfc(raw: str) -> str:
    digits = re.sub(r"[^\d]", "", raw)
    if not digits:
        sys.exit(f"error: could not parse RFC number from '{raw}'")
    return digits.zfill(3)


def find_rfc_file(number: str) -> Path:
    matches = sorted(RFCS_DIR.glob(f"{number}-*.md"))
    if not matches:
        sys.exit(f"error: no RFC file matches docs/rfcs/{number}-*.md")
    if len(matches) > 1:
        joined = ", ".join(p.name for p in matches)
        sys.exit(f"error: multiple RFC files matched {number}: {joined}")
    return matches[0]


def parse_status(number: str) -> tuple[Optional[str], Optional[str]]:
    """Return (status, shipped_branch) for the RFC from docs/STATUS.md."""
    if not STATUS_FILE.exists():
        return (None, None)
    # STATUS.md rows use the zero-padded number (e.g. `| 089 |`). Interpolating
    # `int(number)` would strip the leading zero and never match.
    row_re = re.compile(
        rf"^\|\s*{number}\s*\|\s*\[[^\]]+\]\([^)]+\)\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|",
        re.MULTILINE,
    )
    m = row_re.search(STATUS_FILE.read_text())
    if not m:
        return (None, None)
    status = m.group(1).strip() or None
    branch = m.group(2).strip().strip("`") or None
    if branch == "—":
        branch = None
    return (status, branch)


def load_rfc(raw_number: str) -> RfcContext:
    number = zpad_rfc(raw_number)
    path = find_rfc_file(number)
    body = path.read_text()
    title_match = re.search(r"^#\s+(.+?)\s*$", body, re.MULTILINE)
    title = title_match.group(1).strip() if title_match else path.stem
    status, branch = parse_status(number)
    return RfcContext(
        number=number,
        path=path,
        title=title,
        status=status,
        shipped_branch=branch,
        body=body,
    )


# ── Draft generation ─────────────────────────────────────────────────────────
def build_user_message(rfc: RfcContext, artifact: str) -> str:
    header = ARTIFACT_PROMPTS[artifact]
    context = (
        f"\n\n---\n\n"
        f"RFC number: {rfc.number}\n"
        f"RFC title: {rfc.title}\n"
        f"Status: {rfc.status or 'unknown'}\n"
        f"Shipped in branch: {rfc.shipped_branch or 'not yet shipped'}\n"
        f"\n---\n\nRFC BODY:\n\n{rfc.body}\n"
    )
    return header + context


def dry_run_stub(rfc: RfcContext, artifact: str) -> str:
    """A placeholder that verifies I/O and layout without an API call."""
    return (
        f"# DRAFT STUB — {artifact} — RFC-{rfc.number}\n\n"
        f"Title (source): {rfc.title}\n"
        f"Status: {rfc.status or 'unknown'}\n"
        f"Shipped in branch: {rfc.shipped_branch or 'not yet shipped'}\n"
        f"Source RFC: {rfc.path.relative_to(REPO_ROOT)}\n\n"
        f"[--dry-run] No Claude call made. Re-run without --dry-run to generate.\n"
    )


def call_claude(rfc: RfcContext, model: str, use_cache: bool) -> dict[str, str]:
    try:
        import anthropic  # noqa: WPS433 (local import so --dry-run doesn't require it)
    except ImportError:
        sys.exit(
            "error: `anthropic` package not installed.\n"
            "  fix: pip install anthropic\n"
            "  or:  python scripts/marketing/rfc_to_content.py <n> --dry-run"
        )

    if not os.environ.get("ANTHROPIC_API_KEY"):
        sys.exit("error: ANTHROPIC_API_KEY not set in environment")

    client = anthropic.Anthropic()

    system_block: list[dict] = [{"type": "text", "text": SYSTEM_PROMPT}]
    if use_cache:
        system_block[0]["cache_control"] = {"type": "ephemeral"}

    results: dict[str, str] = {}
    for artifact in ARTIFACTS:
        user_msg = build_user_message(rfc, artifact)
        print(f"  · calling Claude for {artifact}...", file=sys.stderr)
        response = client.messages.create(
            model=model,
            max_tokens=4096,
            system=system_block,
            messages=[{"role": "user", "content": user_msg}],
        )
        text = "".join(
            block.text for block in response.content if block.type == "text"
        ).strip()
        results[artifact] = text
    return results


# ── I/O ──────────────────────────────────────────────────────────────────────
def write_drafts(rfc: RfcContext, drafts: dict[str, str], out_dir: Path) -> list[Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []
    for artifact, text in drafts.items():
        path = out_dir / f"{artifact}.md"
        path.write_text(text.rstrip() + "\n")
        written.append(path)
    return written


REVIEW_CHECKLIST = """\
Review checklist before publishing:
  □ Numbers cited match the RFC (or are removed).
  □ No marketing adjectives snuck in ("powerful", "seamless", "revolutionary", ...).
  □ Blog + Reddit include a code/YAML block a reader can copy.
  □ HN title starts with "Show HN:" and is ≤80 chars.
  □ Every X tweet is ≤280 chars including the [k/N] prefix.
  □ Reddit body's first two paragraphs do not name ContractGate.
  □ Closing link points to https://app.datacontractgate.com/llm-integration.md.
  □ Nothing claims prod behavior that isn't shipped (check RFC status).
"""


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Turn a ContractGate RFC into publishable drafts."
    )
    parser.add_argument("rfc", help="RFC number, e.g. 89 or 089")
    parser.add_argument(
        "--out",
        type=Path,
        help="Output directory (default: docs/marketing/drafts/rfc-<N>)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Skip Claude call; write placeholder stubs only.",
    )
    parser.add_argument(
        "--model",
        default=DEFAULT_MODEL,
        help=f"Claude model ID (default: {DEFAULT_MODEL})",
    )
    parser.add_argument(
        "--no-cache",
        action="store_true",
        help="Disable prompt caching on the system prompt.",
    )
    args = parser.parse_args()

    rfc = load_rfc(args.rfc)
    out_dir = args.out or DEFAULT_OUT_ROOT / f"rfc-{rfc.number}"

    print(f"RFC-{rfc.number}: {rfc.title}", file=sys.stderr)
    print(f"  source:  {rfc.path.relative_to(REPO_ROOT)}", file=sys.stderr)
    print(f"  status:  {rfc.status or 'unknown'}", file=sys.stderr)
    print(f"  branch:  {rfc.shipped_branch or 'not yet shipped'}", file=sys.stderr)
    print(f"  out:     {out_dir.relative_to(REPO_ROOT) if out_dir.is_relative_to(REPO_ROOT) else out_dir}", file=sys.stderr)

    if args.dry_run:
        print("  mode:    DRY-RUN (no API call)", file=sys.stderr)
        drafts = {artifact: dry_run_stub(rfc, artifact) for artifact in ARTIFACTS}
    else:
        print(f"  mode:    live ({args.model}, cache={'off' if args.no_cache else 'on'})", file=sys.stderr)
        drafts = call_claude(rfc, model=args.model, use_cache=not args.no_cache)

    written = write_drafts(rfc, drafts, out_dir)
    print("", file=sys.stderr)
    print("Wrote:", file=sys.stderr)
    for p in written:
        print(f"  {p.relative_to(REPO_ROOT) if p.is_relative_to(REPO_ROOT) else p}", file=sys.stderr)
    print("", file=sys.stderr)
    print(REVIEW_CHECKLIST, file=sys.stderr)


if __name__ == "__main__":
    main()
