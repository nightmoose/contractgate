# RFC-090 — Official MCP server (stdio)

**Status:** Accepted
**Date:** 2026-08-23
**Branch:** nightly-maintenance-2026-08-23-rfc090
**Depends on:** RFC-089 (agent playbook), RFC-028 (deploy), RFC-076 (local test), RFC-081 (quarantine list)

---

## Problem

RFC-089 made ContractGate paste-installable: an agent that fetches
`/llm-integration.md` can infer, deploy, and wire ingest over HTTP. That still
asks the agent to construct `curl`/`jq` pipelines. Hosts that speak MCP
(Cursor, Claude Desktop, Windsurf, VS Code Copilot, Codex) prefer typed tools.

RFC-089 listed this as a follow-up. The playbook and `llms-full.txt` are the
prerequisite content; this RFC wraps them.

## Goal

A stdio MCP server an agent host can launch with:

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

Done means: with `CONTRACTGATE_API_KEY` set, an agent can infer a contract from
samples, dry-run events against it, deploy YAML to stable, and list quarantine
— without writing a shell pipeline.

## Non-goals

- **Remote Streamable HTTP / OAuth.** Local stdio + API-key env. Hosted MCP is
  a later RFC once SSO exists.
- **Embedding MCP in `contractgate-server`.** Agent-protocol churn stays off
  the validation hot path.
- **Mirroring `/openapi.json` as tools.** Agents get a short workflow set, not
  every route.
- **`cg mcp` Rust subcommand.** TypeScript package first (`npx` discovery).
- **Claude marketplace plugin.** Distribution follows the package.
- **Changing the validation engine.** Tools are a thin HTTP client.

## Design

### Package

`mcp/` — `@contractgate/mcp-server`, official TypeScript SDK
(`@modelcontextprotocol/server`, 2026-07-28 spec), stdio via `serveStdio`.

Auth: `CONTRACTGATE_API_KEY` (required) and optional `CONTRACTGATE_BASE_URL`
(default `https://app.datacontractgate.com`). Same key and `X-Api-Key` header
as the CLI and playbook. Per-key `allowed_contract_ids` (RFC-065) still scopes
writes.

### Tools

| Tool | Gateway | Notes |
|---|---|---|
| `infer_contract` | `POST /contracts/infer` | Draft YAML. Does not persist. |
| `validate_events` | `POST /v1/ingest/{id}` or `POST /playground/validate` | `dry_run` defaults **true**. YAML path is playground (never persists). `207`/`422` return the body so agents can self-heal from `suggestion`. |
| `deploy_contract` | `POST /contracts/deploy` | Promotes to `stable`. `destructiveHint`. |
| `get_quarantine` | `GET /quarantine` | Read-only. |
| `list_contracts` | `GET /contracts` | Read-only. Agents need ids after infer. |

Prompt: `integrate-contractgate` — points at the playbook URL.

No Kafka, Kinesis, billing, collab, or live-ingest-by-default tools.

### Docs + drift

- `docs/mcp-reference.md` — user-facing; served raw at `/mcp-reference.md`.
- `docs/llms.txt` + `llms-full.txt` bundle include it.
- `tests/llm_docs_test.rs` asserts every `METHOD /path` in the reference exists
  in `src/main.rs`.

## Implementation checklist

1. `mcp/` package: gateway client, tools, stdio entry, unit tests with mocked fetch.
2. `docs/mcp-reference.md` + RFC.
3. Wire `sync-llm-docs.mjs`, `llms.txt`, playbook §0 note, README snippet, docs card, agent-rule one-liners.
4. CI job: `npm test` + `tsc` in `mcp/`.
5. `cargo test` (drift gate) + `mcp` tests.

## Success

An agent host with the snippet above can complete the RFC-089 flow using only
the five tools. No engine regression.
