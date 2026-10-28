# Directory listings

Ready-to-paste copy for the MCP server. The registry `name` stays
`io.github.nightmoose/contractgate`. The npm package is
`@nightmoose/contractgate-mcp-server`.

Registry `description` in [`server.json`](server.json) (100-character limit):

> AISchemaGate validates and repairs structured LLM output against a contract, plus DataContractGate.

Install command, matching the `name` in [`package.json`](package.json):

```bash
npx -y @nightmoose/contractgate-mcp-server
```

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

Shared longer description, for directories that take one description field:

> AISchemaGate validates and repairs structured LLM output against a contract, plus DataContractGate data contracts. Repair uses the violation fields from that check (min, max, pattern, enum, compliance_mode). The contract checks values a JSON Schema or Pydantic validation can leave open. DataContractGate infers and deploys contracts, validates ingest, including ODCS v3.1.0 and v3.2.0, and lists quarantine.

## awesome-mcp-servers

Paste this single line under the heading `### 💻 Developer Tools`, in
alphabetical order by the link text. `📇` is TypeScript, `☁️` is a cloud
service (the server calls the remote API), and `🍎` `🪟` `🐧` are macOS,
Windows, and Linux.

```
- [nightmoose/contractgate](https://github.com/nightmoose/contractgate) 📇 ☁️ 🍎 🪟 🐧 - AISchemaGate validates and repairs structured LLM output against a contract, plus DataContractGate data contracts. Repair uses the violation fields from that check. The contract checks values a JSON Schema or Pydantic validation can leave open. `npx -y @nightmoose/contractgate-mcp-server`
```

## Glama

On the servers page, **Add MCP Server** asks for these fields:

- **GitHub repository URL:** https://github.com/nightmoose/contractgate
- **Display name:** AISchemaGate and DataContractGate
- **Short description:** AISchemaGate validates and repairs structured LLM output against a contract, plus DataContractGate data contracts. Repair uses the violation fields from that check (min, max, pattern, enum, compliance_mode). The contract checks values a JSON Schema or Pydantic validation can leave open. DataContractGate infers and deploys contracts, validates ingest, including ODCS v3.1.0 and v3.2.0, and lists quarantine.

## Cursor Directory

`https://cursor.directory/mcp/new` redirects to
`https://cursor.directory/plugins/new`. Use the **Manual** tab. An auto scan
of this monorepo reads the root `.mcp.json` (a different server) and
`plugins/contractgate/.mcp.json`, which still pins
`@nightmoose/contractgate-mcp-server@0.2.0`.

- **Name:** AISchemaGate and DataContractGate
- **Description:** AISchemaGate validates and repairs structured LLM output against a contract, plus DataContractGate data contracts. Repair uses the violation fields from that check (min, max, pattern, enum, compliance_mode). The contract checks values a JSON Schema or Pydantic validation can leave open. DataContractGate infers and deploys contracts, validates ingest, including ODCS v3.1.0 and v3.2.0, and lists quarantine.
- **Repository URL:** https://github.com/nightmoose/contractgate
- **Homepage:** https://app.datacontractgate.com/mcp-reference.md
- **Keywords:** aischemagate, llm, structured-output, json-schema, pydantic-validation, llm-guardrails, datacontractgate, odcs, quarantine
- **Component type:** MCP Server
- **Component name:** contractgate
- **Component description:** AISchemaGate validates structured LLM output against a contract and returns violation fields for repair. DataContractGate validates ingest and lists quarantine.
- **Component content:**

```json
{
  "command": "npx",
  "args": ["-y", "@nightmoose/contractgate-mcp-server"],
  "env": {
    "CONTRACTGATE_API_KEY": "${CONTRACTGATE_API_KEY}"
  }
}
```

## Smithery

This server is stdio. Local publish is an MCPB bundle. URL publish on
https://smithery.ai/new is for a public Streamable HTTP endpoint. Server
metadata fields:

- **displayName:** AISchemaGate and DataContractGate
- **description:** AISchemaGate validates and repairs structured LLM output against a contract, plus DataContractGate data contracts. Repair uses the violation fields from that check (min, max, pattern, enum, compliance_mode). The contract checks values a JSON Schema or Pydantic validation can leave open. DataContractGate infers and deploys contracts, validates ingest, including ODCS v3.1.0 and v3.2.0, and lists quarantine.
- **homepage:** https://app.datacontractgate.com/mcp-reference.md
- **repositoryUrl:** https://github.com/nightmoose/contractgate

The stdio command in [`smithery.yaml`](smithery.yaml) is `npx` with args
`["-y", "@nightmoose/contractgate-mcp-server"]`.
