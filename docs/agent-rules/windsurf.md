# ContractGate rules (Windsurf)

Trigger: any change under `contracts/`, or to files whose name contains
`producer`, `publisher`, or `ingest`.

This repository ships event data behind ContractGate. Every event that leaves
a producer must satisfy a YAML contract in `contracts/`.

## Before editing an event publisher or ingestion path

1. Read the contract for the affected event in `contracts/<name>.yaml`. If none
   exists, infer one first: `contractgate infer --from-stdin --name <name>
   --out contracts/<name>.yaml` (feed real sample events on stdin).
2. Any new or renamed field in the producer must be reflected in the contract
   in the same change.

## Before committing

Run `contractgate test --contract contracts/<name>.yaml --data <fixture>` on
representative fixtures. Fix violations by tightening the code, not by
loosening the contract.

## Deploying a contract change

Use `contractgate deploy-contract contracts/<name>.yaml --json`. Never edit a
deployed version in place — bump `version:` in the YAML and redeploy. If the
ContractGate MCP server is connected, prefer its tools over curl. Full
playbook: <https://app.datacontractgate.com/llm-integration.md>. MCP:
<https://app.datacontractgate.com/mcp-reference.md>.
