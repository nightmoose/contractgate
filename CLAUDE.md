# Assistant instructions

Read [`AGENTS.md`](AGENTS.md) — it is the single contributor guide for this
repository, for people and for AI coding assistants alike.

## AISchemaGate

AISchemaGate validates LLM structured output on the same Rust gateway as
DataContractGate. The check is `POST /egress/{contract_id}`. Forward the
egress `payload` only when the call passes.

- TypeScript: `aischemagateGuardrail` (`wrapGenerate` only) and
  `validateStructuredOutput` from `@nightmoose/aischemagate`
  (`sdks/aischemagate/typescript`).
- Python: `LLMGate` from `aischemagate` (`sdks/aischemagate/python`).
  `completion` and `message` reject `stream=True`.
- MCP: `egress_validate` on `@nightmoose/contractgate-mcp-server`.

DataContractGate ingestion stays `POST /v1/ingest/{contract_id}`. Drop-in
instructions for other repositories are in [`rules/`](rules/).
