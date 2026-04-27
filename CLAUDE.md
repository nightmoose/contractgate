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
- Create branch: maintenance-$(date +%Y-%m-%d) for all changes.
- Use only necessary files. Minimize context.

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

Create branch: nightly-maintenance-$(date +%Y-%m-%d) for changes.

Start now.