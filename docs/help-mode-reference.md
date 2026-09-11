# What’s this? — in-app help mode

Inspect mode for the ContractGate dashboard. Toggle it, click a highlighted
control, get a short description of what it is and what it does.

This is a dashboard-only feature. It does not change ingest, contracts, or the
validation engine.

---

## Turn it on

- Sidebar footer: **What’s this?**
- Keyboard: `?` (ignored while a text field is focused)
- Small screens: the floating `?` button, bottom-right

Exit with **Esc**, the banner **Exit** button, or toggle again. Mode does not
survive a reload.

---

## What you get

| Layer | When | What it shows |
|---|---|---|
| Hover tooltip | Mode off, jargon / registered label | One sentence |
| Inspect card | Mode on, click a highlighted control | Title, what it is, what it does, optional when-to-use, optional docs link |
| Miss toast | Mode on, click something unregistered | “No description for this yet.” |
| Empty-state copy | Contracts / Dashboard / Audit with no data | Create → promote stable → ingest (pass / quarantine) |

Highlighted controls use a dashed green outline. Clicks on them open the card
instead of firing the button or navigating.

---

## Coverage

Registered in v1:

- Every sidebar item
- Page titles (Dashboard, Contracts, Catalog, Scorecard, Audit, Scaffold,
  Workbench, Playground, Account, Stream Demo, Docs, Pricing)
- Dashboard stat cards
- Contract tabs and the main create/import actions
- RFC-020 jargon (ontology, glossary, metrics, draft/stable/deprecated,
  quarantine, replay, PII transforms, routing modes)
- The densest controls on Catalog, Scorecard, Scaffold, Workbench, Playground,
  Audit, and Account

Unregistered controls are expected. Add a catalog entry and wrap the label
with `HelpTarget` when a new surface ships.

---

## Adding a help target

1. Add an entry to `dashboard/lib/help/catalog.ts` with a stable id
   (`nav.*`, `page.*`, `term.*`, or `<surface>.*`).
2. Wrap the control:

   ```tsx
   import { HelpTarget } from "@/components/help/HelpTarget";

   <HelpTarget id="term.quarantine">
     <span>Quarantine</span>
   </HelpTarget>
   ```

3. Keep dynamic, one-off tooltips (e.g. “purged — past retention”) on
   `TooltipWrap`. Do not stuff those into the catalog.

Copy rules: one sentence for `what`, one for `does`, plain English, no
unexplained acronyms. Match the RFC-020 glossary tone.

---

## Keyboard

| Key | Action |
|---|---|
| `?` | Toggle inspect mode (not while typing) |
| `Esc` | Close the open card, or exit mode if no card is open |

---

## Related

- RFC-091 — help catalog + inspect mode
- RFC-020 — original jargon tooltip layer (`TooltipWrap`)
