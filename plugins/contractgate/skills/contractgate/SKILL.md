---
name: contractgate
description: >
  Wire ContractGate semantic contracts into a repo. Use when adding data
  contracts, validating events at ingest, Kafka/Redpanda/HTTP producers,
  quarantine, or MCP tools infer_contract / validate_events / deploy_contract.
---

Follow the playbook at https://app.datacontractgate.com/llm-integration.md end to end. Do not invent a parallel procedure.

**MCP first.** If the ContractGate MCP server is connected, use `infer_contract`, `validate_events`, `deploy_contract`, `get_quarantine`, `list_contracts` instead of curl. Setup: https://app.datacontractgate.com/mcp-reference.md

**Rules**

- Read `CONTRACTGATE_API_KEY` from the environment. Never write the raw key into source, `.env` that is tracked, a commit, or chat.
- If the key is unset, stop and ask. Do not invent a key.
- Infer from 5–20 **real** sample events in the repo. Do not fabricate samples.
- Dry-run (`validate_events` with `dry_run=true`, default) before deploy. Deploy only after a dry run passes.
- Write the YAML to `contracts/<name>.yaml` and review it before `deploy_contract`.

**Streaming (optional)**

- Kafka Connect SMT: `nightmoose/contractgate/confluent-connector`
- Redpanda Connect YAML: `docs/examples/redpanda-connect/contractgate.yaml` in this product repo
