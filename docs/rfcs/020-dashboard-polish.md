# RFC-020: Dashboard Polish

| Status        | Accepted (2026-04-28)                                 |
|---------------|-------------------------------------------------------|
| Author        | ContractGate team                                     |
| Created       | 2026-04-28                                            |
| Target branch | `nightly-maintenance-2026-04-28`                      |
| Chunk         | Pre-customer pilot polish (supports v2 punchlist)     |
| Depends-on    | RFC-002 (versioning), RFC-003 (replay), RFC-006 (diff)|

---

## Summary

Four coordinated UI workstreams that bring the dashboard to a state
suitable for first-pilot demos: Quarantine Tab search/filter/replay
hardening, a richer Versions Tab with a state ladder and diff comparison,
polished Replay outcome rendering, and a single-pass tooltip layer that
glosses every piece of jargon a first-time operator would not understand.

No new backend routes. No new state-management libraries. No chart
libraries. Existing `useState` + SWR pattern preserved throughout.

---

## Goals

1. Quarantine Tab: filters (contract, kind, time window, free-text), per-row
   payload preview drawer, per-row inline Replay button with confirmation +
   retention guard, per-event replay history sub-panel, RFC-017 demo-seeder
   empty-state, bulk select + bulk replay with confirmation.
2. Versions Tab: visual state ladder (draft → stable → deprecated), proper
   React modal for promote/deprecate (replaces `window.confirm`), Compare
   button → diff drawer calling `POST /contracts/diff`, latest-stable
   resolver badge/chip.
3. Replay UI: four distinct outcome colors + icons (pass / fail /
   re-quarantined / skipped/already_replayed), batch replay summary modal,
   inline diff between original and post-transform payload.
4. Tooltip layer: single Radix UI tooltip primitive added once; applied to
   every jargon term listed in §Decisions D4.

---

## Non-goals

- Restyle the audit page or any tab not in scope.
- New state-management library (Zustand, Jotai, etc.).
- Real-time subscription / websocket for quarantine. Polling + manual
  refresh (30 s SWR `refreshInterval`, already present) is sufficient.
- Chart library — text + tables + tooltips only.
- i18n — all copy hard-coded English in v1.
- Keyboard shortcuts — none in v1 (revisit if pilot feedback requests them).
- Backend changes. All four workstreams are purely dashboard-side.
- Severity column in the diff table — placeholder column header only;
  values deferred to RFC-015.

---

## Decisions

| # | Question | Recommendation | Rationale |
|---|----------|---------------|-----------|
| D1 | Split page.tsx (1 860 lines) or polish in place? | **Split** into `app/contracts/_tabs/{yaml.tsx, versions.tsx, quarantine.tsx}` + shared `app/contracts/_lib.ts` for types/helpers. Keep `app/contracts/page.tsx` as the orchestrating shell only. | 1 860 lines is well past the readable threshold. Each tab has distinct state; splitting eliminates cross-tab state entanglement. The `_lib.ts` approach avoids a circular import. |
| D2 | Tooltip library: Radix UI, Floating UI, or hand-rolled? | **`@radix-ui/react-tooltip`** (new direct dep; not currently installed). | Radix is accessible by default (ARIA roles, keyboard dismiss), has zero styling opinions (fits the existing dark Tailwind theme), and is the industry convention for shadcn-adjacent stacks. Floating UI would require more wiring for the same result. Hand-rolled misses focus/keyboard semantics. |
| D3 | Modal vs drawer for version diff comparison? | **Drawer** (right-side panel, same pattern as `RawEventDrawer` in audit/page.tsx). | The diff table can have 20+ rows; a centred modal would overflow or need its own scroll. The drawer pattern is already established in the codebase — visual consistency for free. |
| D4 | Tooltip terms to cover | See full list in §Design / Tooltip layer. One sentence per term, plain English, no acronyms. RFC link appended where one exists. | Pilot operators will read the YAML tab for the first time; every labelled section is jargon. |
| D5 | Bulk-replay concurrency: any dashboard-level cap beyond the server's 1 000 row cap? | **No additional cap.** Server enforces 1 000 via `ReplayRequest::validate_bounds`. Bulk action fires a single `POST /contracts/:id/quarantine/replay`. UI selection is limited to what is visible on the page (≤ 100 rows per page). | Avoids double-guarding the same invariant in two places. The 100-row UI page size is the effective cap for a single click. |
| D6 | Replay confirmation: inline vs modal? | **Modal** for both per-row and bulk replay. Replace the current action-bar inline "▶ Replay" button flow (no confirmation) with a `ConfirmReplayModal` component. | RFC-003 §Design prescribes "Confirm dialog before replay: Replay N events against version X. Passes will be inserted into audit_log and forwarded." The existing code fires without any confirmation. A modal forces a deliberate second click; this is a write operation. |
| D7 | Promote/Deprecate confirmation: convert from `window.confirm`? | **Yes — replace with `ConfirmActionModal`** (reusable single component, parameterised by title + body + destructive flag). `window.confirm` is non-styleable and blocked in some browser extensions. | Consistency: all destructive confirmations should look the same. The same component handles promote and deprecate. |
| D8 | Where does the "Compare" checkbox live? | Per-row checkbox in the Versions tab table (separate column from any existing action). Two rows checked → "Compare selected (2)" button enables. Attempting to check a third row replaces the oldest selection. | Keeps the affordance close to the data. Max-2 enforcement is simple to express: if set grows beyond 2, drop the first. |
| D9 | Diff drawer: how to fetch YAML for comparison? | The Versions tab already holds `versions: VersionSummary[]` and calls `getVersion` on demand. When Compare fires, call `getVersion` for both checked rows in parallel, then POST their `yaml_content` fields to `POST /contracts/diff`. | No new API surface needed. Two sequential `getVersion` calls that already cache in local state. |
| D10 | `QuarantinedEvent.status` field: add to TS type? | **Yes.** Extend the `QuarantinedEvent` interface in `lib/api.ts` to include `status: "pending" \| "reviewed" \| "replayed" \| "purged"`. The per-row Replay button is disabled when `status === "purged"` (past retention). The server presumably returns this field; the TS interface was never extended to include it. | Per RFC-003, `purged` rows should be visually distinct and not replayable from the UI. |
| D11 | Kind filter: server-side query param or client-side? | **Client-side only** in v1. Filter events already loaded by checking whether any entry in `violation_details` has a matching `kind`. | Server does not yet expose a `kind` query param on `GET /quarantine`. Adding a client-side filter avoids a backend change and is fast enough for the 100-row page. |
| D12 | Inline payload diff (original vs post-transform) in replay history: where does post-transform come from? | From `ReplayOutcome` — if the server adds `transformed_event` to the replay outcome row (currently absent from the TS type), surface it. If not present, render a "no transform data" placeholder. Do not block the workstream on a backend change. | RFC-003 does not currently include `transformed_event` in `ReplayOutcome`. We add a placeholder slot and leave it as a deferred enhancement. |

---

## Design

### A. File layout after split

```
dashboard/app/contracts/
├── page.tsx                   # shell: tab state, modal state, contracts SWR
├── _lib.ts                    # shared helpers: pickDefaultVersion,
│                              #   newestVersionString, inferFields, buildYaml,
│                              #   PATTERNS, ConfirmActionModal, TooltipWrap
├── _tabs/
│   ├── yaml.tsx               # YAML tab (EditContractModal body slice)
│   ├── versions.tsx           # Versions tab + ConfirmReplayModal
│   └── quarantine.tsx         # QuarantineTab (moved from page.tsx)
├── VisualBuilder.tsx          # unchanged
└── examples.ts                # unchanged
```

`EditContractModal` stays in `page.tsx` as the outer shell (handles
backdrop, header, ingest strip, tab bar, footer). The tab bodies are
imported from `_tabs/`.

### B. Quarantine Tab (`_tabs/quarantine.tsx`)

**Additions over current code:**

1. **Search + filter bar** — four controls in a flex row:
   - Contract select (exists, keep)
   - Kind multi-select: `All`, `validation`, `parse`, `transform`. Filters
     client-side on `ev.violation_details.some(v => v.kind === kind)`.
   - Time window: `Last 1h / 6h / 24h / 7d / All`. Filters client-side on
     `ev.quarantined_at >= now - window`.
   - Free-text: `<input>` that filters on
     `JSON.stringify(ev.raw_event).toLowerCase().includes(term)`.
   All four filters compose with AND. Applied after the SWR fetch.

2. **`status` column** — add a "Status" column to the table showing a
   small badge: `pending` (slate), `reviewed` (indigo), `replayed` (green),
   `purged` (red-line-through). Extend `QuarantinedEvent` type (D10).

3. **Per-row payload preview drawer** — clicking the row body (not
   checkbox or History button) opens a read-only right-panel drawer showing
   `JSON.stringify(ev.raw_event, null, 2)` with a copy button. Follows the
   exact same drawer shell as `RawEventDrawer` in audit/page.tsx.

4. **Per-row inline Replay button** — small "▶" button in a new "Actions"
   column. Disabled when `ev.status === "purged"` (with a tooltip:
   "Event purged — past retention window"). Clicking opens
   `ConfirmReplayModal` scoped to that single event.

5. **`ConfirmReplayModal`** — shared component (in `_lib.ts`) parameterised
   by `count`, `version`, `onConfirm`, `onCancel`. Body text: "Replay
   {count} event{s} against v{version}. Passes will be inserted into
   audit_log and forwarded downstream. Original quarantine rows are
   preserved." Two buttons: "Confirm Replay" (indigo) and "Cancel" (slate).

6. **Replay action bar confirmation** — existing bulk "▶ Replay" button
   now opens `ConfirmReplayModal` instead of firing immediately.

7. **Empty-state** — update the empty-state copy:
   ```
   No quarantined events{for this contract}.
   Events land here when the backend quarantines on a validation,
   parse, or transform violation.
   To generate sample data: run `make stack-up-demo`
   (RFC-017 onboarding stack).
   ```

8. **Replay history sub-panel** — keep existing drawer but surface `status`
   of each history row. Source row status badge displayed at the top.

### C. Versions Tab (`_tabs/versions.tsx`)

**Additions over current code:**

1. **Visual state ladder** — above the version list, a small three-step
   inline diagram:
   ```
   [Draft] ──promote──▶ [Stable] ──deprecate──▶ [Deprecated]
   ```
   Rendered as three pill badges connected by arrows using SVG or pure CSS.
   Each pill is the same color as the existing state badge. Clicking a pill
   triggers the Radix tooltip explaining that state.

2. **`ConfirmActionModal` replace `window.confirm`** — all four confirm
   calls (`handlePromote`, `handleDeprecate`, `handlePromoteVersion`,
   `handleDeprecateVersion`) replaced with `ConfirmActionModal`. The modal
   title and body text are the same strings currently passed to
   `window.confirm`. `destructive={true}` for deprecate; `destructive={false}`
   for promote.

3. **Compare checkbox + diff drawer:**
   - Add a checkbox column to the versions table (leftmost).
   - A `compareSet: Set<string>` (useState) holds at most 2 version strings.
     Adding a third drops the oldest (`compareSet` is maintained as an
     ordered pair).
   - When `compareSet.size === 2`, a "Compare selected (2)" button appears
     above the table.
   - Clicking it: `setDiffLoading(true)`, call `getVersion` for both in
     parallel, then POST to `/contracts/diff`. Opens a `DiffDrawer` on the right.
   - `DiffDrawer` body: `summary` string at top; a three-column table:
     `Kind | Field | Detail | Severity (placeholder —)`. Rows colored by
     kind: added = green-tinted, removed = red-tinted, changed = amber-tinted.
   - Wire the existing `POST /contracts/diff` endpoint which returns
     `{ summary: string, changes: [{kind, field, detail}] }`.

4. **Latest-stable resolver badge** — in the Versions tab section header,
   add a small chip: "Routing to: v{latest_stable} (strict)" or "Routing
   to: v{latest_stable} (fallback)". Sourced from `contract.latest_stable_version`
   and `contract.multi_stable_resolution`. When no stable exists, show
   "No stable version — traffic will 409". Tooltip explains the resolution
   logic.

5. **Name history de-emphasis** — when `nameHistory.length === 0`, the
   entire section collapses to a single muted line: "Contract has always
   been named {name}." No separate section header. When non-empty, the
   existing table is shown with reduced opacity (`text-slate-500` instead of
   `text-slate-400`).

### D. Replay UI (`_tabs/quarantine.tsx` + `_lib.ts`)

**Outcome rendering — four distinct states:**

| Outcome | Color | Icon | Description |
|---------|-------|------|-------------|
| `pass` / `replayed` | `text-green-400` / `bg-green-900/20` | ✅ PASSED | Event accepted; written to audit_log |
| `fail` / `still_quarantined` | `text-red-400` / `bg-red-900/20` | ❌ FAILED | Event still violates; new quarantine row written |
| `already_replayed` | `text-indigo-400` / `bg-indigo-900/20` | ↩ ALREADY REPLAYED | Source row was already in `replayed` state; no-op |
| `purged` / `skipped` | `text-slate-500` / `bg-slate-800/20` | ⊘ SKIPPED | Row purged or otherwise unreachable |

Colors match or complement the audit tab's pass/fail coloring exactly
(audit: `bg-green-900/40 text-green-400` / `bg-red-900/40 text-red-400`).

**Batch replay summary modal** — replace the inline result card with a
`ReplaySummaryModal` that appears after replay completes:
- Header: "Replay complete"
- Counts: total attempted / passed / failed / already replayed / skipped
- Target version badge
- "View in Audit Log →" link that navigates to `/audit?contract_id=X`
- Dismiss button

**Inline original vs post-transform diff** — in the per-event replay
history drawer, if `ReplayOutcome` carries a `transformed_event` field,
render a side-by-side (or stacked on mobile) JSON diff: left = `raw_event`
from the source quarantine row, right = `transformed_event` from the
replay outcome. If `transformed_event` is absent, show a grey placeholder:
"Transform diff not available — server did not return post-transform
payload."

### E. Tooltip layer (`_lib.ts` + everywhere)

**Install:** `npm install @radix-ui/react-tooltip` (adds ~8 kB gzipped).

**Primitive** — a single `TooltipWrap` wrapper component in `_lib.ts`:

```tsx
import * as Tooltip from "@radix-ui/react-tooltip";

export function TooltipWrap({
  children,
  content,
  rfc,
}: {
  children: React.ReactNode;
  content: string;
  rfc?: string; // e.g. "RFC-002"
}) {
  return (
    <Tooltip.Provider delayDuration={300}>
      <Tooltip.Root>
        <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
        <Tooltip.Portal>
          <Tooltip.Content
            className="max-w-xs text-xs bg-[#1f2937] text-slate-200 rounded-lg px-3 py-2 shadow-xl border border-[#374151] z-[200]"
            sideOffset={4}
          >
            {content}
            {rfc && (
              <span className="ml-1 text-indigo-400 underline underline-offset-2 cursor-default">
                {rfc}
              </span>
            )}
            <Tooltip.Arrow className="fill-[#1f2937]" />
          </Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip.Root>
    </Tooltip.Provider>
  );
}
```

**Terms and copy** (applied wherever the label appears in the UI):

| Term | Tooltip text | RFC link |
|------|-------------|----------|
| Ontology | "The named entities and field rules your contract enforces — every inbound event is validated against these definitions." | — |
| Glossary | "Human-readable descriptions of fields, including any compliance constraints attached to each one." | — |
| Metrics | "Named aggregate formulas (e.g. sum, count) computed over events that pass this contract." | — |
| Stable | "A frozen, immutable version eligible to receive inbound traffic. YAML cannot be edited after promotion." | RFC-002 |
| Draft | "A work-in-progress version. YAML is freely editable. Promotes to Stable when ready." | RFC-002 |
| Deprecated | "A retired version. No new unpinned traffic routes to it. Clients that explicitly pin this version get their batch quarantined." | RFC-002 |
| Quarantine | "Events that failed contract validation are held here for inspection and optional replay. Nothing is silently dropped." | RFC-003 |
| Replay | "Re-validate a quarantined event against a current contract version. If it passes, it is written to the audit log and forwarded downstream." | RFC-003 |
| Retention | "How long quarantined events are kept before being purged. Once purged, replay is no longer possible." | — |
| mask | "Replaces the field value with a fixed placeholder (e.g. ****). The original value is never stored." | RFC-004 |
| hash | "Replaces the field value with a deterministic HMAC-SHA256 digest using the contract's per-contract salt." | RFC-004 |
| drop | "Removes the field from the stored event entirely — as if it was never sent." | RFC-004 |
| redact | "Replaces the field value with the literal string [REDACTED]." | RFC-004 |
| format_preserving | "Masks the value while preserving its structure (e.g. a credit card stays 16 digits, just with most digits replaced)." | RFC-004 |
| Salt | "A 32-byte secret tied to this contract used when hashing PII fields. Changing it invalidates all prior hashes." | RFC-004 |
| Compliance mode | "When enabled, any inbound field not declared in the contract ontology is rejected. Nothing undeclared can enter the audit log." | RFC-004 |
| PASS | "This event satisfied every rule in the contract and was written to the audit log." | — |
| FAIL | "This event violated at least one contract rule and was quarantined." | — |
| fallback | "On unpinned traffic, if the latest stable version rejects an event, the gateway tries other stable versions in order until one accepts." | RFC-002 |
| strict | "Unpinned traffic validates against only the single latest stable version. No retry on failure. This is the default." | RFC-002 |

---

## Test plan

Playwright component-level tests (`dashboard/e2e/` or `dashboard/tests/`):

1. **Quarantine search filters** — seed mock SWR data with events across
   two contracts, two kinds, two time windows; assert that applying each
   filter reduces the visible row count correctly.
2. **Version promote modal confirms** — click Promote on a draft row;
   assert `ConfirmActionModal` appears with correct body text; click Cancel
   asserts API is not called; click Confirm asserts `promoteVersion` was
   called.
3. **Replay outcome colors match audit** — render `ReplayHistoryDrawer`
   with one `passed: true` and one `passed: false` outcome; assert the
   green/red class names match the exact strings used in `audit/page.tsx`
   (`bg-green-900/40 text-green-400` / `bg-red-900/40 text-red-400`).
4. **Every tooltip mounts without layout shift** — render the Versions tab
   and Quarantine tab; hover each `TooltipWrap` trigger; assert tooltip
   portal appears and that the document body height does not change on
   mount (no layout shift from the portal insertion).
5. **Compare button** — check two version rows; assert "Compare selected (2)"
   appears; check a third row; assert only 2 remain checked; click Compare;
   assert `DiffDrawer` opens with the `summary` string visible.
6. **Bulk replay confirmation** — select 3 events; click "▶ Replay";
   assert `ConfirmReplayModal` appears before any API call fires.

---

## Rollout

All steps on branch `nightly-maintenance-<date>`:

- [x] 1. Install `@radix-ui/react-tooltip`; added to package.json — `npm install` required.
- [x] 2. Create `_lib.tsx` — helpers + `TooltipWrap` + `ConfirmActionModal` + `ConfirmReplayModal` + `ReplaySummaryModal`.
- [x] 3. Split page.tsx → `_tabs/{yaml,versions,quarantine}.tsx` + `_lib.tsx`; page.tsx reduced to ~420 lines.
- [x] 4. Quarantine Tab: kind/time/text filters + status column + payload drawer + per-row Replay button + `ConfirmReplayModal` + RFC-017 empty-state.
- [x] 5. Versions Tab: state ladder + `ConfirmActionModal` + Compare checkbox + `DiffDrawer` + latest-stable badge + name-history de-emphasis.
- [x] 6. Replay UI: four outcome colors + `ReplaySummaryModal` + transform diff placeholder.
- [x] 7. Tooltip layer: `TooltipWrap` applied to all jargon terms listed in §Design E.
- [x] 8. Playwright tests for all 6 test-plan items; `playwright.config.ts` created.
- [ ] 9. `cd dashboard && npm install && npm run build` — zero TS errors, zero ESLint errors. ⚠️ PENDING — bash workspace unavailable; Alex to run.
- [x] 10. Append `MAINTENANCE_LOG.md` entry.
- [x] 11. Update this RFC: steps marked complete.

---

## Deferred

- Severity column in diff table (placeholder only; values come with RFC-015
  breaking-change taxonomy).
- `transformed_event` in `ReplayOutcome` response — inline payload diff
  placeholder only; actual data requires a Rust change in `src/replay.rs`.
- Server-side `kind` query param on `GET /quarantine` — client-side filter
  in v1; server-side filtering is a follow-up when audit volume warrants it.
- Keyboard shortcuts — none in v1.
- i18n — hard-coded English throughout; revisit at first non-English pilot.
- Auto-refresh indicator (spinner/timestamp showing last poll) — nice-to-have,
  deferred.
- Pagination controls on quarantine list — 100-row limit per fetch is
  sufficient for pilot; proper pagination deferred until first customer has
  sustained quarantine volume.
