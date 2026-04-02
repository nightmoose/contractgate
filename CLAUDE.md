# CLAUDE.md - Autonomous Code Guardian Instructions

**Project Name:** ContractGate  
**Description:** High-performance semantic contract enforcement gateway (Patent Pending). Validates incoming data events in real-time against a full semantic contract before they reach storage or AI systems.

**Current Status:** Basic Next.js frontend with Playground page exists. Now we must build the real Rust validation engine and API backend.

## Tech Stack
- **Core Validation Engine:** Rust (must be fast, <15ms p99 latency target)
- **API Framework:** Axum (Rust)
- **Frontend:** Existing Next.js 15 + TypeScript + Tailwind (do not overhaul unless necessary)
- **Database:** Supabase (for contracts, audit logs, quarantined events)
- **Deployment:** Rust service on Fly.io/Railway + Next.js on Vercel

## Priority Order (Strict)
1. Build robust Rust validation engine (this is the heart of the patent)
2. Create Axum API with /ingest/{contract_id} endpoint
3. Connect frontend Playground to call the real backend
4. Add contract CRUD + audit logging
5. Polish dashboard and audit log pages

## Semantic Contract Format (Lock This In)
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

## Rules for This Run
- Focus heavily on the Rust validation engine first
- Make validation fast, extensible, and well-commented
- Support reject + quarantine actions on violation
- Return clear, structured error messages with violation details
- Keep API simple but production-ready (proper error handling, JSON responses)
- Do not spend time beautifying the frontend yet — just make the Playground actually work with the backend

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

Start by creating a proper Rust project structure if not present (src/main.rs, lib for validation, etc.).

You are now building the core patented technology. Make the validation engine fast, correct, and clearly superior to post-ingestion tools.

Create branch: nightly-maintenance-$(date +%Y-%m-%d) for changes.

Start now.