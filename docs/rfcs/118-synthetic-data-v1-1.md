# RFC-118: Synthetic data v1.1 and cross-field rules

**Status:** Accepted
**Date:** 2026-11-25

## Problem

RFC-117 shipped synthetic data with known gaps. The contract format had no way to say "end date after start date", so the generator could not follow such rules. Invalid mode never broke rules inside array items. Rows could not be wrapped in a contract's envelope. Freshness values moved with the clock, so a seed alone did not repeat. Only the API and the Rust CLI could generate data.

## Decision

1. **Cross-field rules in the contract format.** A new optional top-level `cross_field:` list. Each rule is either a comparison (`field`, `op` in `lt`/`lte`/`gt`/`gte`/`eq`/`ne`, `other`) or a conditional requirement (`field`, `required_if: {field, equals}`). The change is additive: the field is skipped when empty on serialize, so existing contracts and ODCS exports are byte-identical. The validator enforces the rules after quality rules and reports `cross_field_violation` with the rule label in `expected`. Comparisons skip absent or `null` values, and ordering comparisons pass on values with no order, so a type error is not reported twice. Reference: `docs/cross-field-rules-reference.md`.
2. **Generator follows cross-field rules.** Valid rows are repaired toward each rule before the full validator check (swap, regenerate, fill or copy). Invalid mode breaks one cross-field rule at a time, labeled with rule `cross_field` and the rule's label in `constraint`. A row is kept only when its single violation is that rule.
3. **Array items in invalid mode.** Targets now include rules inside array items (type, required, enum, ranges, lengths, formats of object properties or scalar items), applied to the first item at path `field[0].prop`. Patterns inside items are not targeted, because the validator does not check them there yet.
4. **Envelope.** `envelope: true` on the API (and `--envelope` on both CLIs) also returns the rows wrapped in the contract's `envelope` (RFC-038), with `success` and a one-page `pagination` block when `validate_wrapper` is set. Contracts without an envelope return 400 `no_envelope`.
5. **Anchor time.** `anchor_time` (RFC 3339; `--anchor` on the CLIs) sets the time freshness rules are measured from. Default now. The generator's validator oracle uses the anchor too (`validate_at`), so seed plus anchor repeats exactly. Rows anchored far from now fail real-time freshness on ingest, which is expected.
6. **Dashboard.** A "Synthetic data" tab on the contract page: rows, mode, seed, anchor, envelope, preview, JSON and CSV download. It sends the YAML on screen, so drafts work.
7. **Python SDK.** `contractgate synth` console script and `Client.synth` / `AsyncClient.synth`, calling the API. Version 0.4.2, not published.

No database migration.

## Not in this version

- Patterns inside array items: the validator does not enforce them, so the generator does not break them.
- Cross-field rules across array items (for example each line's `qty` against a header field).
- The Python SDK's local validator ignores `cross_field`.
- Rules coupled for real stay in `skipped_rules`, for example a `min` that can only be broken by also breaking a cross-field comparison.
