# Connect Fivetran

**Status:** Paid plans (`growth`, `aisg_pro`, `enterprise`).

Deploys the public ContractGate connector into the caller's own Fivetran account.
ContractGate is DataContractGate and AISchemaGate sharing one engine. The connector
reads records that passed validation and writes one table per contract.

The dashboard button is on **Contracts**. The same call is `POST /v1/integrations/fivetran/connect`.

## What is stored

The Fivetran API key and secret are used for that one request and are not stored.
They are redacted from logs and error text.

ContractGate mints a new API key named `Fivetran connector`, scoped to the org and
the selected contracts, and sends it to Fivetran as a secret. The dashboard user's
key is not reused. The raw key is not returned to the browser.

`base_url` is `CANONICAL_API_ORIGIN` (`https://`). Connect returns 500 and does not
mint a key when that variable is unset or empty. It does not fall back to production.

## Request

```
POST /v1/integrations/fivetran/connect
```

```json
{
  "fivetran_api_key": "<fivetran key>",
  "fivetran_api_secret": "<fivetran secret>",
  "group_id": "<destination group id>",
  "contract_ids": ["<contract uuid>"],
  "table_names": "<contract uuid>=passed_orders",
  "destination_schema": "analytics"
}
```

`table_names` is optional. Each entry is `contract_id=name`, separated by commas.
A missing name becomes `<slug>_<8 hex digits of the contract id>`. The slug is the
contract name in lowercase, with other characters turned into underscores. For
contract `User Events` and id `427c3304-ead1-4ba7-b7e0-123456789abc` the table is
`user_events_427c3304`. A contract name with no letters becomes
`passed_<those 8 hex digits>`. An explicit `table_names` entry replaces that
default for that contract. Names are lowercase letters, digits, and underscores,
at most 63 characters, and must be unique across the selected contracts.

Fivetran is created with `destination_schema_names: SOURCE_NAMING`, so those names
are stored as declared. `FIVETRAN_NAMING` is not used: it splits letter and digit
boundaries (`user_events_427c3304` would become `user_events_427_c_3304`).

`destination_schema` is optional. When set, it is the connection schema: a
lowercase letter, then lowercase letters, digits, and underscores, at most 63
characters. Anything else is `400`. When omitted, the schema is
`contractgate_<8 hex>`.

`contract_ids` is 1–32 UUIDs that belong to the caller's org.

## Response

```json
{
  "connection_id": "conn_id",
  "setup_url": "https://fivetran.com/dashboard/connections/conn_id/status",
  "status": "paused"
}
```

The connection is created paused. `setup_url` is omitted when Fivetran returns an id
that is not safe to put in a URL.

Free and `aisg_free` plans receive `403`. A rejected Fivetran key receives `401`
before a ContractGate key is minted. An unknown `group_id` receives `400`
(`Fivetran group <id> was not found`). Unknown contract ids receive `404`.
An unset `CANONICAL_API_ORIGIN` receives `500` and no key is minted.

## Package

Each Fivetran account gets its own package upload. The ZIP is built in memory from
`vendor/fivetran-connector`, pinned in `vendor/fivetran-connector/PIN` to commit
`3f53aa21f4fd71fcf4092d70aa080a847811993c` of
[nmb-contractgate-fivetran](https://github.com/nightmoose/nmb-contractgate-fivetran).
