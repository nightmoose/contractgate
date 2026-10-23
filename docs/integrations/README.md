# Integration guides

Copy-paste guides for the two products that share this gateway:

- **DataContractGate** validates events on the way in (`POST /v1/ingest/{contract_id}`), quarantines failures, and replays them after a fix. Recipes: [`examples/recipes/`](../../examples/recipes/).
- **AISchemaGate** validates structured model output on the way out (`POST /egress/{contract_id}`), including `min`, `max`, `pattern`, `enum`, and `compliance_mode`.

| Guide | What it wires |
|---|---|
| [Vercel AI SDK](vercel-ai-sdk.md) | `LanguageModelV4Middleware` / `wrapGenerate`, structured output, bounded auto-repair |
| [OpenAI, Anthropic, Pydantic, Instructor](openai-anthropic-structured-output.md) | Semantic value drift that JSON Schema accepts |
| [LangGraph tool-call gate](langgraph-tool-gate.md) | `compliance_mode: true` on tool arguments and state transitions |

The API host for every example is `https://api.datacontractgate.com`. Keys are `cg_live_...` from the dashboard.
