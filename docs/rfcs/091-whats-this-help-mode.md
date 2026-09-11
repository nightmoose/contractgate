# RFC-091 — Help Catalog + “What’s this?” Inspect Mode

**Status:** Accepted
**Date:** 2026-09-11
**Branch:** `nightly-maintenance-2026-09-11-rfc091-whats-this-help-mode`
**Depends on:** RFC-020 (TooltipWrap + jargon glossary)

---

## Problem

The dashboard is a jargon-dense operator console (ontology, quarantine, replay,
scorecard, workbench, scaffold, egress). First-time and returning users have no
in-app way to ask “what is this control and what does it do?”

RFC-020 added Radix `TooltipWrap` and a glossary of ~20 terms, but coverage
stopped at Contracts → YAML / Versions / Quarantine. Catalog, Scorecard,
Scaffold, Workbench, Playground, Audit, Account, and the sidebar have almost
none. Native `title=` attributes are scattered and inaccessible.

A blocking product tour cannot cover nine nav items plus six contract tabs.
Hover-only tooltips are invisible until you already know to hover, and too
small for “what it is / what it does / when to use it.”

---

## Goal

An opt-in **What’s this?** inspect mode: toggle it, highlighted controls get a
dashed outline, click one, get a card. Hover tooltips stay for short jargon.
Empty Contracts / Dashboard / Audit get a three-step first-run, not a modal
tour.

Done means:

1. One help catalog is the source of copy.
2. Sidebar **What’s this?** button and `?` key toggle inspect mode; Esc exits.
3. Registered controls (`data-help`) highlight in mode; click opens the card
   instead of firing the control.
4. Unregistered clicks say “No description for this yet.”
5. Existing RFC-020 hover tooltips still work with mode off.
6. Empty Contracts / Dashboard / Audit explain the create → ingest →
   pass/quarantine loop.

---

## Non-goals

- A blocking “Welcome to ContractGate” spotlight tour (Pendo / Shepherd / Appcues).
- Literally documenting every DOM node. The promise is “click a highlighted
  control,” not “click anything.”
- A third-party product-tour SDK.
- `?` icons next to every label.
- i18n. English copy, same as RFC-020.
- Backend / engine changes.
- Persisting inspect mode across reloads (it is session-only, off by default).

---

## Design

### A. Help catalog

`dashboard/lib/help/catalog.ts` — a typed map keyed by stable ids
(`nav.contracts`, `term.quarantine`, `page.scorecard`, …).

Each entry:

| Field | Required | Role |
|---|---|---|
| `title` | yes | Card heading |
| `what` | yes | One sentence: what it is. Also the default hover tooltip. |
| `does` | yes | One sentence: what it does. |
| `when` | no | When to use it. |
| `href` | no | “Learn more” — in-app route or GitHub reference doc. |
| `hover` | no | Shorter hover string when `what` is too long. |

RFC-020 §Design E term table is the first batch of `term.*` entries. New UI
adds an entry the same way it adds a label.

### B. Inspect mode

`HelpProvider` wraps the dashboard in `layout.tsx`.

- **Toggle:** sidebar footer “What’s this?” (`aria-pressed`) and a floating `?`
  button on small screens (`md:hidden`).
- **Keyboard:** `?` toggles (ignored while typing in input / textarea / select /
  contenteditable). `Esc` closes the card if open, otherwise exits mode.
- **Cursor / highlight:** `html[data-help-mode="on"]` + CSS outline on
  `[data-help]`. Body cursor is `help`.
- **Click capture (mode on):** clicks on `[data-help-ui]` (card, toggle, banner)
  pass through. Other clicks are intercepted. A `[data-help]` ancestor opens
  that entry; otherwise a short “No description for this yet” miss toast at
  the click point.
- **Card:** portal near the target — title, what, does, optional when, optional
  Learn more. Close button. `role="dialog"`.
- Mode is **not** written to `localStorage`.

### C. `HelpTarget`

Wraps a single React element, clones `data-help={id}` onto it. With mode off
and a catalog entry present, wraps in `TooltipWrap` using `hover ?? what`.
Dynamic tooltips that are not catalog terms (ODCS score breakdown, “purged —
past retention”) stay on `TooltipWrap`.

`TooltipWrap` moves to `dashboard/components/help/TooltipWrap.tsx`. A single
`Tooltip.Provider` lives in `HelpProvider`. `contracts/_lib.tsx` re-exports
`TooltipWrap` so RFC-020 call sites keep compiling.

### D. Registration (this RFC)

| Surface | Ids |
|---|---|
| Sidebar nav | `nav.*` for every PUBLIC_NAV and ACCOUNT_NAV item |
| Page headers | `page.*` on each signed-in page title + Stream Demo, Docs, Pricing |
| Dashboard stats | `dashboard.total-events`, `pass-rate`, `violations`, `avg-latency`, `public-contracts` |
| Contract tabs / actions | list, consumed, visual-builder, generate, csv, quarantine, new, import-odcs, import-ref |
| RFC-020 jargon | `term.ontology` … `term.strict` (existing copy, now catalog-backed) |
| Catalog / Scorecard / Scaffold / Workbench / Playground / Audit / Account | page header plus the densest controls (egress, drift, seed, validate, stored-payload, api-keys) |

Unregistered controls are valid; they miss-toast in inspect mode.

### E. First-run empty states

Not a tour. Copy only, when the page is actually empty:

1. **Contracts list** — three steps: create → promote stable → POST ingest
   (pass → Audit, fail → Quarantine).
2. **Dashboard** — empty contracts and empty recent-events point at that loop.
3. **Audit** — keep the curl snippet; add the same three-step line above it.

Hero banner dismiss persists in `localStorage` (`cg.hero.dismissed`) so it does
not return every refresh.

---

## File layout

```
dashboard/lib/help/catalog.ts
dashboard/components/help/
  HelpProvider.tsx      # context, keyboard, click capture, miss toast, banner
  HelpTarget.tsx
  HelpToggle.tsx
  HelpCard.tsx
  TooltipWrap.tsx
  FirstRun.tsx
dashboard/app/layout.tsx                 # HelpProvider wrap
dashboard/components/Sidebar.tsx         # toggle + data-help on nav
docs/help-mode-reference.md
docs/rfcs/091-whats-this-help-mode.md
```

No new npm dependency. `@radix-ui/react-tooltip` is already installed.

---

## Decisions

| # | Question | Choice | Rationale |
|---|---|---|---|
| D1 | Tour vs inspect mode vs hover-only | Inspect mode + short hover for jargon + empty-state first-run | Tours go stale and get skipped. Hover is too small and undiscoverable. Inspect is opt-in and scales. |
| D2 | Persist mode on? | No | Users would forget it was on and think the app was broken. |
| D3 | Click-anything vs registered targets | Registered `[data-help]` + honest miss toast | Wrapping every pixel is unmaintainable. |
| D4 | Tooltip library | Keep Radix | Already shipped in RFC-020; accessible; no new dep. |
| D5 | Copy source of truth | One catalog file | Stops RFC-020’s inline-string drift. |
| D6 | Docs links | Optional `href` on the card, not on hover | Hover stays one sentence. |

---

## Testing

Playwright (`dashboard/e2e/rfc091-help-mode.spec.ts`), public pages so auth is
not required:

1. `/pricing` (or `/docs`): sidebar “What’s this?” sets `html[data-help-mode="on"]`
   and `[data-help]` nodes outline.
2. Click a highlighted sidebar item: help card opens with title + what + does;
   URL does not change.
3. Click a non-`[data-help]` area: miss toast appears.
4. Esc closes the card; second Esc exits mode.
5. `?` toggles mode (body focused, not an input).
6. Mode off: hover on a `HelpTarget` still mounts a tooltip portal (RFC-020
   layout-shift invariant).

---

## Rollout

Frontend-only. No migration, no gateway change, no plan gate. Ship behind the
normal dashboard deploy.

---

## Key Decisions

1. **Inspect mode is the primary help affordance**, not a tour and not
   wall-to-wall `?` icons.
2. **One catalog** owns copy; components take ids, not strings.
3. **Registered targets only**, with an explicit miss state.
4. **Hover stays** for jargon when mode is off.
5. **Empty states teach the loop**; they do not spotlight the chrome.
