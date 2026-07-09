<!--
RFC-078 — Walkthrough spine.

Every supported ingestion surface gets ONE walkthrough that fills these five
beats, in this order, with these headings. The consistency is the point: the
second walkthrough a reader opens should feel familiar in 90 seconds.

Rules for filling this in:
- Keep prose minimal. A walkthrough is copy-paste-first, not an essay.
- The passing and failing records MUST be the literal inputs used in that
  surface's test, so the doc and the test cannot drift.
- Link to the surface's reference doc for exhaustive detail; do not duplicate it.
- Replace every <SLOT> below. Delete this comment block in the real page.
-->

# <SURFACE> Walkthrough

One-sentence statement of what this surface gates and where it sits in the
pipeline. Link to the [<SURFACE> reference](<reference-doc>.md) for full detail.

## 1. The contract

Copy-paste YAML for this surface. Link to the runnable file in
`examples/contracts/<surface>/`.

```yaml
# <contract YAML>
```

## 2. The command

The exact invocation. For local validation, `cg test`:

```
cg test --contract <contract>.yaml --data <data>.ndjson
```

## 3. A passing record

The clean input and the result it produces.

```json
{ /* passing record */ }
```

```
PASS  1
1/1 records passed   validated in <X>ms
```

## 4. A failing record

A record that the gate rejects, and the specific violation. Show the gate
biting — pick the most surface-relevant failure (e.g. unredacted PII, stale
timestamp, off-allowlist source).

```json
{ /* failing record */ }
```

```
FAIL  1
record  0  <field>  <violation_kind>  <message>
```

## 5. Wire it in

The minimal snippet to put the check in the reader's real pipeline — the
endpoint call, a CI step, or a producer hook. Link to the relevant ingress
reference for the production path.
