# Vercel AI SDK + AISchemaGate

Validate structured model output against an AISchemaGate contract at generation time. JSON Schema (and `Output.object`) checks shape. The contract checks values: `min`, `max`, `pattern`, `enum`, and `compliance_mode`. When those fail, the middleware can re-prompt with the violations and return the egress payload.

The helper is `aischemagateGuardrail` in `@nightmoose/aischemagate`. It implements `LanguageModelV4Middleware` (`specificationVersion: 'v4'`, `wrapGenerate`) and does not depend on the `ai` package. Install `ai` in the app that calls `wrapLanguageModel`.

```bash
npm install @nightmoose/aischemagate ai zod
```

```ts
import { generateText, Output, wrapLanguageModel, gateway } from 'ai';
import { aischemagateGuardrail } from '@nightmoose/aischemagate';
import { z } from 'zod';

const model = wrapLanguageModel({
  model: gateway('openai/gpt-5.4'),
  middleware: aischemagateGuardrail({
    apiKey: process.env.CONTRACTGATE_API_KEY!,
    contractId: process.env.AISG_CONTRACT_ID!, // UUID or contract name
    autoRepair: true,
    maxRetries: 2, // extra model calls; hard cap is 5
  }),
});

const result = await generateText({
  model,
  output: Output.object({
    schema: z.object({
      total: z.number(),
      currency: z.string(),
    }),
  }),
  prompt: 'Extract the invoice total and currency as JSON.',
});

// result.output was parsed from the egress payload, not the raw model text.
console.log(result.output);
```

## What the middleware does

1. `wrapGenerate` reads a JSON object from text content. A markdown fence is stripped. If the text is not JSON, a tool-call `input` or `args` object is used.
2. It calls `POST /egress/{contract_id}` with that object as a one-event batch.
3. A passing call rewrites the text (or tool-call input) to `JSON.stringify` of `response.payload[0]`. Forward that value. Declared transforms such as email masking have already been applied.
4. `autoRepair` re-prompts only when a violation kind is semantic:

   | Contract rule | Violation `kind` |
   |---|---|
   | `min` / `max` | `range_violation` |
   | `pattern` | `pattern_mismatch` |
   | `enum` | `enum_violation` |
   | `compliance_mode: true` | `undeclared_field` |

   A `type_mismatch` does not start a repair loop. The repair prompt lists each field, kind, and message, and is appended to `params.prompt` (an array of messages). `maxRetries` is the number of extra model calls after the first, clamped to 5.
5. If the payload still fails, `wrapGenerate` throws `AiSchemaGateRejectedError` (`violations`, `attempts`, `payload`). Set `throwIfRejected: false` to return the model result instead.

Streaming is not wrapped. Call `generateText`, not `streamText`, when the next step needs a validated object.

## Contract

```yaml
version: "1.0.0"
name: invoice_extract
description: Invoice fields the model is allowed to emit
compliance_mode: true
ontology:
  entities:
    - name: total
      type: float
      required: true
      min: 0
    - name: currency
      type: string
      required: true
      enum: ["USD", "EUR", "GBP"]
```

`{ "total": -5, "currency": "usd" }` is valid JSON and matches the Zod object above. Egress rejects it: `total` is under `min`, and `usd` is not in the enum. The repair prompt names both violations. A later call that returns `{ "total": 10, "currency": "USD" }` is what the middleware writes back.

## Already-parsed JSON

`validateStructuredOutput` runs the same egress check without a model. Pass a `repair` callback when you want the bounded loop yourself:

```ts
import { validateStructuredOutput } from '@nightmoose/aischemagate';

const outcome = await validateStructuredOutput(
  {
    apiKey: process.env.CONTRACTGATE_API_KEY!,
    contractId: process.env.AISG_CONTRACT_ID!,
    autoRepair: true,
    maxRetries: 2,
  },
  modelJson,
  async (_payload, violations) => askModelToFix(violations),
);

if (outcome.ok) save(outcome.payload);
```

`dryRun: true` skips the audit row and quarantine. Leave it false when this call is the system of record.
