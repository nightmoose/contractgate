# Marketing drafts

This folder holds LLM-generated content drafts produced by
[`../../../scripts/marketing/rfc_to_content.py`](../../../scripts/marketing/rfc_to_content.py).

**Everything in this folder (except this README) is gitignored.** Drafts are
noisy, iterated locally, and don't belong in the repo history. Promote
finalized posts *out* of `drafts/` — to the website repo, a public gist,
dev.to as `published: false`, or wherever the final home lives.

## Generate drafts for an RFC

```bash
# Dry-run first (no API call, verifies the pipeline):
python scripts/marketing/rfc_to_content.py 89 --dry-run

# Live run (requires ANTHROPIC_API_KEY in env, `pip install anthropic`):
python scripts/marketing/rfc_to_content.py 89
```

Output lands at `docs/marketing/drafts/rfc-089/`:

```
rfc-089/
├── blog.md                       # dev.to-ready, 800-1200 words + frontmatter
├── hn.md                         # Show HN title + URL + first comment
├── x-thread.md                   # 5-7 numbered tweets, char-counted
└── reddit-dataengineering.md     # subreddit + title + body
```

## Review before publishing

Every draft is a first draft. Do not paste-and-post. The script prints a
checklist to stderr on completion — actually run through it. Common failure
modes to catch by hand:

- Marketing adjectives snuck in ("powerful", "seamless", "revolutionary").
- Numbers cited that aren't in the RFC (Claude will confabulate throughput
  figures if you let it).
- HN title exceeds 80 chars.
- An X tweet exceeds 280 chars including its `[k/N]` prefix.
- Reddit body names ContractGate in the first two paragraphs.
- Post claims shipped-in-prod behavior for a Draft-status RFC.

## Related

- Strategy this folder implements: [`../marketing-plan.md`](../marketing-plan.md), §4 Loop A.
- Voice + style rules the script prompts against: [`../marketing-plan.md`](../marketing-plan.md), §7.
- Cold-outbound (unrelated motion, one-off asset): [`../private-beta-outreach-pack.md`](../private-beta-outreach-pack.md).
