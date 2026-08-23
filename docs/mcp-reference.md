# ContractGate MCP Server

**RFC-090.** Official Model Context Protocol server for Cursor, Claude Desktop,
Windsurf, VS Code Copilot, Codex, and any other MCP host.

The server is a thin stdio client of the existing gateway. It does not run
validation itself. Auth is the same API key the CLI and playbook already use.

## Install

Add this to the host's MCP config (`~/.cursor/mcp.json`, Claude Desktop
`claude_desktop_config.json`, etc.).

Once `@contractgate/mcp-server` is on npm:

```json
{
  "mcpServers": {
    "contractgate": {
      "command": "npx",
      "args": ["-y", "@contractgate/mcp-server"],
      "env": {
        "CONTRACTGATE_API_KEY": "${CONTRACTGATE_API_KEY}"
      }
    }
  }
}
```

Until then, from a clone (`cd mcp && npm install && npm run build`):

```json
{
  "mcpServers": {
    "contractgate": {
      "command": "node",
      "args": ["<repo>/mcp/dist/index.js"],
      "env": {
        "CONTRACTGATE_API_KEY": "${CONTRACTGATE_API_KEY}"
      }
    }
  }
}
```

Restart the host after editing the config.

## Environment

| Variable | Required | Default |
|---|---|---|
| `CONTRACTGATE_API_KEY` | yes | — |
| `CONTRACTGATE_BASE_URL` | no | `https://app.datacontractgate.com` |

Never put the raw key in the config file. Reference the environment variable
the way your host supports (`${CONTRACTGATE_API_KEY}` in Cursor; Claude Desktop
reads the process environment).

Get a key at <https://app.datacontractgate.com/account>.

## Tools

### `infer_contract`

`POST /contracts/infer`. Draft YAML from real sample events. Does not persist.

| Argument | Type | Required |
|---|---|---|
| `name` | string | yes |
| `samples` | object[] | yes, ≥1 |
| `description` | string | no |

Write the returned `yaml_content` to `contracts/<name>.yaml` and review it
before deploying. Inference is a starting point.

### `validate_events`

Validate events against a deployed contract or against in-flight YAML.

| Argument | Type | Required |
|---|---|---|
| `events` | object[] | yes, ≥1 |
| `contract_id` | uuid | exactly one of `contract_id` / `yaml_content` |
| `yaml_content` | string | exactly one of `contract_id` / `yaml_content` |
| `dry_run` | boolean | no, default `true` |

- `contract_id` → `POST /v1/ingest/{contract_id}`. Default `dry_run=true` (no
  audit row, no quarantine, no metered usage). Set `dry_run=false` only after a
  dry run has passed.
- `yaml_content` → `POST /playground/validate` per event. Never persists.
  `dry_run` is ignored.

`200` / `207` / `422` all return the body. Read `results[].violations` —
entries may include `received`, `expected`, and `suggestion` so you can fix
the producer or the YAML without guessing.

### `deploy_contract`

`POST /contracts/deploy`. Finds-or-creates the contract by `name`, inserts the
YAML as `stable`, deprecates prior stable versions. Refused while quarantine
is pending. `409` if that `(name, version)` already exists — bump `version:`
in the YAML and retry.

| Argument | Type | Required |
|---|---|---|
| `name` | string | yes |
| `yaml_content` | string | yes |
| `source` | string | no |
| `deployed_by` | string | no (defaults to `mcp`) |

Save the returned `contract_id`. It is not a secret.

### `get_quarantine`

`GET /quarantine`. Source quarantine rows for the caller's org, newest first.

| Argument | Type | Required |
|---|---|---|
| `contract_id` | uuid | no |
| `limit` | int | no, default 100, max 500 |
| `offset` | int | no |

### `list_contracts`

`GET /contracts`. Identities the key can see.

## Prompt

`integrate-contractgate` — loads the agent playbook URL
(<https://app.datacontractgate.com/llm-integration.md>) as the instruction to
follow. Use it when wiring ContractGate into a repo for the first time.

## What this server will not do

- Live ingest by default (`validate_events` defaults to dry-run).
- Kafka / Kinesis / billing / collaborator management.
- Invent contract fields that were not in the samples.

Full executable flow without MCP: <https://app.datacontractgate.com/llm-integration.md>.
