# Connect Fivetran

**Status:** Connect Fivetran is available on paid hosted plans (`growth`, `aisg_pro`, `enterprise`) and on any self-hosted instance with `CONTRACTGATE_SELF_HOSTED=true`. Hosted free plans get 403. Auth stays on. `CONTRACTGATE_SELF_HOSTED` accepts `1`, `true`, or `TRUE`.

Deploys the public ContractGate connector into the caller's own Fivetran account.
ContractGate is DataContractGate and AISchemaGate sharing one engine. The connector
reads records that passed validation and writes one table per contract.

The dashboard button is on **Contracts**. The form opens when `GET /v1/me`
returns `paid_features: true`, including a self-hosted free org. If that field
is missing, the form follows the stored plan. A hosted free org sees an
upgrade prompt. The same call is `POST /v1/integrations/fivetran/connect`.
The guide is [dashboard/app/docs/fivetran/page.tsx](../dashboard/app/docs/fivetran/page.tsx).

## What is stored

The Fivetran API key and secret are used for that one request and are not stored.
They are redacted from logs and error text.

On a hosted org, ContractGate mints a new API key named `Fivetran connector`,
scoped to the org and the selected contracts, and sends it to Fivetran as a
secret. The dashboard user's key is not reused. The raw key is not returned
to the browser.

Self-host that Fivetran can reach keeps auth on and sets
`CONTRACTGATE_SELF_HOSTED`. Orgs still default to `free`. That flag treats
the org as paid for every paid gate, including Connect Fivetran,
`GET /v1/contracts/{id}/passed`, and payload storage. The monthly event cap
is unlimited, the default rate limit is the paid limit unless the key
overrides it, and the passed-event window is 365 days. The connector key is
a normal org-scoped `api_keys` row, so `/passed` accepts it. Hosted
production and staging leave the variable unset. Hosted `free` and
`aisg_free` still receive 403.

`GET /v1/me` keeps the stored `plan`. A self-hosted free org reports
`"plan": "free"`, `"paid_features": true`, `"self_hosted": true`, and
`"monthly_event_limit": null`. See [auth-reference.md](auth-reference.md)
and [self-host-reference.md](self-host-reference.md).

`CONTRACTGATE_DEV_NO_AUTH=1` is only for a local or private process. Do not
put that process on a public URL.

`base_url` is `CANONICAL_API_ORIGIN` (`https://`). Connect returns 500 and does not
mint a key when that variable is unset or empty. It does not fall back to production.

## Request

```
POST /v1/integrations/fivetran/connect
x-api-key: <ContractGate API key>
```

Body max 64 KiB. Required fields are `fivetran_api_key`, `fivetran_api_secret`,
`group_id`, and `contract_ids`. Optional fields are `table_names` and
`destination_schema`.

```json
{
  "fivetran_api_key": "your_fivetran_key",
  "fivetran_api_secret": "your_fivetran_secret",
  "group_id": "projected_sickle",
  "contract_ids": ["427c3304-ead1-4ba7-b7e0-123456789abc"],
  "table_names": "427c3304-ead1-4ba7-b7e0-123456789abc=user_events_427c3304",
  "destination_schema": "analytics"
}
```

| Field | Required | Shape |
|---|---|---|
| `fivetran_api_key` | yes | String, 8–256 ASCII graphic characters, no colon. Paste the key. Do not Base64-encode it. |
| `fivetran_api_secret` | yes | Same length and character rules. A colon is allowed. |
| `group_id` | yes | 1–128 letters, digits, underscores, or hyphens. The destination group `id` from `GET https://api.fivetran.com/v1/groups`, not the group name. |
| `contract_ids` | yes | Array of 1–32 unique UUID strings in the caller's org. |
| `table_names` | no | String, at most 2048 characters. Omit, null, or `""` uses the default name for every contract. Otherwise comma-separated `contract_id=name` pairs. Each id is in `contract_ids` and appears once. A name starts with a lowercase letter or `_` and then uses only lowercase letters, digits, and underscores, at most 63 characters. Names are unique. A contract with no pair still gets the default name. |
| `destination_schema` | no | Omit or null uses `contractgate_` plus 8 hex digits. A value starts with a lowercase letter and then uses only lowercase letters, digits, and underscores, at most 63 characters. |

Fivetran is created with `destination_schema_names: SOURCE_NAMING`, so those names
are stored as declared. `FIVETRAN_NAMING` is not used: it splits letter and digit
boundaries (`user_events_427c3304` would become `user_events_427_c_3304`).

## Default table name

The name is fixed from the contract name and id, so it is known before the first
sync. It is `<slug>_` plus the first 8 hex characters of the contract id: the
eight characters before the first hyphen, lowercased.

The slug keeps ASCII letters and digits only, lowercased. Each run of other
characters becomes one underscore. Leading digits and underscores are
dropped. The slug stops at 54 characters so the full name stays within 63.
A name with no ASCII letters left uses `passed_` plus those same 8 hex characters.
`Café` becomes `caf_<8hex>`.

| Contract name | Contract id | Table |
|---|---|---|
| User Events | `427c3304-ead1-4ba7-b7e0-123456789abc` | `user_events_427c3304` |
| 123 Orders | `427c3304-ead1-4ba7-b7e0-123456789abc` | `orders_427c3304` |
| Café | `427c3304-ead1-4ba7-b7e0-123456789abc` | `caf_427c3304` |
| !!! | `427c3304-ead1-4ba7-b7e0-123456789abc` | `passed_427c3304` |

An explicit `table_names` entry replaces the default for that contract only.

## Response

```json
{
  "connection_id": "conn_id",
  "setup_url": "https://fivetran.com/dashboard/connections/conn_id/status",
  "status": "paused"
}
```

The connection is created paused. `setup_url` is null when Fivetran returns an id
that is not safe to put in a URL. `status` is `setup_failed` when a setup test
reports `FAILED` or `JOB_FAILED`. The scheduled sync is every 6 hours
(`sync_frequency` 360) once you unpause it.

Errors are `{"error","status"}`.

| Status | When | Key minted |
|---|---|---|
| 400 | Body is not a JSON object, a field fails the table above, or the group does not exist. The `error` text starts with `Invalid request: `. An unknown group is `Invalid request: Fivetran group <id> was not found`. A bad schema is `Invalid request: destination_schema must start with a lowercase letter and contain only lowercase letters, digits, and underscores, at most 63 characters`. A bad table name is `Invalid request: table name must start with a letter or underscore and use only lowercase letters, numbers, and underscores`. | no |
| 401 | `Unauthorized: missing or invalid API key` when the ContractGate key is missing and auth is on. `Fivetran rejected the API key` when Fivetran rejects the key or secret. The Fivetran check runs before a key is minted. | no |
| 403 | `Forbidden: Connect Fivetran is a paid-plan feature; upgrade to growth, aisg_pro, or enterprise` on `free` or `aisg_free` when `CONTRACTGATE_SELF_HOSTED` is unset | no |
| 404 | A contract id is not in the caller's org | no |
| 413 | Body over 64 KiB (`Payload too large: Connect Fivetran body exceeds 65536 bytes`) | no |
| 500 | `CANONICAL_API_ORIGIN` is unset, blank, or not an https origin with no path, query, or credentials. Unset text: `CANONICAL_API_ORIGIN is unset. Set it to this deployment's public https API origin before connecting Fivetran.` | no |
| 502 | `Fivetran request failed (<status>)` after the group check | the key for that attempt is revoked; an uploaded package that never became a connection is deleted |

## Cleanup and another connect

Each successful connect inserts one `api_keys` row named `Fivetran connector`
(`allowed_contract_ids` = the selected contracts) and creates one new paused
Fivetran connection. Connect does not reuse or update an existing connection.
A second call mints a second key and a second connection.

Connect once per set of contracts. To change contracts, table names, or schema,
delete the Fivetran connection and revoke the key, then connect again.

Revoke from Account → API Keys (`DELETE /api/keys` with `{"id":"<api key uuid>"}`)
or in the database:

```sql
UPDATE api_keys
SET revoked_at = now()
WHERE org_id = '<org uuid>'
  AND name = 'Fivetran connector'
  AND revoked_at IS NULL
  AND deleted_at IS NULL;
```

The gateway cache accepts a revoked key for up to 60 seconds.

Delete the connection from `setup_url`, or:

```bash
curl -sS -u "$FIVETRAN_KEY:$FIVETRAN_SECRET" -X DELETE \
  "https://api.fivetran.com/v1/connections/$CONNECTION_ID"
```

`$CONNECTION_ID` is `connection_id` from the 200 body. ContractGate does not store it.

## Manual configuration.json

`fivetran deploy --configuration configuration.json` reads these keys. `api_key`
and `contract_ids` are required. `base_url` and `table_names` are optional.
`contract_ids` is a comma-separated string, not a JSON array. An empty
`table_names` uses the connector default `passed_<uuid with hyphens turned into
underscores>`, not the slug name the connect API sends. Set `table_names` when
you want the slug names. The setup test is named Configuration check.

```json
{
  "api_key": "cg_live_0123456789abcdef0123456789abcdef0123456789abcdef",
  "contract_ids": "427c3304-ead1-4ba7-b7e0-123456789abc",
  "base_url": "https://api.datacontractgate.com",
  "table_names": "427c3304-ead1-4ba7-b7e0-123456789abc=user_events_427c3304"
}
```

`base_url` must be https with no path, query, or credentials. Blank uses
`https://api.datacontractgate.com`. `http://` is accepted only for `localhost`
and `127.0.0.1`.

The CLI `--api-key` is Base64 of the Fivetran `key:secret`, not the ContractGate key.
`--destination` is the Fivetran group name, not `group_id`.

## Self-host: org, user, and API key

A fresh `docker compose up` applies `supabase/migrations` on first boot, then
`ops/postgres/seed/099_demo_org.sql`. That seed inserts demo org
`cccccccc-cccc-cccc-cccc-cccccccccccc` on plan `free`. It does not insert a user
or an API key. The `contractgate` CLI does not create orgs, users, or keys.
`POST /api/keys` needs a dashboard session this Postgres does not have.

The compose database is user `cg`, password `cg`, database `contractgate`.
Inserting `auth.users` fires `handle_new_user()`, which inserts `user_profiles`,
an `orgs` row, and an owner `org_memberships` row. The plan is `free` unless
`raw_user_meta_data` contains `"product": "aischemagate"`, which starts
`aisg_free`. This org is not the demo org.

```bash
docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U cg -d contractgate <<'SQL'
INSERT INTO auth.users (id, email, raw_user_meta_data)
VALUES (
  '11111111-1111-1111-1111-111111111111',
  'owner@example.com',
  '{"display_name":"Warehouse"}'::jsonb
);
SQL
```

Mint a key in the shape `src/api_key_auth.rs` uses: raw `cg_live_` plus 48 hex
characters, `key_prefix` the first 12 characters, `key_hash` standard base64 of
SHA-256 of the raw UTF-8 key (44 characters). `pgcrypto` comes from migration 005.
The statement prints the raw key once.

```bash
docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U cg -d contractgate <<'SQL'
WITH minted AS (
  SELECT 'cg_live_' || encode(gen_random_bytes(24), 'hex') AS raw_key
),
inserted AS (
  INSERT INTO api_keys (user_id, org_id, name, key_prefix, key_hash)
  SELECT
    '11111111-1111-1111-1111-111111111111',
    m.org_id,
    'warehouse',
    left(minted.raw_key, 12),
    replace(encode(digest(minted.raw_key, 'sha256'), 'base64'), E'\n', '')
  FROM minted
  JOIN org_memberships m
    ON m.user_id = '11111111-1111-1111-1111-111111111111'
   AND m.deleted_at IS NULL
  RETURNING id, key_prefix
)
SELECT inserted.id, inserted.key_prefix, minted.raw_key
FROM inserted CROSS JOIN minted;
SQL
```

On the gateway service, remove `CONTRACTGATE_DEV_NO_AUTH` (only `1`, `true`, and
`TRUE` turn auth off). Set `CONTRACTGATE_SELF_HOSTED` to `1`, `true`, or `TRUE`.
Set `CANONICAL_API_ORIGIN` to the public https origin in front of the gateway,
then recreate the container. Fivetran cloud cannot call localhost.

## Package

Each Fivetran account gets its own package upload. The ZIP is built in memory from
`vendor/fivetran-connector`, pinned in `vendor/fivetran-connector/PIN` to commit
`3f53aa21f4fd71fcf4092d70aa080a847811993c` of
[nmb-contractgate-fivetran](https://github.com/nightmoose/nmb-contractgate-fivetran).
