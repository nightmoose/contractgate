# Readiness Review, in under 10 minutes

A free sales-call kit: a CSV goes in, and out comes a draft DataContractGate
contract plus a report of rejected rows with plain-English reasons. PII
fields are flagged and masked in the report.

This folder has two synthetic sample CSVs (never real customer data). They
were generated with `contractgate synth` in valid and invalid mode from the
`_orders_source.yaml` and `_signups_source.yaml` contracts in this directory.

## Prerequisites

A release build of the Rust CLI on your `PATH`:

```bash
cargo build --release --bin contractgate
export PATH="$PWD/target/release:$PATH"
```

## Script (about one minute on a development host)

```bash
# 1. Review the checkout extract.
contractgate review examples/readiness-review/orders.csv --out /tmp/dcg-review-orders

# 2. Open the report (macOS: open, Linux: xdg-open).
xdg-open /tmp/dcg-review-orders/report.html 2>/dev/null || open /tmp/dcg-review-orders/report.html

# 3. Optionally review a second file (customer signups).
contractgate review examples/readiness-review/signups.csv --out /tmp/dcg-review-signups --name customer_signups
```

What you should see:

- `contract.yaml`: a draft inferred from the majority type of each column
  (types, required, low-cardinality enums, ranges, email/date/uuid patterns).
- `report.html`: a self-contained DataContractGate-branded summary (counts,
  pass rate, top failing rules, PII fields, sample rejected rows with reasons).
  Emails and other flagged PII are masked.
- `rejected.csv`: every rejected row, PII masked, with `_row` and `_reasons`.

Measured at 4:20 PM America/New_York on 2026-11-25, from this repository,
with a release `contractgate` already on `PATH`: `orders.csv` (50 rows, 4
rejected) took 7.0 ms and `signups.csv` (50 rows, 2 rejected) took 8.7 ms.
A consecutive run of both took 15.3 ms. The rest of a live call is walking
through the HTML report.

## Docker one-liner

From the repository root, after the image is built once:

```bash
docker build -t dcg-review -f examples/readiness-review/Dockerfile .
docker run --rm -v "$PWD/examples/readiness-review:/data" dcg-review \
  /data/orders.csv --out /data/out-orders
```

Then open `examples/readiness-review/out-orders/report.html`.

## Regenerating the samples

```bash
contractgate synth examples/readiness-review/_orders_source.yaml \
  --rows 40 --seed 11 --format csv --anchor 2026-11-25T15:00:00Z
# Invalid-mode rows (one planted break each) use --invalid --seed 13, then
# drop the `_broken_field` and `_broken_rule` columns and interleave.
```

The Python CLI wrapper, when the Rust binary is on `PATH`:

```bash
contractgate-py review examples/readiness-review/orders.csv --out /tmp/dcg-review-orders
```
