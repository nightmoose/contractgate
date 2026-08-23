import { Gateway } from "./gateway.js";

export type InferArgs = {
  name: string;
  samples: Record<string, unknown>[];
  description?: string;
};

export type ValidateArgs = {
  events: Record<string, unknown>[];
  contract_id?: string;
  yaml_content?: string;
  dry_run?: boolean;
};

export type DeployArgs = {
  name: string;
  yaml_content: string;
  source?: string;
  deployed_by?: string;
};

export type QuarantineArgs = {
  contract_id?: string;
  limit?: number;
  offset?: number;
};

export function xorContractTarget(args: {
  contract_id?: string;
  yaml_content?: string;
}): void {
  const hasId = Boolean(args.contract_id);
  const hasYaml = Boolean(args.yaml_content);
  if (hasId === hasYaml) {
    throw new Error(
      "Pass exactly one of contract_id (deployed contract) or yaml_content (in-flight YAML via playground).",
    );
  }
}

export function inferContract(gw: Gateway, args: InferArgs) {
  return gw.infer({
    name: args.name,
    samples: args.samples,
    description: args.description,
  });
}

export async function validateEvents(gw: Gateway, args: ValidateArgs) {
  xorContractTarget(args);
  if (args.yaml_content) {
    const results = [];
    for (let i = 0; i < args.events.length; i++) {
      const body = await gw.playground(args.yaml_content, args.events[i]);
      results.push({ index: i, ...(typeof body === "object" && body ? body : { body }) });
    }
    return { mode: "playground", persisted: false, results };
  }
  return gw.ingest(args.contract_id!, args.events, {
    dryRun: args.dry_run !== false,
  });
}

export function deployContract(gw: Gateway, args: DeployArgs) {
  return gw.deploy({
    name: args.name,
    yaml_content: args.yaml_content,
    source: args.source,
    deployed_by: args.deployed_by ?? "mcp",
  });
}

export function getQuarantine(gw: Gateway, args: QuarantineArgs) {
  return gw.quarantine({
    contractId: args.contract_id,
    limit: args.limit,
    offset: args.offset,
  });
}

export function listContracts(gw: Gateway) {
  return gw.listContracts();
}
