# RFC-118: Synthetic data v1.1 and cross-field rules

**Status:** Accepted
**Date:** 2026-11-25

## Problem

RFC-117 shipped synthetic data with known gaps. The contract format had no way to say "end date after start date", so the generator could not follow such rules. Invalid mode never broke rules inside array items. Rows could not be wrapped in a contract's envelope. Freshness values moved with the clock, so a seed alone did not repeat. Only the API and the Rust CLI could generate data.

## Decision

1. **Cross-field rules in the contract format.** A new optional top-level `cross_field:` list. Each rule is either a comparison (`field`, `op` in `lt`/`lte`/`gt`/`gte`/`eq`/`ne`, `other`) or a conditional requirement (`field`, `required_if: {field, equals}`). The change is additive: the field is skipped when empty on serialize, so existing contracts and ODCS exports are byte-identical. The validator enforces the rules after quality rules and reports `cross_field_violation` with the rule label in `expected`. Comparisons skip absent or `null` values, and ordering comparisons pass on values with no order, so a type error is not reported twice. Reference: `docs/cross-field-rules-reference.md`.
2. **Generator follows cross-field rules.** Valid rows are repaired toward each rule before the full validator check (swap, regenerate, fill or copy). Invalid mode breaks one cross-field rule at a time, labeled with rule `cross_field` and the rule's label in `constraint`. A row is kept only when its single violation is that rule.
3. **Array items in invalid mode.** Targets now include rules inside array items (type, required, enum, ranges, lengths, formats of object properties or scalar items), applied to the first item at path `field[0].prop`. Patterns inside items are not targeted, because the validator did not check them there yet (both added in v1.2).
4. **Envelope.** `envelope: true` on the API (and `--envelope` on both CLIs) also returns the rows wrapped in the contract's `envelope` (RFC-038), with `success` and a one-page `pagination` block when `validate_wrapper` is set. Contracts without an envelope return 400 `no_envelope`.
5. **Anchor time.** `anchor_time` (RFC 3339; `--anchor` on the CLIs) sets the time freshness rules are measured from. Default now. The generator's validator oracle uses the anchor too (`validate_at`), so seed plus anchor repeats exactly. Rows anchored far from now fail real-time freshness on ingest, which is expected.
6. **Dashboard.** A "Synthetic data" tab on the contract page: rows, mode, seed, anchor, envelope, preview, JSON and CSV download. It sends the YAML on screen, so drafts work.
7. **Python SDK.** `contractgate synth` console script (renamed `contractgate-py` in v1.2) and `Client.synth` / `AsyncClient.synth`, calling the API. Version 0.4.2, not published.

No database migration.

## v1.2 (same RFC, follow-up)

The v1.1 gaps are closed:

1. **Patterns inside array items** are enforced by the validator (`lines[2].sku` is checked against `lines[].sku`'s pattern) and broken by invalid mode. This tightens validation: events whose item strings do not match a declared item pattern used to pass and now fail with `pattern_mismatch`.
2. **Cross-field rules on array items.** A path may contain `[]` (`lines[].qty lte max_line_qty`, `lines[].fee_reason required_if lines[].kind == fee`). The rule runs once per item; `[]` paths in one rule share the item, plain paths are top-level. Violations name the item (`lines[0].qty`). The generator follows and breaks them. At most one `[]` per path and one array per rule; other shapes fail to compile.
3. **Python local validator parity.** The Python SDK enforces `cross_field`, quality `completeness` and `freshness`, item patterns and `null_as_absent`, and counts lengths in UTF-8 bytes like Rust. A shared corpus, `tests/fixtures/parity/rfc118.json`, runs in both `tests/parity_rfc118.rs` and `sdks/python/tests/test_parity_rfc118.py` and must give identical `(field, kind, message)` lists. `PARITY_WRITE=1 cargo test --test parity_rfc118` regenerates expectations from Rust.
4. **Fewer skipped rules.** Invalid mode repairs the other fields around a broken target (shifts the other side of a comparison, flips a `required_if` condition, drops an optional field) before giving up. Freshness now accepts RFC 3339 strings and float epochs as well as integer epochs, so `type` on a freshness field can be broken with a fresh value of another representation, and freshness on `timestamp` fields is generated as RFC 3339. A rule that still cannot fail alone is skipped with a static `reason` naming the rule it is tied to; the remaining cases are listed in `docs/synthetic-data-reference.md` and are logically impossible to break alone.
5. **Versions.** `Cargo.toml`, `mcp/` and the Python SDK are all 0.4.2, so a `v0.4.2` tag passes the release checks. Not tagged or published.
6. **CLI names.** The Python console script is `contractgate-py` (and `python -m contractgate`), so it no longer collides with the Rust `contractgate` binary. 0.4.2 was never published, so no installed user loses a command.

No database migration.
