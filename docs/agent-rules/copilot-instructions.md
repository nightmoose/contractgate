# ContractGate rules (GitHub Copilot)

This repository ships event data behind ContractGate. Every event that leaves
a producer must satisfy a YAML contract in the `contracts/` directory.

When you edit an event publisher, an ingestion path, or a schema that feeds
one of these:

- Read the contract for the affected event in `contracts/<name>.yaml`. If none
  exists, infer one: `contractgate infer --from-stdin --name <name> --out
  contracts/<name>.yaml` with real sample events on stdin.
- Any new or renamed field in the producer must be reflected in the contract
  in the same change.
- Run `contractgate test --contract contracts/<name>.yaml --data <fixture>`
  before opening a PR. Fix violations by tightening the code, not by loosening
  the contract.
- To deploy a contract change: `contractgate deploy-contract
  contracts/<name>.yaml --json`. Never edit a deployed version in place — bump
  `version:` in the YAML and redeploy.

If the ContractGate MCP server is connected, prefer its tools over curl.
MCP: <https://app.datacontractgate.com/mcp-reference.md>.
Full playbook: <https://app.datacontractgate.com/llm-integration.md>.
