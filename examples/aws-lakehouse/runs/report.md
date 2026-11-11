# P8 lakehouse contract run

- Time: 2026-11-11T11:45:42Z (UTC)
- Region: us-east-1
- Bucket: `nmb-lakehouse-065614567889`
- Contract: `stripe_charges_raw` (in-process contractgate, not the staging or prod API)
- Records: 10000 (seed 42)
- Passed / quarantined: **9495 / 505** (reject rate 0.0505)
- Iceberg counts match write-time split: True
- Athena DQ bytes scanned: 107640
- Wall clock: 39.36 s
- Check p50: 11237 ns

## Produced labels

| label | n |
|---|---|
| clean | 9110 |
| guest_checkout | 290 |
| unknown_status | 106 |
| negative_amount | 104 |
| missing_created | 104 |
| uppercase_currency | 102 |
| bad_email | 95 |
| bad_id_prefix | 89 |

## Write-time violation kinds

| kind | n |
|---|---|
| status:enum_violation | 106 |
| amount:range_violation | 104 |
| created:missing_required_field | 104 |
| currency:pattern_mismatch | 102 |
| id:pattern_mismatch | 89 |

## Athena checks on the passed Iceberg table (should all be 0)

| check | rows |
|---|---|
| negative_amount | 0 |
| bad_currency | 0 |
| bad_status | 0 |
| missing_created | 0 |
| bad_id | 0 |

## Notes

- Write-time validation used the in-process contractgate package, not the staging or prod API.
- Python contractgate 0.4.x does not enforce format:email, so bad_email rows may pass ingest.
- Guest checkouts (no customer) are allowed by stripe_charges_raw and land in charges_raw.
- Glue Data Quality (R12 nmb-glue) was not used; no nmb-glue role exists, and Athena SQL covered the checks.
