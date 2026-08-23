import { McpServer } from "@modelcontextprotocol/server";
import * as z from "zod/v4";
import { ConfigError, Gateway, GatewayError } from "./gateway.js";
import {
  deployContract,
  getQuarantine,
  inferContract,
  listContracts,
  validateEvents,
} from "./tools.js";

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

function gateway(opts: ServerOpts): Gateway {
  return opts.gateway ?? Gateway.fromEnv(opts.env);
}

export function createServer(opts: ServerOpts = {}): McpServer {
  const server = new McpServer({
    name: "contractgate",
    version: "0.1.0",
  });

  server.registerTool(
    "infer_contract",
    {
      title: "Infer a contract from sample events",
      description:
        "Generate draft ContractGate YAML from real JSON sample events via POST /contracts/infer. Does not persist. Review and write the YAML to contracts/<name>.yaml before deploying.",
      inputSchema: z.object({
        name: z.string().min(1).describe("Contract name, snake_case, e.g. user_events"),
        samples: z
          .array(sample)
          .min(1)
          .describe("1–20 real event objects. Do not invent them."),
        description: z.string().optional().describe("Optional contract description"),
      }),
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
    },
    async (args) => {
      try {
        return jsonResult(await inferContract(gateway(opts), args));
      } catch (err) {
        return errorResult(err);
      }
    },
  );

  server.registerTool(
    "validate_events",
    {
      title: "Validate events against a contract",
      description:
        "Validate events. Pass contract_id to hit POST /v1/ingest/{id} (dry_run defaults true — no audit, quarantine, or usage). Pass yaml_content to hit POST /playground/validate per event (never persists). Exactly one of contract_id or yaml_content. 207/422 responses are returned as data so you can apply each violation's suggestion.",
      inputSchema: z
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
        })
        .refine((v) => Boolean(v.contract_id) !== Boolean(v.yaml_content), {
          message:
            "Pass exactly one of contract_id (deployed) or yaml_content (playground).",
        }),
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: true },
    },
    async (args) => {
      try {
        return jsonResult(await validateEvents(gateway(opts), args));
      } catch (err) {
        return errorResult(err);
      }
    },
  );

  server.registerTool(
    "deploy_contract",
    {
      title: "Deploy a contract to stable",
      description:
        "POST /contracts/deploy — find-or-create by name, insert YAML as stable, deprecate prior stable. 409 if that (name, version) exists: bump version in the YAML. Refused while quarantine is pending. Save the returned contract_id.",
      inputSchema: z.object({
        name: z.string().min(1).describe("Contract name matching the YAML name: field"),
        yaml_content: z.string().min(1).describe("Full contract YAML"),
        source: z.string().optional().describe("Logical feed name, e.g. app-backend"),
        deployed_by: z.string().optional().describe("Actor recorded on the version row"),
      }),
      annotations: { readOnlyHint: false, destructiveHint: true, openWorldHint: true, idempotentHint: false },
    },
    async (args) => {
      try {
        return jsonResult(await deployContract(gateway(opts), args));
      } catch (err) {
        return errorResult(err);
      }
    },
  );

  server.registerTool(
    "get_quarantine",
    {
      title: "List quarantined events",
      description:
        "GET /quarantine — rejected events for the caller's org, newest first, with violation details.",
      inputSchema: z.object({
        contract_id: z.string().uuid().optional().describe("Restrict to one contract"),
        limit: z.number().int().min(1).max(500).optional().describe("Default 100, max 500"),
        offset: z.number().int().min(0).optional(),
      }),
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
    },
    async (args) => {
      try {
        return jsonResult(await getQuarantine(gateway(opts), args));
      } catch (err) {
        return errorResult(err);
      }
    },
  );

  server.registerTool(
    "list_contracts",
    {
      title: "List contracts",
      description: "GET /contracts — identities the API key can see.",
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
    },
    async () => {
      try {
        return jsonResult(await listContracts(gateway(opts)));
      } catch (err) {
        return errorResult(err);
      }
    },
  );

  server.registerPrompt(
    "integrate-contractgate",
    {
      title: "Integrate ContractGate",
      description:
        "Wire ContractGate into this repo using the official agent playbook: infer a contract from real samples, deploy it, and dry-run ingest.",
    },
    () => ({
      messages: [
        {
          role: "user" as const,
          content: {
            type: "text" as const,
            text: `Read https://app.datacontractgate.com/llm-integration.md and execute it in this repository.

If ContractGate MCP tools are available, prefer them over curl:
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
