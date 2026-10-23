# @nightmoose/aischemagate

**AISchemaGate** — thin alias package over
[`@nightmoose/contractgate-sdk`](https://www.npmjs.com/package/@nightmoose/contractgate-sdk)
for LLM structured-output validation. Same Rust engine, same
`cg_live_...` API keys, same tenant service, same backend as
[DataContractGate](https://datacontractgate.com).

```bash
npm install @nightmoose/aischemagate
```

## Why this package exists

The underlying engine, SDK, and API are **DataContractGate** — one validator,
one gateway. `@nightmoose/aischemagate` is the LLM / AI-agent-shaped
front door: it re-exports the SDK under a name that reads correctly from
the LLM-output-validation angle. Use whichever import name matches your
mental model; the behavior is identical.

See:

- `aischemagate.com` — product site
- `aischemagate.dev` — developer documentation
- RFC-094 (`docs/rfcs/094-sdk-llm-output-guard.md`) — SDK design

## 15-line quickstart

```ts
import { LLMGate } from '@nightmoose/aischemagate';
import OpenAI from 'openai';

const gate = new LLMGate({
  apiKey: process.env.CONTRACTGATE_API_KEY!,
  contractId: 'extract_invoice_v1',
  llm: new OpenAI(),
});

const result = await gate.completion({
  model: 'gpt-4o-mini',
  messages: [{ role: 'user', content: 'Extract invoice JSON from: ...' }],
  response_format: { type: 'json_object' },
  autoRepair: true,
});

if (result.ok) forwardDownstream(result.payload);  // egress-sanitized
```

For Anthropic, call `gate.message({ model: 'claude-sonnet-4-5', ... })`.
For vLLM / Ollama OpenAI-compat, call `gate.completion(...)` unchanged.

## Vercel AI SDK

`createAISchemaGateMiddleware` is structurally a `LanguageModelV4Middleware`
(`wrapGenerate`). `validateOrRepairObject` wraps `generateObject`,
`generateText({ output: Output.object(...) })`, or any async JSON function.
Neither imports the `ai` package. Both validate with `client.egress`
(`POST /egress/{contract_id}`) and, on a semantic failure (`min`, `max`,
`pattern`, `enum`, `compliance_mode`), send the violations back to the
model up to `maxRetries` (default 2, hard cap 5).

```ts
import { createAISchemaGateMiddleware, validateOrRepairObject } from '@nightmoose/aischemagate';
```

See [docs/integrations/vercel-ai-sdk.md](../../../docs/integrations/vercel-ai-sdk.md).

## Running tests locally

Tests resolve the `@nightmoose/contractgate-sdk` dependency through
`npm install`. From this directory:

```bash
npm install
npm run build        # one-time, or after changes
npm test
```

## What's new

See [CHANGELOG.md](CHANGELOG.md) and the parent
[`@nightmoose/contractgate-sdk` CHANGELOG](../../typescript/CHANGELOG.md) —
the alias version tracks initial release only; feature work is versioned
on `contractgate-sdk`.
