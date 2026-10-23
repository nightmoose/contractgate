# @nightmoose/contractgate-mcp-server

Official [Model Context Protocol](https://modelcontextprotocol.io) server for
**DataContractGate** and **AISchemaGate**. Thin stdio wrapper over the HTTP
API at `https://api.datacontractgate.com` (override with `CONTRACTGATE_BASE_URL`).
The dashboard host `https://app.datacontractgate.com` is not the gateway.

One server, two products, same API key:

- **DataContractGate** — infer and deploy data contracts, validate ingest
  (`validate_events` → `POST /v1/ingest/{id}`), including ODCS v3.1 and v3.2
  contracts, and inspect quarantine (`get_quarantine`). Replay stays on the
  HTTP API (`POST /contracts/{id}/quarantine/replay`).
- **AISchemaGate** — runtime validation of LLM structured output
  (`egress_validate` → `POST /egress/{id}`). Violation fields (`min`, `max`,
  `pattern`, `enum`, `compliance_mode`) are what a caller uses for bounded
  auto-repair. Forward `payload` from the tool result, not the original body.

Registry metadata for the Official MCP Registry is [`server.json`](server.json)
(`description` is the 100-character registry field; keywords and the longer
summary are under `_meta`). Smithery config is [`smithery.yaml`](smithery.yaml);
its default `contractgateBaseUrl` is the API gateway. npm keywords and this
README are the directory listing.

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
