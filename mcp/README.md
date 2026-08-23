# @contractgate/mcp-server

Official [Model Context Protocol](https://modelcontextprotocol.io) server for
ContractGate. Thin stdio wrapper over the existing HTTP API.

See [`docs/mcp-reference.md`](../docs/mcp-reference.md) for tools, auth, and
the host config snippet.

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

```bash
cd mcp
npm install
npm test
npm run build
```
