# Enforce your OpenMetadata contracts at ingestion with DataContractGate

OpenMetadata (OM) is where your team finds data, owners, lineage and quality.
DataContractGate (DCG) sits in front of the warehouse. It checks every event against
a contract as the event arrives and quarantines the ones that fail, so they never land.

This integration connects the two:

1. **Result publisher.** For each DCG contract it keeps a test case on the matching OM
   table and posts a `Success` or `Failed` result for every validation window. The result
   includes the event count, quarantined count, violation rate and contract version. DCG's
   ingest-time enforcement then appears in OM's Data Quality tab, incidents and alerts,
   next to your warehouse tests.
2. **Contract sync.** It pulls an OM data contract as ODCS v3.1.0 YAML and imports it into
   DCG, so the contract you approved in the catalog is enforced at the front door.

It is about 400 lines of Python. The publisher and contract sync use only the standard
library; the optional DCG→OM shim needs PyYAML. MIT licensed, like DCG.

> **Scope.** DCG is the source of truth for contracts and ingest rules. OpenMetadata
> receives **results**, plus an optional **one-way OM → DCG** contract import. The reverse
> direction (DCG contracts into OM) is **not supported**: it is lossy, because OM drops
> DCG's constraint rules, glossary and metrics (see the round-trip findings below).

![DCG results on the OpenMetadata Data Quality tab](../../docs/images/openmetadata/03-table-data-quality.png)

---

## How it works

```
 producers ──► DataContractGate ──► warehouse table  ◄── OpenMetadata catalog
                 │  pass / quarantine                         ▲
                 │                                            │
                 └── GET /contracts/{id}/report ──► publisher ┘
                                                    POST /v1/dataQuality/testCases/
                                                         testCaseResults/{fqn}
```

| Piece | DCG endpoint | OM endpoint |
|---|---|---|
| Contract name, latest stable version | `GET /contracts/{id}` | |
| Windowed totals, quarantined count, per-version split, top violations | `GET /contracts/{id}/report?from=&to=&format=json` | |
| Custom test definition `dcgIngestContract` (created once) | | `GET/POST /v1/dataQuality/testDefinitions` |
| One test case per contract on the mapped table | | `GET/POST /v1/dataQuality/testCases` |
| One result per run | | `POST /v1/dataQuality/testCases/testCaseResults/{testCaseFqn}` |
| Contract sync, OM → DCG | `POST /contracts/import` (`target_contract_id`) | `GET /v1/dataContracts/{id}/odcs/yaml` |

Each run posts one result:

| Field | Value |
|---|---|
| `testCaseStatus` | `Failed` if `quarantined / total > max_violation_rate`, otherwise `Success` |
| `testResultValue` | `total`, `passed`, `quarantined`, `violation_rate`, `contract_version` |
| `passedRows` / `failedRows` | passed / quarantined events |
| `result` | e.g. `DCG contract orders v1.0.0: 180/200 passed, 20 quarantined (violation rate 10.00%, threshold 2.00%). Top violations: amount:range_violation x20, currency:enum_violation x20.` |

`contract_version` is the version that handled the most events in the window. If the
window has no events, the latest stable version is used.

---

## 1. Run OpenMetadata locally (free)

Use the official docker compose quickstart. The Airflow `ingestion` container isn't needed,
because the publisher writes results through the REST API. Leaving it out saves about 2 GB RAM.

```bash
mkdir om && cd om
curl -sL -o docker-compose.yml \
  https://github.com/open-metadata/OpenMetadata/releases/download/2.0.5-release/docker-compose.yml
docker compose -p openmetadata up -d mysql elasticsearch execute-migrate-all openmetadata-server
curl -s localhost:8585/api/v1/system/version     # {"version":"2.0.5",...}
```

Open <http://localhost:8585> and log in as `admin@open-metadata.org` / `admin` (the
quickstart default).

> Troubleshooting: if `execute_migrate_all` exits with a MySQL `Connect timed out`, the
> host firewall is dropping traffic between containers on the compose bridge network.
> Allow forwarding on that bridge (for example, `iptables -I DOCKER-USER -i br-<id> -j ACCEPT`
> and the same with `-o`), then run `up` again.

### Get a bot token

In OM, open **Settings → Bots → ingestion-bot** and copy the JWT. Or use the API:

```bash
ADMIN=$(curl -s -X POST localhost:8585/api/v1/users/login -H 'Content-Type: application/json' \
  -d "{\"email\":\"admin@open-metadata.org\",\"password\":\"$(printf admin | base64)\"}" | jq -r .accessToken)
BOT=$(curl -s -H "Authorization: Bearer $ADMIN" localhost:8585/api/v1/bots/name/ingestion-bot | jq -r .botUser.id)
export OM_JWT=$(curl -s -H "Authorization: Bearer $ADMIN" localhost:8585/api/v1/users/auth-mechanism/$BOT | jq -r .config.JWTToken)
```

### Create a demo table (optional)

```bash
export OM_HOST=http://localhost:8585
python examples/create_sample_table.py        # -> dcg_demo.analytics.public.orders
```

This creates the `dcg_demo` service, the `analytics` database, the `public` schema and an
`orders` table. Running it again is safe, because it uses PUT.

---

## 2. Configure

All secrets come from environment variables. Never put them in the mapping file.

| Env var | Meaning |
|---|---|
| `OM_HOST` | OM base URL, e.g. `http://localhost:8585` (`/api` is appended) |
| `OM_JWT` | OM bot JWT (ingestion-bot or a dedicated bot) |
| `DCG_API_URL` | e.g. `https://api.datacontractgate.com` |
| `DCG_API_KEY` | DCG API key (`cg_live_...`), sent as `x-api-key` |
| `DCG_ORG_ID` | Only for a local dev gateway with `CONTRACTGATE_DEV_NO_AUTH=1` (sent as `x-org-id`) |
| `DCG_OM_CONFIG` | Default path for `--config` |

See `.env.example`.

### Map contracts to tables

The mapping file is JSON (`examples/mapping.example.json`):

```json
{
  "window_minutes": 60,
  "skip_empty": true,
  "defaults": { "max_violation_rate": 0.0 },
  "contracts": [
    { "contract_id": "<dcg uuid>", "table_fqn": "dcg_demo.analytics.public.orders",
      "test_case_name": "dcg_orders_ingest", "max_violation_rate": 0.01 },
    { "contract_id": "<another uuid>" }
  ]
}
```

| Key | Default | Meaning |
|---|---|---|
| `contract_id` | required | DCG contract UUID |
| `table_fqn` | from contract metadata | OM table FQN (`service.database.schema.table`) |
| `test_case_name` | `dcg_<contract_name>` | OM test case name. The test case FQN is `<table_fqn>.<name>` |
| `max_violation_rate` | `defaults.max_violation_rate` or `0.0` | The result is `Failed` above this rate |
| `window_minutes` | none (all time) | Report window ending now. Set it to your schedule interval |
| `skip_empty` | `true` | Post nothing when the window has no events |
| `test_definition` | `dcgIngestContract` | Name of the custom test definition |

If `table_fqn` is missing, the publisher reads it from the **contract metadata**. It checks
the latest stable version's YAML first and then the contract description:

```yaml
# DCG contract YAML (top-level key)
x-openmetadata-table: "dcg_demo.analytics.public.orders"
```
or, in the contract description: `openmetadata-table: dcg_demo.analytics.public.orders`.

---

## 3. Publish results

```bash
cd integrations/openmetadata
python -m dcg_openmetadata publish --config mapping.json --dry-run   # reads DCG only
python -m dcg_openmetadata publish --config mapping.json
```

The output has one row per contract, and the exit code is 1 if any contract errored:

```json
[{ "contract_id": "40df…", "table_fqn": "dcg_demo.analytics.public.orders",
   "test_case": "dcg_demo.analytics.public.orders.dcg_om_integration_test_orders",
   "status": "Failed",
   "result": "DCG contract om_integration_test_orders v1.0.0: 180/200 passed, 20 quarantined (violation rate 10.00%, threshold 2.00%). Top violations: amount:range_violation x20, currency:enum_violation x20." }]
```

The first run creates the `dcgIngestContract` test definition and the test case. Later runs
only add results. Schedule it with cron at the same interval as `window_minutes`:

```cron
*/15 * * * * cd /opt/contractgate/integrations/openmetadata && python -m dcg_openmetadata publish --config mapping.json
```

![A passing and a failing DCG result on the test case page](../../docs/images/openmetadata/01-test-case-results.png)

![DCG test cases in the Data Quality overview](../../docs/images/openmetadata/04-data-quality-overview.png)

---

## 4. Sync a contract from OpenMetadata into DCG

```bash
# New DCG contract (draft) from an OM contract:
python -m dcg_openmetadata sync --om-contract-id <om contract uuid>
# New draft version on an existing DCG contract:
python -m dcg_openmetadata sync --om-contract-id <om uuid> --target-contract-id <dcg uuid>
```

This runs `GET /v1/dataContracts/{id}/odcs/yaml` (with `Accept: application/yaml`) and then
`POST /contracts/import`. DCG imports the document as a **draft** with
`requires_review: true`. Review it and promote it in DCG before it is enforced.

### Round trip: what survives DCG → OM → DCG

`examples/roundtrip_check.py` exports a DCG version, imports it into OM on a table, exports
it from OM and re-imports it into DCG with `target_contract_id`. It then diffs the `x-`
extensions. Results against OM 2.0.5 after the strict-ODCS export (2026-10-15, see
`docs/odcs/strict-odcs.md`):

| Item | DCG export → OM | OM export | Back in DCG |
|---|---|---|---|
| Field names, logical types (`number`, not `double`), `required` | kept | kept | kept |
| Contract `id` | used as the OM contract id | kept | n/a (target id passed) |
| `description` (now the ODCS object form) | accepted as-is | kept as `purpose` | **kept** (DCG reads the object form) |
| `enum` (native `invalidValues` rule with `validValues`) | accepted | **kept** | **kept** as `enum` + a validity rule |
| `pattern`, `minimum`/`maximum`, lengths (native `logicalTypeOptions`) | accepted | **dropped by OM** | lost |
| Nested `customProperties` (`x-cg-*`, `x-cg-quality`) | **rejected (HTTP 400)**. OM deserializes them as an object, while the spec says array. The shim drops them | gone | covered by the native fields above where OM keeps them |
| Root `customProperties` (`x-contractgate-*`) | object values **rejected**. OM needs string values, so the shim JSON-encodes them | **silently dropped** | glossary, metrics and transforms **lost** |
| `type: custom` freshness rule | accepted (string `implementation`) | not returned | lost |
| `name`, `version` | ignored. OM names it `contract_<hash>`, version `0.x` | as OM set them | DCG keeps its own name. The version comes from the doc |

**Bottom line:** OM is still not a lossless store for DCG contracts, but more now
survives. The OM→DCG direction gives fields, types, required, description and enums
(`import_source: odcs_stripped`, `requires_review: true`). Pattern and range rules must be
re-added in DCG. Keep DCG as the source of truth for ingest rules and push only *results*
to OM. `dcg_openmetadata.sync.dcg_odcs_for_om()` (needs `pip install pyyaml`) now only
works around OM 2.0.5's two deviations from the ODCS schema: nested `customProperties`
and non-string custom property values. It still wraps a string `description` for older
DCG exports.

---

## 5. Try it end to end

```bash
export OM_HOST=http://localhost:8585 OM_JWT=... DCG_API_URL=... DCG_API_KEY=...
python examples/create_sample_table.py
CID=$(python examples/seed_dcg_demo.py --good 100)              # test contract + 100 good events
echo "{\"window_minutes\":10,\"contracts\":[{\"contract_id\":\"$CID\",\"max_violation_rate\":0.02}]}" > /tmp/map.json
python -m dcg_openmetadata publish --config /tmp/map.json      # -> Success
python examples/seed_dcg_demo.py --contract-id $CID --good 80 --bad 20
python -m dcg_openmetadata publish --config /tmp/map.json      # -> Failed (10% > 2%)
python examples/seed_dcg_demo.py --delete $CID                 # clean up DCG
```

### Clean up OpenMetadata

```bash
python examples/cleanup_om.py                    # test case(s), dcgIngestContract definition,
                                                 # dcg_demo service (+ db, schema, table, contract)
python examples/cleanup_om.py --keep-definition  # keep the definition if other tables use it
```

This uses hard deletes, so it can't be undone. Skip it if you want to keep the demo pages.

The seed contract is named `om_integration_test_orders` and declares
`x-openmetadata-table`, so it needs no `table_fqn`.

Pages to look at (local OM):

- Test case results: `http://localhost:8585/test-case/dcg_demo.analytics.public.orders.dcg_om_integration_test_orders/test-case-results`
- Table data quality tab: `http://localhost:8585/table/dcg_demo.analytics.public.orders/profiler/data-quality`
- Data quality overview: `http://localhost:8585/data-quality/test-cases`

---

## Tests

```bash
cd integrations/openmetadata
python -m unittest discover -s tests -v
```

`tests/test_publisher.py` covers the result mapping, thresholds, version choice, mapping
config and metadata lookup. `tests/test_mock_om.py` starts an in-process mock of OM and DCG
and checks that the definition and test case are created once and that results, auth
headers and `target_contract_id` are sent correctly.

## Screenshots

`docs/images/openmetadata/`: `01-test-case-results.png`, `03-table-data-quality.png`,
`04-data-quality-overview.png` (OM 2.0.5, captured with headless Chrome).
