# Readiness Review

`contractgate review <file.csv>` is a free sales-call kit: a CSV goes in, and
out comes a draft DataContractGate contract plus a report of rejected rows
with plain-English reasons.

## What it writes

Given `contractgate review data.csv --out ./review-out`:

| File | Contents |
|---|---|
| `review-out/contract.yaml` | Draft contract inferred from the file (types, required, low-cardinality enums, ranges, email/date/uuid patterns). |
| `review-out/report.html` | Self-contained DataContractGate-branded report: summary counts, pass rate, top failing rules, PII fields flagged, sample rejected rows with reasons. PII is masked. |
| `review-out/rejected.csv` | Every rejected row, PII masked, plus `_row` and `_reasons`. |

The draft is inferred from rows that match the majority type of each column,
using the same inference engine as `POST /contracts/infer` and
`contractgate infer`. Obvious outliers (a bad email in a column of emails, a
negative amount in a column of non-negative numbers) are then checked against
that draft with the same validator ingest uses. PII detection reuses
`scaffold`'s name-and-value detector; flagged values are masked in both the
HTML report and `rejected.csv`.

Exit code `0` means the files were written, even when some rows were
rejected. Exit code `2` is a CSV or compile error.

## CLI

```bash
contractgate review data.csv
contractgate review data.csv --out ./review-out --name orders
```

`--out` defaults to `<file-stem>-review` next to the CSV. `--name` defaults
to the file stem.

If the Rust CLI is on `PATH`, `contractgate-py review` is a thin wrapper
that calls it.

A timed walkthrough and two synthetic sample CSVs (generated with
`contractgate synth` in invalid mode, never real data) live in
[examples/readiness-review/](../examples/readiness-review/README.md). A
Docker one-liner is documented there.

There is no API endpoint and no database migration for this command. It is
local-only.
