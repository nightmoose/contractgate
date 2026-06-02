# CLAUDE.md - Autonomous Code Guardian Instructions

**Project Name:** ContractGate  
**Description:** A high-performance semantic contract enforcement gateway that validates incoming data events in real time against a full semantic contract (ontology + business glossary + metric definitions) before the data reaches any storage or AI system. Marketed as "Patent Pending".

**Core Goal:** Prevent garbage-in-garbage-out at ingestion time with sub-15ms latency. Make it simple enough for small data teams to adopt quickly while being powerful enough to demonstrate the patented idea.

## Tech Stack (MVP)
- **Validation Engine:** Rust (primary — for speed and low latency)
- **API Layer:** Axum (Rust web framework) or thin wrapper
- **Frontend Dashboard:** Next.js 15 + TypeScript + Tailwind CSS
- **Database:** Supabase (contracts, audit logs, quarantined events)
- **Deployment:** Rust service on Fly.io/Railway, Next.js on Vercel

## MVP Features (Strict Scope)
1. Contract Management
   - Create/edit versioned contracts in clean YAML format (ontology, glossary, metrics)
   - Store contracts in Supabase

2. Ingestion API
   - POST /ingest/{contract_id} — accepts single JSON event or batch
   - Real-time validation against the semantic contract
   - On success: forward to configurable destination (webhook, Supabase, Kafka topic stub, etc.)
   - On violation: reject with clear error + log to audit trail + optional quarantine

3. Dashboard (Next.js)
   - List of contracts
   - Live ingestion monitor (throughput, pass rate, violations)
   - Searchable audit log
   - Simple test ingestion playground
   - Prominent "Patent Pending" badge

## Performance Target
- < 15ms p99 latency for validation on modest hardware
- Capable of handling thousands of events per second

## Coding Rules
- Rust core must be fast, safe, and well-commented for the validation logic
- Keep contract YAML format simple and human-readable
- Use strong typing and proper error handling everywhere
- Frontend must be clean, modern, and professional (dark theme preferred)
- Always include clear violation reasons in responses

## Important Commands
[CODE_BLOCK_START]
# Rust
cargo check
cargo build
cargo test
cargo run

# Frontend
cd dashboard
npm install
npm run dev
npm run build

# Database
npx prisma generate   # if using Prisma for dashboard
[CODE_BLOCK_END]

## Rules for Autonomous Nightly Runs
- Priority 1: Fix all build, test, or compilation errors
- Priority 2: Improve performance and reliability of the validation engine
- Priority 3: Add or polish MVP features (contract CRUD, better error messages, dashboard improvements, audit logging)
- Never add features outside the MVP scope (no complex ML remediation, no full ontology reasoner, no enterprise multi-tenancy yet)
- Create branch: nightly-maintenance-$(date +%Y-%m-%d)
- Use small, focused commits with prefix "nightly: "

## Non-Goals for Now
- Do not add native Kafka/Flink connectors yet
- Do not implement heavy ML auto-remediation
- Do not over-engineer the contract language

You are the senior Rust + Full-stack engineer for ContractGate. Keep the system fast, simple, and demonstrably valuable for data teams fighting dirty data at ingestion.

Start every run by reading this file and running cargo check + npm run build in the dashboard.
