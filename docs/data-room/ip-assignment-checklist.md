# IP Assignment & Ownership Checklist

**Owner:** Alex (founder). Purpose: give an acquirer a clean, unambiguous chain of
title to the ContractGate IP. These are **non-code** items; engineering can't
close them, but diligence will ask for every one. Fill in and attach the
supporting documents to the data room.

Status legend: ✅ done · ⬜ open · ❔ confirm

## Chain of title

- ⬜ **Founder IP assignment** — a signed agreement assigning all ContractGate IP
  (code, designs, docs, the patent application) from Alex Suarez personally to
  the owning entity (nightmoose / the legal entity being sold). Attach.
- ⬜ **Contractor / contributor assignments** — for anyone (paid or unpaid) who
  contributed code, designs, or content: a signed IP-assignment or work-for-hire.
  If there are none, state "sole author" explicitly. Check the git contributor
  list against this.
- ❔ **Prior-employer / moonlighting clearance** — confirm no current/former
  employer has a claim (invention-assignment clauses, developed-on-company-time).
- ⬜ **Entity ownership** — cap table / who owns the entity that owns the IP.

## Patent

- ⬜ **Docket details** — the repo asserts "Patent Pending." Record:
  application/serial number, filing date, jurisdiction(s), status, and counsel of
  record. Attach the filing receipt.
- ❔ **Assignment recorded** — confirm the application is assigned to the entity
  (not the individual) and recorded with the patent office.

## Trademarks & brand

- ❔ **"ContractGate" / "nightmoose"** — any registered or common-law marks;
  domains (`datacontractgate.com`, `nightmoose.com`) and who holds them.

## Open-source & licensing

- ✅ Project license: **MIT** ([LICENSE](../../LICENSE), [NOTICE](../../NOTICE)).
- ✅ **Dependency license inventory** — Rust tree: [dependency-licenses.md](./dependency-licenses.md)
  (cargo-about; regenerate via `about.toml` + `about.hbs`). Optional follow-up:
  npm dashboard scan + `deny.toml` `[licenses]` CI allowlist.
- ❔ **Third-party assets** — fonts, icons, sample datasets (e.g. ACS catalog
  data): confirm redistribution rights.

## Data & privacy

- ❔ **Customer data ownership / DPA** — for hosted pilots, confirm terms on who
  owns ingested data and any processing agreements.

---

*Rust dependency inventory is in-tree. Remaining items are legal/founder work.*
