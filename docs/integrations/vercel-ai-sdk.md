# Vercel AI SDK — AISchemaGate

Validate structured output from the [Vercel AI SDK](https://sdk.vercel.ai) with
**AISchemaGate** before application code reads the object. The helper lives in
`@nightmoose/aischemagate` and does not depend on the `ai` package.
`createAISchemaGateMiddleware` is structurally a `LanguageModelV4Middleware`
(`specificationVersion: 'v4'`, `wrapGenerate`). `validateOrRepairObject` wraps
`generateObject`, `generateText({ output: Output.object(...) })`, or any async
function that returns JSON.

Both call `client.egress`, which is `POST /egress/{contract_id}`. The HTTP
client has no separate `validate` method. On a semantic failure (`min`, `max`,
`pattern`, `enum`, or `compliance_mode` / `undeclared_field`) the helper builds
a repair prompt from the violation's `field`, `kind`, `message`, `expected`,
`received`, and `suggestion`, and tries again up to `maxRetries` (default 2,
hard cap 5).

Forward the returned object only. After a pass it is the egress payload, which
includes any PII transform the contract declared.

## Install

```bash
npm install @nightmoose/aischemagate ai
```

```ts
import { Client } from '@nightmoose/aischemagate';

const client = new Client({
  baseUrl: process.env.CONTRACTGATE_BASE_URL ?? 'https://api.datacontractgate.com',
  apiKey: process.env.CONTRACTGATE_API_KEY!, // cg_live_...
});
```

Deploy the contract first (dashboard or `POST /contracts/deploy`) and keep its
UUID. The same key and gateway serve DataContractGate ingest and AISchemaGate
egress.

## Middleware

`wrapLanguageModel` from the Vercel AI SDK accepts the middleware directly.

```ts
import { generateObject, wrapLanguageModel } from 'ai';
import { openai } from '@ai-sdk/openai';
import { z } from 'zod';
import { createAISchemaGateMiddleware } from '@nightmoose/aischemagate';

const model = wrapLanguageModel({
  model: openai('gpt-4o-mini'),
  middleware: createAISchemaGateMiddleware({
    client,
    contractId: process.env.CONTRACTGATE_CONTRACT_ID!,
    maxRetries: 2,
    // dryRun: true while iterating. Omit it to write audit rows.
  }),
});

const { object } = await generateObject({
  model,
  schema: z.object({
    currency: z.enum(['USD', 'EUR', 'GBP']),
    amount: z.number(),
  }),
  prompt: 'Extract the invoice JSON.',
});
```

On success the middleware replaces the model's JSON text (or tool-call input)
with the sanitized egress payload, so `generateObject` parses that object.
When every attempt still fails it throws `AISchemaGateRepairError`
(`violations`, `attempts`, `repairPrompts`). The default disposition is
`fail`, so one bad object does not look like a partial success.

## `validateOrRepairObject`

Use this when you already call `generateObject` or `generateText` yourself, or
when the generator is not a language-model middleware.

```ts
import { generateObject, Output, generateText } from 'ai';
import { openai } from '@ai-sdk/openai';
import { z } from 'zod';
import { validateOrRepairObject } from '@nightmoose/aischemagate';

const schema = z.object({
  currency: z.enum(['USD', 'EUR', 'GBP']),
  amount: z.number().min(0).max(100000),
});

const gated = await validateOrRepairObject(client, {
  contractId: process.env.CONTRACTGATE_CONTRACT_ID!,
  maxRetries: 2,
  generate: async ({ repairPrompt }) => {
    const { object } = await generateObject({
      model: openai('gpt-4o-mini'),
      schema,
      prompt: repairPrompt ?? 'Extract the invoice as JSON.',
    });
    return object;
  },
});

if (!gated.ok) {
  // gated.violations is the last AISchemaGate response. Do not use gated.object.
  throw new Error(gated.violations.map((v) => `${v.field}: ${v.message}`).join('\n'));
}
forward(gated.object);
```

The same helper accepts a `generateText` result. A result shaped
`{ output, text, finishReason }` is unwrapped to `output`:

```ts
const gatedFromText = await validateOrRepairObject(client, {
  contractId: process.env.CONTRACTGATE_CONTRACT_ID!,
  maxRetries: 2,
  generate: async ({ repairPrompt }) =>
    generateText({
      model: openai('gpt-4o-mini'),
      prompt: repairPrompt ?? 'Extract the invoice as JSON.',
      output: Output.object({ schema }),
    }),
});
```

`generate` may also return a JSON string or a plain object. Attempt 1 is called
with `repairPrompt === undefined`. Later attempts receive the prompt and the
structured `violations` from the previous egress response.

Exhaustion returns `{ ok: false, attempts, violations, repairPrompts }`. It does
not throw. `attempts` is `maxRetries + 1`.

## Contract

Semantic checks belong in the contract, not only in the Zod schema. Zod can
guarantee JSON shape. AISchemaGate still enforces `min`, `max`, `pattern`,
`enum`, and `compliance_mode`.

```yaml
version: "1.0.0"
name: invoice_extract
description: LLM invoice extraction
compliance_mode: true
ontology:
  entities:
    - name: currency
      type: string
      required: true
      enum: ["USD", "EUR", "GBP"]
    - name: amount
      type: float
      required: true
      min: 0
      max: 100000
```

`compliance_mode: true` rejects fields the model invented. The repair prompt
tells the model to drop `undeclared_field` entries.

## What is forwarded

`POST /egress/{contract_id}` returns `payload`. The helpers substitute that
payload for the model JSON. Do not forward the original model text after a
`block` or `fail` disposition.
