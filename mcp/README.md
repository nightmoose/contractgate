# @nightmoose/contractgate-mcp-server

**AISchemaGate** validates and repairs structured LLM output against a contract,
plus **DataContractGate** data contracts. Repair uses the violation fields from
that check. The contract checks values a JSON Schema or Pydantic validation can
leave open.

Official [Model Context Protocol](https://modelcontextprotocol.io) server. Thin
stdio wrapper over the HTTP API at `https://api.datacontractgate.com` (override
with `CONTRACTGATE_BASE_URL`). The dashboard host
`https://app.datacontractgate.com` is not the gateway.

One server, two products, same API key:

- **AISchemaGate** — validates structured LLM output
  (`egress_validate` → `POST /egress/{id}`). Repair uses the violation fields
  from that check (`min`, `max`, `pattern`, `enum`, `compliance_mode`). Forward
  `payload` from the tool result, not the original body.
- **DataContractGate** — infer and deploy data contracts, validate ingest
  (`validate_events` → `POST /v1/ingest/{id}`), including ODCS v3.1.0 and v3.2.0
  contracts, and inspect quarantine (`get_quarantine`). Replay stays on the
  HTTP API (`POST /contracts/{id}/quarantine/replay`).

Registry metadata for the Official MCP Registry is [`server.json`](server.json).
The `title` and `description` lead with AISchemaGate (`description` is the
100-character registry field). The server `name` stays
`io.github.nightmoose/contractgate`. Keywords and the longer summary are under
`_meta`. Smithery config is [`smithery.yaml`](smithery.yaml);
its default `contractgateBaseUrl` is the API gateway. npm keywords, this
README, and [`DIRECTORY_LISTINGS.md`](DIRECTORY_LISTINGS.md) are the directory
listing.

See [`docs/mcp-reference.md`](../docs/mcp-reference.md) for tools and auth.
AISchemaGate app integrations: [`docs/integrations/`](../docs/integrations/).

```json
{
  "mcpServers": {
    "contractgate": {
      "command": "npx",
      "args": ["-y", "@nightmoose/contractgate-mcp-server"],
      "env": {
        "CONTRACTGATE_API_KEY": "${CONTRACTGATE_API_KEY}"
      }
    }
  }
}
```

```bash
cd mcp
npm install
npm test
npm run build
```
