# CLAUDE.md - ContractGate Maintenance

**Project:** ContractGate - High-performance semantic contract enforcement gateway (Patent Pending)

**Current Phase:** Fully built. Now iterating on missing functions + eliminating tech debt. Performance and correctness are non-negotiable.

**Tech Stack**
- Rust (Axum) backend – validation engine must stay <15ms p99
- Next.js 15 + TS + Tailwind frontend (do not overhaul)
- Supabase (contracts, audit logs, quarantined events)

**Strict Priorities**
1. Fix missing functions
2. Eliminate tech debt (refactor, clean, optimize)
3. Preserve all existing functionality
4. Keep validation engine fast, correct, and patent-core

**Semantic Contract Format (locked)**
Use this clean YAML structure:

```
version: "1.0"
name: "user_events"
description: "Contract for user interaction events"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]+$"
    - name: event_type
      type: string
      required: true
      enum: ["click", "view", "purchase", "login"]
    - name: timestamp
      type: integer
      required: true
    - name: amount
      type: number
      required: false
      min: 0

glossary:
  - field: amount
    description: "Monetary amount in USD"
    constraints: "must be non-negative"

metrics:
  - name: total_revenue
    formula: "sum(amount) where event_type = 'purchase'"
```

## Rules (always obey)
- Be ultra-concise. Result first. Short sentences. No fluff, no explanations unless asked.
- One issue or refactor at a time unless told otherwise.
- Always test changes (cargo test, cargo check).
- Never break existing behavior.
- Prefer simple, idiomatic Rust. Comment only when it adds real value.
- Create branch: nightly-maintenance-$(date +%Y-%m-%d)-<rfc-slug> for all changes (e.g. nightly-maintenance-2026-05-16-rfc024).
- Use only necessary files. Minimize context.
- **When adding or changing user-facing functionality:** (1) check whether any existing `docs/` page covers that surface and update it for breaking changes; (2) add a new `docs/<feature>-reference.md` if no doc exists. User-facing = any CLI flag, API endpoint, contract field, or config key a user can read or write.

## Important Commands
```
# Rust backend
cargo check
cargo build
cargo test
cargo run

# Frontend
cd dashboard
npm run build
```

Create branch: nightly-maintenance-$(date +%Y-%m-%d)-<rfc-slug> for changes (e.g. nightly-maintenance-2026-05-16-rfc024).

Start now.

## Prod gotchas (read before “fixing” dashboard/API)

- **Dashboard CORS + 502 on `contractgate-api.fly.dev` is often a crash-loop, not CORS.** Fly 502s have no `Access-Control-Allow-Origin`. Check `fly logs -a contractgate-api` for panics first.
- **2026-07-14 outage:** `jsonwebtoken` 10 without `rust_crypto` panicked on every Bearer JWT → SIGABRT → 502. Fix: keep `features = ["use_pem", "rust_crypto"]` in `Cargo.toml`. Full write-up: [`docs/reviews/incident-2026-07-14-jwt-crypto-provider.md`](docs/reviews/incident-2026-07-14-jwt-crypto-provider.md).
- **Merged to `main` ≠ prod fixed.** Confirm Fly release advanced (`fly releases -a contractgate-api`). Never run multiple concurrent `fly deploy`s.
- **`/health` 200 does not prove JWT auth works.**

## RFC Status Index

[`docs/STATUS.md`](docs/STATUS.md) — shipped-vs-draft view of all 58 RFCs.

<!-- code-review-graph MCP tools -->
## MCP Tools: code-review-graph (optional)

> **This section only applies if the `code-review-graph` MCP server is
> connected in your environment.** Check with `list_connected_mcps` or look for
> tools named `detect_changes`, `query_graph`, etc. in your tool list. If the
> MCP is **not** connected, use Grep/Glob/Read normally — skip this section.

When the `code-review-graph` MCP **is** connected, prefer it over file
scanning: the graph is faster, cheaper (fewer tokens), and surfaces structural
context (callers, dependents, test coverage) that grep cannot.

### When to use graph tools (if connected)

- **Exploring code**: `semantic_search_nodes` or `query_graph` instead of Grep
- **Understanding impact**: `get_impact_radius` instead of manually tracing imports
- **Code review**: `detect_changes` + `get_review_context` instead of reading entire files
- **Finding relationships**: `query_graph` with callers_of/callees_of/imports_of/tests_for
- **Architecture questions**: `get_architecture_overview` + `list_communities`

Fall back to Grep/Glob/Read when the graph is not connected or does not cover
what you need.

### Key Tools

| Tool | Use when |
|------|----------|
| `detect_changes` | Reviewing code changes — gives risk-scored analysis |
| `get_review_context` | Need source snippets for review — token-efficient |
| `get_impact_radius` | Understanding blast radius of a change |
| `get_affected_flows` | Finding which execution paths are impacted |
| `query_graph` | Tracing callers, callees, imports, tests, dependencies |
| `semantic_search_nodes` | Finding functions/classes by name or keyword |
| `get_architecture_overview` | Understanding high-level codebase structure |
| `refactor_tool` | Planning renames, finding dead code |

### Workflow (when connected)

1. The graph auto-updates on file changes (via hooks).
2. Use `detect_changes` for code review.
3. Use `get_affected_flows` to understand impact.
4. Use `query_graph` pattern="tests_for" to check coverage.
