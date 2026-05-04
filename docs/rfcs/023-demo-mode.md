# RFC-023: Demo Mode — Zero-Auth Local Experience

| Field         | Value                                            |
|---------------|--------------------------------------------------|
| Status        | **Draft — 2026-05-04**                           |
| Author        | ContractGate team                                |
| Created       | 2026-05-04                                       |
| Target branch | `nightly-maintenance-2026-05-04-demo-mode`       |
| Tracking      | Demo onboarding feedback (post-RFC-017)          |

---

## Summary

Ship a "self-hosted OSS edition" demo experience that lets a new user clone
the repo, run one command, and see contracts enforcing real traffic in under
15 minutes. No Supabase project, no API keys, no sign-up flow.

The mechanism is a single env var, `NEXT_PUBLIC_DEMO_MODE=1`, that the
dashboard checks at three chokepoints (middleware, AuthGate, OrgProvider) and
short-circuits the Supabase auth flow. The Rust gateway already runs without
auth when no `API_KEY` is configured — no backend change needed beyond
verifying the org_id contract between seeder and dashboard.

This is the OSS-edition pattern (see Airbyte, Sentry, PostHog): the demo is a
real binary with a feature subset, not a sandboxed mock. Multi-tenancy, SSO,
RLS, GitHub sync, and API-key management remain gated behind the
non-demo build path that RFC-001 / RFC-013 deliver.

---

## Goals

1. `git clone && make demo` boots a working stack with seeded contracts and
   live traffic in under 15 minutes (3 min first-build, then instant).
2. Dashboard at `http://localhost:3000` loads with no login wall, shows the
   three starter contracts, audit log, quarantine viewer, playground.
3. Seeded events visible end-to-end: dashboard contracts list ↔ gateway
   `/contracts` ↔ Postgres rows all share the same `org_id`.
4. Dashboard build does not require any Supabase URL or anon key to compile
   or run.
5. Production / paid build path unchanged — same Dockerfile, same
   middleware.ts file, gated only by the env var.
6. Banner in demo mode makes it obvious to the user this is the OSS edition
   and points at the upgrade path.

## Non-goals

- Local Supabase container. Supabase CLI works, but adds 4 services and
  ~600 MB of images to the demo footprint. Defer.
- Removing auth code. AuthGate / middleware / OrgProvider stay; they
  short-circuit, not delete. Keeps one build path.
- Per-user state in the demo (no real accounts → no per-user contracts).
  Single fixed org, single "owner" role.
- Fixing every "feature requires real auth" path (GitHub config, invites,
  member management). These show a "not available in demo mode" banner.
- Replacing the GHCR-published `dashboard` image flow. Demo builds locally
  so it works before CI publishes anything.

---

## Current state — what's broken

Three independent failures combine to wedge the new-user experience:

### 1. Dashboard middleware crashes without Supabase env vars

`dashboard/middleware.ts` calls `createServerClient` at the top of every
request with `process.env.NEXT_PUBLIC_SUPABASE_URL!` (non-null assertion).
With the placeholder `.env.local` value `https://your-project.supabase.co`,
the Supabase client throws on `auth.getUser()` and the request 500s before
any page renders.

The page-level `AuthGate` previews are therefore unreachable — the
middleware kills the request first.

### 2. `dashboard` image not published to GHCR

`docker-compose.yml` references `ghcr.io/contractgate/dashboard:${TAG:-latest}`
under the `ui` profile. CI does not publish this image (only `gateway`).
`docker compose --profile ui up` fails with manifest-not-found before the
container starts.

### 3. `NEXT_PUBLIC_API_URL` default points at port 3001

`dashboard/lib/api.ts` defaults `BASE` to `http://localhost:3001`. The
gateway in compose listens on `8080`. Without a populated `.env.local` the
dashboard talks to nothing.

### 4. Demo seeder org_id contract is fragile

`demo-seeder` posts `x-org-id: cccccccc-cccc-cccc-cccc-cccccccccccc` (the
fixed UUID seeded by `ops/postgres/seed/099_demo_org.sql`). The dashboard's
`OrgProvider` derives `org_id` from the Supabase session — which doesn't
exist in demo mode — so `lib/api.ts` sends no `x-org-id` and the contracts
list comes back empty even though they were created.

---

## Design

### Single switch: `NEXT_PUBLIC_DEMO_MODE`

A build-time env var (Next.js inlines `NEXT_PUBLIC_*` at build). When `"1"`,
the dashboard:

- **middleware.ts** — early-returns `NextResponse.next()` before constructing
  the Supabase client. No Supabase env vars referenced, so missing/placeholder
  values are harmless.
- **components/AuthGate.tsx** — renders `children` directly, skipping the
  preview/login illustration entirely.
- **lib/org.tsx** — `OrgProvider` returns the fixed demo UUID and a
  hardcoded `{ role: "owner", orgName: "Demo Org" }` synchronously. Skips
  the Supabase session fetch.
- **lib/api.ts** — calls `setApiOrgId(DEMO_ORG_UUID)` at module init when
  the flag is set, so every API call carries the right `x-org-id` header
  even before any provider mounts.
- **components/DemoBanner.tsx** (new) — small fixed banner: "Demo Mode •
  single tenant • [Learn about ContractGate Cloud →]". Mounted in
  `app/layout.tsx` conditionally.

Pages that depend on real auth (GitHub config, member invites, account
settings) get a guard at the top:

```tsx
if (process.env.NEXT_PUBLIC_DEMO_MODE === "1") {
  return <DemoFeatureUnavailable feature="GitHub sync" />;
}
```

### Compose: dashboard joins the demo profile

```yaml
dashboard:
  profiles: [ui, demo]
  build:
    context: ./dashboard
    args:
      NEXT_PUBLIC_DEMO_MODE: "1"
      NEXT_PUBLIC_API_URL: http://localhost:8080
      NEXT_PUBLIC_API_KEY: cg_demo_key
  ports: ["3000:3000"]
  depends_on:
    gateway:
      condition: service_healthy
```

Drop the `image:` line. Build from source. First build is ~2-3 min; layer
cache makes subsequent runs instant.

`Dockerfile` accepts the three build args, writes them to `.env.production`
before `next build` so they're inlined into the static bundle.

### `lib/api.ts` default fix

Change default `BASE` from `http://localhost:3001` to `http://localhost:8080`.
`3001` is a stale Dockerfile default that no compose path actually uses.
Independent fix; lands with this RFC but not gated on the flag.

### Org-id contract pinned in one place

New `dashboard/lib/demo.ts`:

```ts
export const DEMO_ORG_UUID = "cccccccc-cccc-cccc-cccc-cccccccccccc";
export const DEMO_MODE = process.env.NEXT_PUBLIC_DEMO_MODE === "1";
```

`lib/org.tsx`, `lib/api.ts`, the seeder defaults in `docker-compose.yml`,
and `ops/postgres/seed/099_demo_org.sql` all reference this UUID. A
compose-smoke step asserts the dashboard's `/contracts` API call returns
≥3 contracts — catches future drift.

### Makefile target

```make
.PHONY: demo demo-down demo-logs
demo:
	docker compose --profile demo --profile ui up --build
demo-down:
	docker compose --profile demo --profile ui down -v
demo-logs:
	docker compose --profile demo --profile ui logs -f
```

`make demo` is the one command the README quickstart points at.

---

## User experience

After `git clone && make demo`:

| URL                     | What user sees                                        |
|-------------------------|-------------------------------------------------------|
| `localhost:3000`        | Dashboard. No login. Demo banner top-right.           |
| `localhost:3000/contracts` | 3 starter contracts (user_events, orders, page_views). YAML editor works. Playground works. |
| `localhost:3000/audit`  | Live event stream. ~3000 events/5 min. Pass/fail/quarantine breakdown. |
| `localhost:3000/contracts` → quarantine tab | Quarantined rows with field-level violation reasons. |
| `localhost:3000/stream-demo` | Real-time throughput viz. Already auth-free today.   |
| `localhost:8080/health` | `{"status":"ok"}`                                     |
| `localhost:3002`        | Grafana (admin/admin). Pre-imported dashboard.        |

Pages that show "not available in demo mode" with upgrade CTA: GitHub sync,
invite members, API key management, account settings.

---

## Phased rollout

1. **Phase 1 — Unblock (this RFC):** middleware + AuthGate + OrgProvider
   short-circuit. Compose dashboard build. Makefile target. README
   quickstart. Compose-smoke assertion. Land in
   `nightly-maintenance-2026-05-04-demo-mode`.
2. **Phase 2 — Polish (follow-up):** demo banner with upgrade CTA.
   `DemoFeatureUnavailable` component for gated pages. Screenshot-test
   the demo flow in CI.
3. **Phase 3 — OSS positioning (separate RFC):** README OSS-vs-Cloud
   feature matrix, `/pricing` page update, license review for the OSS
   subset.

---

## Risks

- **Risk: demo path drifts from prod path.** Mitigation: same source files,
  one env var. CI runs both `compose-smoke` (auth on) and `compose-demo-smoke`
  (auth off). If demo bypass ever lets a prod build through, the
  prod-smoke catches it.
- **Risk: org_id mismatch returns silently.** Mitigation: compose-smoke
  asserts `GET /contracts` returns ≥3 entries with the demo UUID. Pinned
  constant in `lib/demo.ts` is the single source of truth.
- **Risk: writes from demo dashboard pollute audit_log with `contract_version`
  default values.** Mitigation: gateway writes the matched version regardless
  of auth path (per memory: feedback_audit_honesty). Verify in test.
- **Risk: users assume demo is production-ready.** Mitigation: persistent
  banner. README "OSS Edition" framing. Cloud upgrade CTA on every gated
  page.
- **Risk: 2-3 min first-build pushes past 15-min target on slow networks.**
  Mitigation: publish dashboard image to GHCR as a follow-up so `make demo`
  can pull instead of build. Out of scope for this RFC.

## Testing

- `cargo test` — unchanged; no Rust changes.
- `cd dashboard && npm run build` with `NEXT_PUBLIC_DEMO_MODE=1` set — must
  succeed without any `NEXT_PUBLIC_SUPABASE_*` env vars present.
- `cd dashboard && npm run build` without the flag — must still work
  (prod path unchanged).
- New compose-smoke step: after `--profile demo --profile ui up`, curl
  `http://localhost:3000/api/...` (or driver Playwright headlessly), assert
  contracts list non-empty.
- Playwright `dashboard/e2e/demo.spec.ts` — covers: page loads, contracts
  visible, audit shows events, playground validates a sample.

## Open questions

1. Banner copy — "Demo Mode" vs "OSS Edition" vs "Self-Hosted Free"?
   Naming sets pricing-page expectations. Decide before Phase 2.
2. Should `/playground` work in demo mode without persisting state, or
   should it persist to the demo org? Lean: persist — it's the same
   org_id flow as everything else.
3. Do we need a `make demo-reset` that wipes `pg_data` and re-seeds?
   `docker compose down -v` works but is undiscoverable.
4. Is the seeder's 5-min duration the right default for demo, or should
   demo-mode seeder run forever at low rate so the dashboard is always
   "live"?

---

## Acceptance criteria

- [ ] `make demo` boots full stack on a clean clone with no env setup.
- [ ] Dashboard loads at `localhost:3000` with zero Supabase env vars set.
- [ ] Contracts page shows ≥3 contracts within 30s of seeder start.
- [ ] Audit page shows pass/fail/quarantine counts climbing live.
- [ ] Playground validates a sample event end-to-end.
- [ ] `make demo-down` cleans up volumes; rerun is idempotent.
- [ ] README "Try it in 10 minutes" section points at `make demo`.
- [ ] CI runs compose-demo-smoke and asserts dashboard contract list.
- [ ] Prod build (`NEXT_PUBLIC_DEMO_MODE` unset) is byte-identical to today.
