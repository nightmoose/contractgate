# IP assignment checklist (owner / legal)

**Not automated.** Complete offline and attach evidence to the data room package
you send externally. Engineering maintains the checklist structure only.

Last updated: 2026-07-15

---

## Entity & ownership

- [ ] Operating entity name and jurisdiction documented
- [ ] Product trademarks / domain ownership confirmed (`datacontractgate.com`, etc.)
- [ ] GitHub org ownership and admin access documented

## Founder / employee IP

- [ ] Founder IP assignment / invention assignment agreement signed to the entity
- [ ] Any prior open-source contributions to this codebase covered by assignment or CLA
- [ ] No uncleared co-founders or informal collaborators with residual claims

## Contractors / freelancers

- [ ] All paid contractors signed work-for-hire / IP assignment covering ContractGate
- [ ] List of contractor GitHub handles + date ranges (optional appendix)

## Patent

- [ ] Patent counsel name
- [ ] Application serial number(s)
- [ ] Filing date(s)
- [ ] Status: provisional / non-provisional / pending / granted
- [ ] Relationship to MIT-licensed code clarified for counterparties (see `NOTICE`)

## Third-party code

- [ ] MIT `LICENSE` + `NOTICE` current
- [ ] Dependency license inventory reviewed (`docs/data-room/dependency-licenses.md`)
- [ ] No known GPL/AGPL contamination in the Rust production binary path
- [ ] Dashboard npm licenses acceptable for distribution model (optional separate scan)

## Evidence to attach (external package)

- Signed assignment PDFs (redact as needed)
- Patent filing receipt / docket export
- Generated `dependency-licenses.md` (or HTML from cargo-about)
