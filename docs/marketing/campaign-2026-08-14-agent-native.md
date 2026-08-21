# Campaign — Agent-Native Onboarding (Aug 14–20, 2026)

**Implements:** [`marketing-plan.md`](marketing-plan.md) §4 Loop E + §6 backlog item #1 (RFC-089).
**Lead angle:** paste one URL into a coding agent and it wires up your pipeline.
**Voice:** [`marketing-plan.md`](marketing-plan.md) §7. Every post below already follows it — don't "polish" the adjectives back in.
**Primary account:** personal (@alexsuarez). Brand account reposts, never duplicates.

**Why this angle:** it came from a real user, unprompted — *"the easiest way to reduce friction for me is if i can just paste it into claude where it has the details and it can implement it for me."* That quote is the campaign. Per §2 segment 3, this audience is reachable and, per §4 Loop E, competitors have no presence in it.

**The one asset everything points at:** <https://app.datacontractgate.com/llm-integration.md>

---

## Day 0 — Friday Aug 14, before anything ships

Both of these are prerequisites, not nice-to-haves.

- [ ] **Wire the OG image.** No `openGraph` metadata exists in `dashboard/app/layout.tsx` and there's no `opengraph-image` file, so every link posted this week renders a blank grey card. Asset exists: `datacontractgate_website/marketing/assets/p0/01-hero-og.jpg`.
- [ ] **Enable Vercel Web Analytics.** KPI #3 (`/llms.txt` + `/llm-integration.md` fetches, target 200/week) has no data source without it. One toggle.
- [ ] Confirm `/llms.txt` and `/llm-integration.md` both return 200 as `text/plain`. (Verified 2026-08-13 — recheck after any deploy.)
- [ ] Generate supporting drafts: `python scripts/marketing/rfc_to_content.py 89`. Use them as raw material for Day 4/5, not as final copy.

---

## Day 1 — Friday Aug 14 · X · single post

Opens the week on the origin story. No product pitch, no link in the opener.

*(273 chars)*

> Someone told me how they adopt a new dev tool now:
>
> "the easiest way to reduce friction for me is if i can just paste it into claude where it has the details and it can implement it for me. so like an article with the github repo."
>
> Not docs. A URL their agent can execute.

Reply to your own post (keeps the link out of the main tweet, which the algorithm prefers):

> It's live: https://app.datacontractgate.com/llm-integration.md
>
> Paste that + the repo into Claude/Cursor/Codex, say "implement this", and the agent finds your event shape, infers a contract, deploys it, and wires validation into your producer.

---

## Day 2 — Saturday Aug 15 · X · single post

Weekend = low reach. Spend it on the cheapest high-value post: a concrete broken event. §7 says show the bad event, then the fix.

Split across post + reply — two code blocks don't fit in 280. (If you're on X Premium, post them as one.)

**Post** *(199 chars)*

> This event is valid JSON. It passes your schema. It still poisons your warehouse:
>
> { "user_id": "u_123", "event_type": "purchsae", "amount": -49.99 }
>
> Typo'd enum. Negative amount. Both schema-valid.

**Reply** *(243 chars)*

> A schema checks shape. A contract checks meaning:
>
> - name: event_type
>   type: string
>   enum: ["click", "view", "purchase", "login"]
> - name: amount
>   type: number
>   min: 0
>
> Rejected at ingest, quarantined, replayable once the producer is fixed.

---

## Day 3 — Sunday Aug 16 · no posting

Write the dev.to post. Title from backlog #1: **"How I made ContractGate agent-installable: one URL and 'implement this'"**

Outline (800–1200 words, §7 rules):

1. The user quote. Verbatim, up top.
2. Why existing entry points fail an agent — README is Docker-demo-first; the quickstart hand-escapes YAML into a curl body; `/docs` is React, so a fetch returns a JS shell, not instructions.
3. What an agent-executable doc has to contain: ordered steps encoding the dependency chain (key → sample → contract → deploy → wire → verify), exact request/response shapes, the locked YAML format inlined so the agent can't invent fields, and a do-not list.
4. The drift problem and the fix — a doc agents execute is an API surface, so `tests/llm_docs_test.rs` asserts every endpoint the playbook cites still exists in the router and that its example contract compiles.
5. `llms.txt` as a convention, and why serving raw markdown matters.
6. Close: the paste prompt, and the link.

---

## Day 4 — Monday Aug 17 · dev.to publish + X thread

Publish the blog post, then thread it. Thread ≠ blog summary — it's the argument, standalone.

**[1/6]**
> Docs are written for humans who read. Increasingly the first thing to touch your product is an agent that executes.
>
> Those are different artifacts. Most of us have only built the first one.

**[2/6]**
> An agent fetching a typical docs site gets a JavaScript shell. Our /docs pages are React — a plain fetch returns nothing useful.
>
> The README? Docker-demo-first. Correct for humans, useless for an agent wiring a cloud pipeline.

**[3/6]**
> So the doc has to be ordered as a dependency chain, not a feature tour:
>
> key → find the event shape → infer a contract → review it → deploy → wire the producer → dry-run verify
>
> Skip a step and the next one has nothing to work with.

**[4/6]**
> The part I underestimated: an agent will invent fields.
>
> Fix is to inline the exact contract format in the doc, with a table of every legal key. Now it copies instead of guessing.

**[5/6]**
> A doc that agents execute is an API surface. It rots like one.
>
> So there's a CI test that extracts every `METHOD /path` from the playbook and asserts the route still exists, plus compiles the example contract through the real engine.

**[6/6]**
> Full writeup: [dev.to link]
>
> The artifact itself: https://app.datacontractgate.com/llm-integration.md
>
> Paste it into Claude with the repo and say "implement this."

---

## Day 5 — Tuesday Aug 18 · Show HN · 8:30am ET

Tuesday morning ET is the strongest HN window. Title ≤ 80 chars.

**Title:**
> Show HN: A doc written for coding agents, not humans, that installs my product

**URL:** `https://app.datacontractgate.com/llm-integration.md`

Submitting the doc itself, not the homepage. It's the artifact under discussion and it stands alone without signup.

**First comment (post immediately after submitting):**

> Author here. ContractGate validates events against semantic data contracts at ingest — enum, pattern, range, and required-field checks, not just JSON Schema shape — and quarantines the ones that fail.
>
> A user told me the way he adopts tools now is pasting a URL into Claude and saying "implement this." Everything I'd written was for a human reader: the README leads with a Docker demo, the docs are React pages that return a JS shell to a plain fetch.
>
> So the linked file is written in the imperative, to the agent. Ordered as a dependency chain (get a key → find the event shape in the repo → infer a contract from real samples → review it → deploy → wire the producer → dry-run verify), with exact request and response shapes, and the contract format inlined verbatim so it doesn't invent fields.
>
> Two things I got wrong first time:
>
> 1. Agents confabulate schema fields unless the legal keys are enumerated in front of them.
> 2. A doc that agents execute is an API surface and decays like one. There's now a CI test that pulls every `METHOD /path` out of the doc and fails the build if the route no longer exists, plus compiles the example contract through the real validator.
>
> Self-hosting works without an account: `git clone` + `make demo` runs the actual gateway locally.
>
> Happy to be told this is the wrong shape for the problem.

**Rules:** no upvote requests anywhere. Answer every comment for the first three hours. If someone says the doc is just good documentation — agree, that's the point.

---

## Day 6 — Wednesday Aug 19 · Reddit + X

### r/dataengineering — practitioner voice, first-person singular

§3 is explicit: no "I built X" openers. Problem first. ContractGate is not named until the third paragraph.

**Title:** Schema-valid events that are still wrong — what are you actually doing about it?

**Body:**

> Spent the last while dealing with a category of bad data that schema validation doesn't catch. The event is well-formed JSON, every field is the declared type, the schema registry is happy — and the payload is still wrong. A typo'd enum value. A negative amount. An epoch timestamp in milliseconds where everything else is seconds.
>
> These pass every check at ingest and turn into a support ticket a week later, after the dashboards have been wrong for six days.
>
> The approaches I've seen: dbt tests catch it after landing (too late, the warehouse already has it), Great Expectations runs on batches (same problem, plus it's a separate orchestration surface), Kafka Streams filters push the logic into a consumer each team reimplements slightly differently.
>
> What I ended up building instead is a validation gate at ingress that enforces semantics — enums, patterns, numeric ranges, required fields, plus a glossary — and routes failures to quarantine for replay after you fix the producer. It's open source and self-hostable (`make demo`), and it runs in front of Kafka or plain HTTP: https://github.com/nightmoose/contractgate
>
> Genuinely curious what everyone else does here. Is this a problem people are solving at ingest, or is downstream testing just accepted as the answer?

Answer every reply. Do not link the cloud product in this thread.

### X · single post, same day

> Per-event validation p99: 31µs, measured with a warm contract cache.
>
> That number is why this runs at ingest instead of as a dbt test after the fact. At 86k events/sec/core, validating in the hot path costs less than the network hop you're already paying for.

---

## Day 7 — Thursday Aug 20 · X thread · build-in-public wrap

Post it whether the week went well or badly — the honesty is the content.

**[1/4]**
> Ran a week of one idea: make the product installable by pasting a URL into a coding agent.
>
> Numbers, good and bad: [fill in — llms.txt fetches, HN rank/karma, signups, first-ingest conversions]

**[2/4]**
> What worked: [fill in]

**[3/4]**
> What didn't: [fill in]

**[4/4]**
> The artifact, if it's useful to anyone building the same thing: https://app.datacontractgate.com/llm-integration.md
>
> It's just markdown. Steal the structure.

---

## Measurement — check Thursday, against §5

| Metric | Where | This week's bar |
|---|---|---|
| `/llms.txt` + `/llm-integration.md` fetches | Vercel Analytics (enable Day 0) | any non-zero baseline; 200/wk is the 90-day target |
| Signup → first `/v1/ingest` within 24h | usage rows | ≥1 real completion beats any impression count |
| HN karma | HN | median ≥30 per §5; ≥100 is a good day |
| GitHub stars added | GH API | 15/wk is the 90-day target; this week just measure |
| Bot signups | `auth.users` | separate real from bots before reading signup numbers |

**Do not** report impressions or follower count. §5 is explicit about this.

---

## If it goes sideways

- **HN post sinks without comments.** Normal — timing dominates. Don't repost the same URL. It becomes a section of the consolidated "v0.2 shipped" Show HN (§4 Loop D) in 6–8 weeks.
- **Reddit post gets removed.** Most likely a self-promotion rule. Message the mods, don't repost. The GitHub link in paragraph four is the usual trigger; move it to a comment next time.
- **Someone finds a bug in the playbook.** Fix it same day and reply with the commit. That response is worth more than the original post.
- **A wave of bot signups arrives.** Expected — the captcha is gone. Rules are drafted in the 2026-08-13 session notes: rate-limit `/auth/*` excluding `/auth/callback`, Challenge not Deny. Don't let bot noise get read as campaign traffic.
