# Kickoff Prompt Template — paste this to start a chunk

Copy verbatim, replace `{{N}}`, `{{NNN}}`, and `{{slug}}`.

---

```
You are picking up Punchlist Chunk {{N}} for ContractGate.

READ FIRST, IN THIS ORDER, BEFORE WRITING ANY CODE:
1. CLAUDE.md (root) — project rules. Note: ultra-concise, one issue at a
   time, never break existing behavior, branch name format.
2. docs/punchlist/{{NN}}-{{slug}}.md — the chunk's scope summary.
3. docs/rfcs/{{NNN}}-{{slug}}.md — the RFC for this chunk. It is in
   Draft state. Decisions are pre-recommended in the "Decisions" table.
4. MAINTENANCE_LOG.md — the most recent runs. Match that style for
   logging your run.
5. Any RFC the chunk RFC says it depends on (the "Depends on" header
   row). Read those before designing anything new.

DO NOT WRITE A NEW RFC. The RFC already exists. Your job is to:
  a) Confirm or contest each decision in the RFC's Decisions table.
     If you contest one, stop and surface it to the user with a
     specific alternative — do not proceed unilaterally.
  b) After the user signs off (explicit "go" or "approved"), flip the
     RFC's Status header to "Accepted (YYYY-MM-DD) — sign-off on Q1..QN
     (recommendations)" and proceed.
  c) Implement strictly within the RFC's Goals + Design + Rollout.
     Anything not in the RFC is out of scope; surface it as a follow-up,
     do not silently build it.

WORKING RULES:
- Branch: nightly-maintenance-$(date +%Y-%m-%d). Create it before
  touching code.
- Caveman style in chat. Result first. No preambles.
- One issue at a time. Land each Rollout step before starting the next.
- After every code change: `cargo check`, then `cargo test` for any
  module touched. Dashboard: `cd dashboard && npm run build` if you
  touched it.
- Validation engine p99 must stay under 15ms. If you touch the hot
  path, measure before/after.
- Audit honesty rule: any code that writes `contract_version` to
  audit_log must use the version that actually matched, never a default.

WHEN YOU FINISH:
1. Append a run entry to MAINTENANCE_LOG.md in the existing style.
2. Update the chunk RFC's "Rollout" section with checkmarks on each
   completed step.
3. Mark the corresponding line(s) in
   contractgate-upgrade-punchlist.md (in user uploads) as done — but
   only via a comment in your final summary, not by editing the upload.
4. Final chat message: caveman-style summary listing files touched,
   tests run, anything deferred to a follow-up RFC.

DO NOT:
- Write your own RFC. The RFC exists.
- Implement deferred items listed in the RFC's "Deferred" or
  "Non-goals" sections.
- Skip the dependency RFCs.
- Refactor unrelated code while you're in there. Note debt; defer it.
- Mock the database in integration tests.
- Use bullet points or headers in chat. Caveman style.

Begin by reading the five documents above and reporting back, in
under 200 words, your understanding of the scope and any decision in
the RFC's Decisions table you'd flag for review before starting.
```

---

## Chunk → RFC quick reference

| Chunk | Punchlist file | RFC |
|---|---|---|
| 1 | `docs/punchlist/01-inference-family.md` | `docs/rfcs/006-inference-formats.md` *(landed)* |
| 2 | `docs/punchlist/02-cli-gitops-core.md` | `docs/rfcs/007-cli-gitops-core.md` |
| 3 | `docs/punchlist/03-breaking-change-story.md` | `docs/rfcs/008-breaking-change-story.md` |
| 4 | `docs/punchlist/04-observability-quick-wins.md` | `docs/rfcs/009-observability-quick-wins.md` |
| 5 | `docs/punchlist/05-self-hosted-basics.md` | `docs/rfcs/010-self-hosted-basics.md` |
| 6 | `docs/punchlist/06-sdk-rollout.md` | `docs/rfcs/011-sdk-rollout.md` |
| 7 | `docs/punchlist/07-templates-marketplace.md` | `docs/rfcs/012-templates-marketplace.md` |
| 8 | `docs/punchlist/08-multitenant-saas.md` | `docs/rfcs/013-multitenant-saas.md` *(gated on RFC-001)* |
