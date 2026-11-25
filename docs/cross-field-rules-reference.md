# Cross-field rules reference

A contract can compare one field with another, or require a field only when another field has a given value. Rules live in an optional top-level `cross_field:` list. Contracts without it parse and validate exactly as before. Design: [RFC-118](rfcs/118-synthetic-data-v1-1.md).

```yaml
cross_field:
  - field: end_date        # end_date >= start_date
    op: gte
    other: start_date
  - field: host_email      # host_email != guest_email
    op: ne
    other: guest_email
  - field: refund_reason   # refund_reason required if status == "refunded"
    required_if:
      field: status
      equals: refunded
    description: Refunds need a reason.
```

Each rule sets `field` and exactly one of:

- `op` and `other`: `field <op> other`. `op` is one of `lt`, `lte`, `gt`, `gte`, `eq`, `ne`.
- `required_if: {field, equals}`: when the other field equals the value (any JSON value), `field` must be present and not `null`.

`field`, `other` and `required_if.field` accept dot paths into nested objects (`shipping.country`). `description` is optional. A rule that sets both forms, or neither, fails to compile.

## Rules on array items

Put `[]` after an array name to check the rule once per item:

```yaml
cross_field:
  - field: lines[].qty           # every line's qty <= max_line_qty
    op: lte
    other: max_line_qty
  - field: lines[].fee_reason    # within a line: fee_reason required if kind == "fee"
    required_if:
      field: lines[].kind
      equals: fee
```

Paths with `[]` refer to the same item, so `lines[].fee_reason` and `lines[].kind` read one line at a time. A path without `[]` (`max_line_qty`) is a top-level field, the same for every item. A violation is reported on the item path (`lines[2].qty`), with the rule label (`lines[].qty <= max_line_qty`) in `expected`. A missing or empty array produces no checks. A path may use `[]` once, and all `[]` paths in one rule must name the same array; other rules fail to compile.

## How values compare

| Both values are | Compared as |
|---|---|
| numbers | numbers |
| RFC 3339 timestamps | instants (offsets respected) |
| `YYYY-MM-DD` dates | calendar dates |
| other strings | text, byte order |

For `eq` and `ne`, values with no order (booleans, objects, mixed types) are compared as JSON values. For `lt`, `lte`, `gt` and `gte`, a pair with no order passes, because the field's own `type` or format rule already reports the bad value.

A comparison is skipped when either field is absent or `null`. Use `required: true` for presence.

## Violations

A failed rule is reported on `field` with kind `cross_field_violation`. `expected` holds the rule's label, for example `end_date >= start_date` or `refund_reason required if status == "refunded"`.

## Where it applies

- Ingest, egress, the playground, `contractgate test` and every other path that uses the gateway validator.
- The synthetic data generator follows the rules in valid mode and breaks them one at a time in invalid mode ([reference](synthetic-data-reference.md)).
- ODCS export keeps the rules in `x-contractgate-ontology`, so an export and re-import round-trips them.
- The Python SDK's local validator (`Contract.compile().validate(...)`) enforces the same rules with the same violations. Both are checked against one shared fixture set (`tests/fixtures/parity/rfc118.json`).
