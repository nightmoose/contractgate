import { createRequire } from "node:module";
import { ZodJsonSchemaAdapter } from "@tmcp/adapter-zod";
import { isInputRequired, McpServer } from "tmcp";
import * as z from "zod/v4";
import { ConfigError, Gateway, GatewayError } from "./gateway.js";
import {
  deployContract,
  egressValidate,
  getDropCounts,
  getQuarantine,
  inferContract,
  listContracts,
  validateEvents,
} from "./tools.js";

// dist/server.js → ../package.json; package.json always ships in the tarball.
const PKG_VERSION: string = createRequire(import.meta.url)("../package.json").version;

const sample = z.record(z.string(), z.unknown());

export type ServerOpts = {
  gateway?: Gateway;
  env?: NodeJS.ProcessEnv;
};

function jsonResult(data: unknown) {
  return { content: [{ type: "text" as const, text: JSON.stringify(data, null, 2) }] };
}

function errorResult(err: unknown) {
  const text =
    err instanceof ConfigError || err instanceof GatewayError || err instanceof Error
      ? err.message
      : String(err);
  return { content: [{ type: "text" as const, text }], isError: true as const };
}

async function runTool(fn: () => Promise<unknown>) {
  try {
    return jsonResult(await fn());
  } catch (err) {
    // tmcp uses a private throw to pause a handler for client input.
    // These handlers do not elicit; rethrow so a future call is not swallowed.
    if (isInputRequired(err)) throw err;
    return errorResult(err);
  }
}

function gateway(opts: ServerOpts): Gateway {
  return opts.gateway ?? Gateway.fromEnv(opts.env);
}

export function createServer(opts: ServerOpts = {}): McpServer {
  const server = new McpServer(
    {
      name: "contractgate",
      version: PKG_VERSION,
    },
    {
      adapter: new ZodJsonSchemaAdapter(),
      capabilities: {
        tools: { listChanged: true },
        prompts: { listChanged: true },
      },
    },
  );

  server.tool(
    {
      name: "infer_contract",
      title: "Infer a contract from sample events",
      description:
        "Generate draft DataContractGate YAML from real JSON sample events via POST /contracts/infer. Does not persist. Review and write the YAML to contracts/<name>.yaml before deploying.",
      schema: z.object({
        name: z.string().min(1).describe("Contract name, snake_case, e.g. user_events"),
        samples: z
          .array(sample)
          .min(1)
          .describe("1–20 real event objects. Do not invent them."),
        description: z.string().optional().describe("Optional contract description"),
      }),
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
    },
    async (args) => runTool(() => inferContract(gateway(opts), args)),
  );

  server.tool(
    {
      name: "validate_events",
      title: "Validate events against a contract",
      description:
        "Validate events. Pass contract_id to hit POST /v1/ingest/{id} (dry_run defaults true — no audit, quarantine, or usage). Pass yaml_content to hit POST /playground/validate per event (never persists). Exactly one of contract_id or yaml_content. 207/422 responses are returned as data so you can apply each violation's suggestion.",
      schema: z
        .object({
          events: z.array(sample).min(1).describe("Event objects to validate"),
          contract_id: z
            .string()
            .uuid()
            .optional()
            .describe("Deployed contract UUID from deploy_contract"),
          yaml_content: z
            .string()
            .optional()
            .describe("In-flight contract YAML; uses the playground, never persists"),
          dry_run: z
            .boolean()
            .optional()
            .describe("Ingest only. Default true. Set false only after a successful dry run."),
          on_fail: z
            .enum(["quarantine", "drop"])
            .optional()
            .describe("Ingest only. Default quarantine. drop counts a failure and stores neither a quarantine row nor an audit row."),
        })
        .refine((v) => Boolean(v.contract_id) !== Boolean(v.yaml_content), {
          message:
            "Pass exactly one of contract_id (deployed) or yaml_content (playground).",
        }),
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: true },
    },
    async (args) => runTool(() => validateEvents(gateway(opts), args)),
  );

  server.tool(
    {
      name: "deploy_contract",
      title: "Deploy a contract to stable",
      description:
        "POST /contracts/deploy — find-or-create by name, insert YAML as stable, deprecate prior stable. 409 if that (name, version) exists: bump version in the YAML. Refused while quarantine is pending. Save the returned contract_id.",
      schema: z.object({
        name: z.string().min(1).describe("Contract name matching the YAML name: field"),
        yaml_content: z.string().min(1).describe("Full contract YAML"),
        source: z.string().optional().describe("Logical feed name, e.g. app-backend"),
        deployed_by: z.string().optional().describe("Actor recorded on the version row"),
      }),
      annotations: { readOnlyHint: false, destructiveHint: true, openWorldHint: true, idempotentHint: false },
    },
    async (args) => runTool(() => deployContract(gateway(opts), args)),
  );

  server.tool(
    {
      name: "get_quarantine",
      title: "List quarantined events",
      description:
        "GET /quarantine — rejected events for the caller's org, newest first, with violation details.",
      schema: z.object({
        contract_id: z.string().uuid().optional().describe("Restrict to one contract"),
        limit: z.number().int().min(1).max(500).optional().describe("Default 100, max 500"),
        offset: z.number().int().min(0).optional(),
      }),
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
    },
    async (args) => runTool(() => getQuarantine(gateway(opts), args)),
  );

  server.tool(
    {
      name: "list_contracts",
      title: "List contracts",
      description: "GET /contracts — identities the API key can see.",
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
    },
    async () => runTool(() => listContracts(gateway(opts))),
  );

  server.tool(
    {
      name: "egress_validate",
      title: "Validate an outbound payload",
      description:
        "AISchemaGate structured-output check. POST /egress/{contract_id}. dry_run defaults true — no audit row or quarantine. Returns 200/207/422 so you can read min, max, pattern, enum, and compliance_mode violations and run a bounded auto-repair. Forward response.payload only — never the original body after a block or fail.",
      schema: z.object({
        contract_id: z.string().uuid().describe("Deployed contract UUID"),
        events: z.array(sample).min(1).describe("Outbound payload objects to validate"),
        disposition: z
          .enum(["block", "fail", "tag"])
          .optional()
          .describe("Default block. block = drop failing records from payload; fail = reject entire batch; tag = pass through with flags."),
        dry_run: z
          .boolean()
          .optional()
          .describe("Default true. Set false only after a successful dry run — live calls write audit and quarantine rows unless on_fail is drop."),
        on_fail: z
          .enum(["quarantine", "drop"])
          .optional()
          .describe("Default quarantine. drop counts a failure and stores neither a quarantine row nor an audit row. Ignored when the contract locks on_fail."),
        version: z
          .string()
          .optional()
          .describe("Pin a specific contract version. Default: latest stable."),
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: true },
    },
    async (args) => runTool(() => egressValidate(gateway(opts), args)),
  );

  server.tool(
    {
      name: "get_drop_counts",
      title: "List dropped-record counts",
      description:
        "GET /contracts/{id}/drop-counts. Minute buckets of dropped failures by reason, kept 30 days. Counts and failure metadata only — never record contents.",
      schema: z.object({
        contract_id: z.string().uuid().describe("Deployed contract UUID"),
        hours: z
          .number()
          .int()
          .min(1)
          .max(24 * 30)
          .optional()
          .describe("Lookback in hours. Default 24."),
      }),
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
    },
    async (args) => runTool(() => getDropCounts(gateway(opts), args)),
  );

  server.prompt(
    {
      name: "integrate-contractgate",
      title: "Integrate DataContractGate",
      description:
        "Wire DataContractGate into this repo using the official agent playbook: infer a contract from real samples, deploy it, and dry-run ingest.",
    },
    () => ({
      messages: [
        {
          role: "user" as const,
          content: {
            type: "text" as const,
            text: `Read https://app.datacontractgate.com/llm-integration.md and execute it in this repository.

If DataContractGate MCP tools are available, prefer them over curl:
1. infer_contract from 5–20 real sample events found in this repo.
2. Write contracts/<name>.yaml, tighten enums/patterns, do not invent fields.
3. validate_events with yaml_content against a known-good and known-bad event.
4. deploy_contract, save contract_id.
5. validate_events with contract_id and dry_run=true; only then drop dry_run.

CONTRACTGATE_API_KEY must already be in the environment. Never inline the key.`,
          },
        },
      ],
    }),
  );

  return server;
}
