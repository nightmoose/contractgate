# RFC-036: Source-First New Contract Flow

**Status:** Draft  
**Date:** 2026-05-16  
**Author:** Alex Suarez  

---

## Problem

The `+ New Contract` button drops users into a raw YAML textarea with boilerplate.
This works for power users but leaves everyone else staring at a blank page with no
guidance on where their contract's data is actually coming from. A contract created
without a source is incomplete by definition — source traceability is the whole point
of the gateway.

Meanwhile, three high-value source paths already exist but are buried as separate
top-level tabs (CSV inference, public catalog fork). Users who would benefit most from
them don't discover them because they're not offered at the moment of intent.

---

## Goals

1. Make `+ New Contract` present a **source picker** as the first step — eliminating
   the blank-page problem and surfacing existing creation paths at the right moment.
2. Support three source paths inside the wizard:
   - **Fork from Catalog** — pick a curated public contract, customize name/filter,
     create a fork in one action.
   - **Infer from CSV** — upload or paste a CSV; backend infers the YAML contract.
   - **Start blank** — raw YAML editor for users who know their schema.
3. No new backend routes. All required APIs already exist (RFC-034, RFC-035).
4. Do not remove existing Contracts page tabs (`From CSV`, `Generate`, `Visual Builder`,
   `Quarantine`). The wizard is an entry-point shortcut, not a replacement.

---

## Non-Goals

- Inlining the Visual Builder or "Generate from Sample" paths into the wizard (those
  creation methods are not source-driven in the same way; they remain as tabs).
- Filter configuration inside the fork step (fork is created with no filter; the user
  can configure predicates on the resulting contract after creation, as today).
- URL-based upstream source as a creation entry-point (deferred — no public contract
  catalog entry would be created from the wizard in v1).

---

## Decisions

| # | Question | Decision |
|---|---|---|
| D1 | Replace `ManualCreatePanel` inline or use modal? | Modal. Keeps the page layout stable and matches existing modal patterns in the codebase. |
| D2 | Show filter UI during fork in wizard? | No. Fork with name only; filter config happens post-creation in the contract editor. Reduces wizard surface area significantly. |
| D3 | Catalog picker — all contracts or paginated? | All (no pagination in v1). Public catalog is admin-curated and small. Add pagination if it grows past ~20 entries. |
| D4 | Where does CSV flow live after wizard? | Wizard embeds a trimmed version of `CsvGeneratorTab` (paste/upload + Infer button + YAML preview). On save, closes wizard and navigates to list. The full CSV tab stays. |
| D5 | "Start blank" path — YAML seed? | Pre-fill with `EXAMPLE_YAML` (same as today's `ManualCreatePanel`). |
| D6 | Do the existing tabs change? | No. The `From CSV` tab, `Generate`, `Visual Builder`, `Quarantine` tabs are untouched. |
| D7 | Where is the wizard component? | New file: `dashboard/app/contracts/NewContractWizard.tsx`. Imported and rendered in `page.tsx`, replacing the `showCreate`/`ManualCreatePanel` pattern. |

---

## UX Flow

```
[+ New Contract] click
        │
        ▼
┌─────────────────────────────────────────────┐
│  How do you want to start?                  │
│                                             │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐ │
│  │ 🗂 Fork from  │ │ 📊 Infer     │ │ ✏️ Start     │ │
│  │   Catalog    │ │   from CSV   │ │   Blank      │ │
│  │              │ │              │ │              │ │
│  │ Use a        │ │ Upload or    │ │ Open YAML    │ │
│  │ curated open │ │ paste a CSV; │ │ editor with  │ │
│  │ data source  │ │ fields auto- │ │ example      │ │
│  │ as your base │ │ inferred     │ │ contract     │ │
│  └──────────────┘ └──────────────┘ └──────────────┘ │
└─────────────────────────────────────────────┘
        │
     [pick one]
        │
   ┌────┴────────────────────────────────┐
   │                                     │
   ▼ Fork from Catalog                   ▼ Infer from CSV / Start Blank
   ─────────────────                     ──────────────────────────────
   List public contracts                 Embedded CSV infer UI
   [search by name]                          or
   → select one                         Raw YAML editor
   → enter Fork Name                    → [Save Contract]
   → [Fork into my contracts]
   → contract created; wizard closes
```

---

## Component Design

### `NewContractWizard.tsx`

```tsx
type WizardStep = "pick" | "catalog" | "csv" | "blank";

interface Props {
  onClose: () => void;
  onCreated: () => void;
}
```

**Step: `pick`** — Three tiles. Clicking a tile transitions to that step.

**Step: `catalog`**  
- `GET /public-contracts` on mount.  
- Renders a scrollable list of `OpenDataContract` summaries (name, description, source_format, version).  
- Search input filters client-side.  
- Selected row highlighted; shows a "Fork Name" text input below.  
- "Fork into my contracts" calls `forkPublicContract(id, { name, description: "" })`.  
- On success: `onCreated()`.

**Step: `csv`**  
- Paste/upload toggle (same as `CsvGeneratorTab`).  
- "Infer from CSV" calls `inferCsv(...)`.  
- Editable YAML textarea for review.  
- "Save Contract" calls `createContract(yaml)` then `onCreated()`.

**Step: `blank`**  
- Pre-filled with `EXAMPLE_YAML`.  
- "Create Contract" calls `createContract(yaml)` then `onCreated()`.

---

## Changes to `page.tsx`

- Remove `showCreate` state and `ManualCreatePanel` usage.
- Add `showWizard` boolean state.
- `+ New Contract` button sets `showWizard = true`.
- Render `<NewContractWizard onClose={...} onCreated={...} />` when `showWizard`.
- All other state, tabs, and modals unchanged.

---

## No Backend Changes

All APIs consumed by this RFC are already implemented:

| API | Implemented in |
|-----|---------------|
| `GET /public-contracts` | RFC-034 / `public_catalog.rs` |
| `POST /contracts/:id/fork` | RFC-034 / `public_catalog.rs` |
| `POST /infer-csv` | RFC-035 |
| `POST /contracts` (create) | existing |

---

## Acceptance Criteria

- [ ] `+ New Contract` opens the source picker modal (not the inline YAML panel).
- [ ] Catalog step: public contracts load and are filterable by name.
- [ ] Catalog step: fork creates a contract in the user's org and wizard closes.
- [ ] CSV step: paste/upload → infer → editable YAML → save works end-to-end.
- [ ] Blank step: pre-filled YAML editor → save works.
- [ ] Escape key and ✕ button close wizard without creating a contract.
- [ ] Existing tabs (`From CSV`, `Generate`, `Visual Builder`, `Quarantine`) still work.
- [ ] `ManualCreatePanel` and `showCreate` state removed from `page.tsx`.
- [ ] `npm run build` passes with no type errors.

---

## Resolved Questions

- **OQ1:** No YAML preview in the Catalog step. Users can delete the forked contract
  if they don't want it — delete is cheap, preview adds friction. **Closed: no preview.**
- **OQ2:** No auto-navigation to the `csv` tab post-creation. The wizard shows enough
  inline context. **Closed: stay on list view.**
