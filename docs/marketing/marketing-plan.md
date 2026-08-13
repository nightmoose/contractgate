# ContractGate Marketing Plan

**Status:** Living doc. Owned by @alexsuarez.
**Last revised:** 2026-08-09.
**Companion:** [`../../scripts/marketing/rfc_to_content.py`](../../scripts/marketing/rfc_to_content.py) — the automation that turns shipped RFCs into publishable drafts.

---

## 1. Positioning

**One sentence:** ContractGate is the runtime enforcement gate for streaming data contracts — you write a semantic YAML contract, we validate every event at ingress in sub-millisecond, and route the bad ones to quarantine before consumers see them.

**Who this is for:** platform, data, and MLOps engineers who own a Kafka / Kinesis / HTTP ingest path and have already been burned by a producer shipping a semantically broken event that was schema-valid.

**Who this is *not* for:** teams looking for a shift-left change-management workflow in the producer's IDE (that's Gable), or a schema registry replacement (that's Confluent SR + rules). We are the *runtime choke point* that producers cannot skip.

**Elevator differentiator (memorize):** *"A schema is not a contract. We enforce the contract."* — direct nod to Derosiaux's Dec 2025 thesis, which is the clearest external articulation of the problem we solve.

---

## 2. Audience segments (evidence-backed)

| Segment | Signal that they're the buyer | Primary channel to reach |
|---|---|---|
| **Kafka-heavy platform engineers** | Search "kafka schema registry data contracts", read Conduktor/Confluent blogs, attend Kafka Summit | Technical blog posts, X, r/apachekafka |
| **Data engineers dealing with quarantine / bad-data incidents** | Search "quarantine kafka events", "dead letter queue best practices", follow Data Engineering Weekly | dev.to, r/dataengineering, DE Weekly Substack |
| **Agent-coding teams wiring pipelines with Cursor/Claude/Codex** | Paste raw URLs into an LLM and say "implement this" (see RFC-089 verbatim user feedback) | `/llms.txt` discoverability, HN, GitHub SEO |

Each segment gets a distinct content lane. Do not blend voices in the same post — a platform engineer skimming for latency numbers hates the "here's what I did with an AI agent" opener.

---

## 3. Channels — ranked by fit

### Tier 1 (weekly cadence, high ROI)

1. **Hacker News** — Show HN every meaningful ship (roughly one per shipped RFC of user-visible size). Ships without a corresponding post are wasted marketing surface.
2. **Technical blog** — dev.to first (zero setup, built-in audience). Move to `blog.datacontractgate.com` only after 5+ posts land and analytics show organic traffic worth the SEO investment.
3. **X (@contractgate, new account)** — one thread per shipped RFC + daily-ish micro-content (bad-event examples, quarantine screenshots, latency numbers). Personal account (@alexsuarez) reposts/quote-tweets the brand account, no duplication of effort.

### Tier 2 (opportunistic, 2-3x/month)

4. **Reddit** — r/dataengineering (weekly-ish), r/rust (when RFC touches engine internals), r/mlops (when RFC touches AI pipelines), r/apachekafka (when RFC touches ingress). **Practitioner voice only** — no "I built X, check it out" openers. Frame as a problem you hit, then link.
5. **Data Engineering Weekly** — pitch a guest post every 2-3 months. Ananth reads DMs on X and Substack.
6. **GitHub discoverability** — awesome-lists (awesome-kafka, awesome-data-engineering, awesome-rust-applications), README SEO, `topics:` tags. One-shot work per list, compounds forever.

### Tier 3 (low priority for now)

7. **LinkedIn** — cross-post finished blog posts only. No original content. You hate it; don't fake it.
8. **YouTube** — gated on recording one 3-minute demo you don't hate. Not a Q3 priority.
9. **Podcasts** — inbound-only. Don't chase.

### Explicitly not doing

- Paid ads (SEM, sponsored newsletters) until organic proves the funnel converts.
- Conference sponsorships (too early; no clear ICP validated).
- Cold email at scale (the [private-beta-outreach-pack](private-beta-outreach-pack.md) is the right shape — individualized, capped at ~10 people).

---

## 4. Compounding loops (the actual strategy)

Content that doesn't compound is a treadmill. Every loop below produces artifacts that keep working after the day they ship.

### Loop A — RFC → content pipeline *(this is what we're automating)*

Every shipped RFC of user-visible size produces: (1) a technical blog post, (2) a Show HN draft, (3) an X thread, (4) a Reddit post variant. See `scripts/marketing/rfc_to_content.py`. Cost per RFC: ~5 minutes of review after generation. Compounds because we already write RFCs — this is pure conversion of existing work into distribution.

### Loop B — SEO landing pages per contract format

`app.datacontractgate.com/contracts/csv`, `/kafka`, `/openapi`, `/avro`, `/proto`. Each page: what a contract looks like for this format, live-inference demo, curl snippet, link to `/llm-integration.md`. Compounds because these are the queries a developer types when they hit the problem. One page = one long-tail keyword captured forever.

### Loop C — Public sample-contracts registry

A GitHub repo (`contractgate/sample-contracts`) with 20-30 real-world contracts (Stripe webhook, Segment events, Kafka Connect DBZ CDC, etc.) — copy-paste ready. Every contract file is a permalinkable landing page and a search-result surface. Compounds via GitHub's own discovery + our own docs linking in.

### Loop D — Show HN cadence tied to major ships

Every 6-8 weeks, package the last several RFCs into a "ContractGate v0.X shipped" post. HN treats consolidated ships better than drip-feed. Aim for launch narratives, not changelog dumps. Compounds because each successful Show HN produces backlinks and a new signup cohort.

### Loop E — Agent-native discoverability *(already 60% built via RFC-089)*

`/llms.txt` + `/llm-integration.md` are the surface that captures the "paste-this-into-Claude" traffic — a channel that literally didn't exist 18 months ago and where competitors have zero presence today. Compounds because each new LLM-fluent developer bookmarks the paste-prompt.

---

## 5. KPIs (5 metrics, no more)

Tracked weekly in a Notion / Airtable dashboard (not built yet — Q3 TODO):

| Metric | Source | Target 90d |
|---|---|---|
| **Signup → first successful `/v1/ingest` within 24h** | Existing usage rows (already tracked per RFC-089 success metric) | 40% |
| **GitHub stars added / week** | GH API | 15/week trailing avg by day 90 |
| **`/llms.txt` + `/llm-integration.md` fetch count** | Vercel analytics | 200/week |
| **HN karma per submission** | HN API | Median ≥ 30, at least one ≥ 100 |
| **Cold-outbound reply rate** | Manual tracking sheet from private-beta-outreach-pack | ≥ 25% reply, ≥ 10% demo-booked |

**Not KPIs:** vanity followers, impressions, "engagements", email list size. Signup + activation only.

---

## 6. 90-day content backlog (mapped to shipped RFCs)

Order = publish order. Each entry: **[RFC]** → post angle → target channel.

1. **RFC-089** → "How I made ContractGate agent-installable: one URL and 'implement this'" → dev.to + HN Show HN
2. **RFC-086** → "The three-line change that made us stop storing customer payloads by default" → dev.to + X thread
3. **RFC-083** → "Metering streaming ingress without adding a microsecond to the hot path" → dev.to + r/rust
4. **RFC-081** → "Reconciling Kafka quarantine + replay is harder than it looks — here's the race we hit" → dev.to + r/dataengineering + r/apachekafka
5. **RFC-084** → "We built a Slack lead-intake bot in a weekend and it 3x'd our reply rate" → dev.to (meta-content, appeals to founders)
6. **RFC-082** → "The pilot report format that closed our first three design partners" → LinkedIn cross-post + X (this one's OK for LinkedIn — it's founder-narrative content)
7. **RFC-079** → "Killing the JS inference engine: one Rust core for every contract source" → r/rust + dev.to
8. **RFC-077** → "A data contract for RAG ingestion — what fields you actually need" → r/mlops + dev.to
9. **Consolidated Show HN**: "ContractGate v0.2 — semantic contract enforcement for Kafka" — bundles RFCs 081-089 → HN + X
10. **Loop B kickoff**: `/contracts/csv` landing page → SEO play, no promotion needed

---

## 7. Voice + style rules (for the automation and manually)

- **First-person plural** in blog posts ("we hit"), **first-person singular** in Reddit ("I hit"). Never "you" in openers.
- Lead with the problem, not the product. Show a broken event, then the fix.
- Latency numbers, event counts, quarantine screenshots. Concrete beats abstract every time.
- Zero marketing adjectives ("powerful", "seamless", "enterprise-grade"). If you'd cringe reading it in someone else's post, cut it.
- Every post includes at least one code block or YAML snippet the reader could copy.
- Every post links to `/llm-integration.md` in the closing paragraph, not the opener.
- Never say "revolutionary" or "game-changing". Ever.

---

## 8. Explicitly not doing (Q3)

- Rebuilding the website. It works.
- Building a "content calendar" tool. The RFC pipeline is the calendar.
- Auto-posting to any platform. Every draft is human-reviewed before publish.
- Growth hacks (referral loops, viral coefficients, PLG scoring). Too early; we need the top-of-funnel first.
- SEO tooling (Ahrefs, Semrush) subscriptions. Free Search Console + Vercel analytics until we outgrow them.

---

## 9. Review cadence

- Weekly (Monday, 15min): what shipped, what drafts got generated, what got published, what got replies.
- Monthly (first Friday): pull KPI numbers, adjust channel mix, revise backlog.
- Quarterly: rewrite this doc from scratch based on what actually worked.
