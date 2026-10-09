const { baseUrl } = require("../lib/base_url");
const { resolveContractId } = require("../lib/contracts");
const { asBool, parseRecord, summarize } = require("../lib/record");

const SAMPLE = {
  passed: true,
  contract_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
  contract_version: "1.0.0",
  quarantine_id: null,
  violations_summary: "",
  payload: { email: "ada@example.com", status: "new" },
  record: { email: "ada@example.com", status: "new" },
  dry_run: false,
};

function errorMessage(body, status) {
  if (body && typeof body.detail === "string" && body.detail) return body.detail;
  if (body && typeof body.error === "string" && body.error) return body.error;
  return `ContractGate returned HTTP ${status}.`;
}

function asJson(response) {
  const data = response.data;
  if (data && typeof data === "object") return data;
  if (typeof data === "string" && data) {
    try {
      return JSON.parse(data);
    } catch {
      return { detail: data };
    }
  }
  return {};
}

const perform = async (z, bundle) => {
  // "Test step" in the Zap editor. It used to return SAMPLE without calling the
  // gateway, which read as a passing test. Now it validates for real, as a dry
  // run (no quarantine row, no usage), and returns a failure instead of
  // halting so the editor shows the violations.
  const editorTest = Boolean(bundle.meta && bundle.meta.isLoadingSample);

  const record = parseRecord(bundle.inputData.record);
  if (!bundle.inputData.contract_id) {
    throw new z.errors.Error("Pick a contract.", "MissingContract", 400);
  }
  const contractId = await resolveContractId(z, bundle, bundle.inputData.contract_id);

  const dryRun = editorTest || asBool(bundle.inputData.dry_run, false);
  const haltOnFailure = !editorTest && asBool(bundle.inputData.halt_on_failure, true);
  const version = bundle.inputData.version ? String(bundle.inputData.version).trim() : "";

  const params = new URLSearchParams();
  if (dryRun) params.set("dry_run", "true");
  if (version) params.set("version", version);
  const qs = params.toString();
  const url = `${baseUrl(bundle.authData)}/v1/ingest/${encodeURIComponent(contractId)}${qs ? `?${qs}` : ""}`;

  const headers = { "Content-Type": "application/json" };
  const idempotencyKey = bundle.inputData.idempotency_key
    ? String(bundle.inputData.idempotency_key).trim()
    : "";
  if (idempotencyKey) headers["Idempotency-Key"] = idempotencyKey;

  const response = await z.request({
    method: "POST",
    url,
    body: record,
    headers,
    skipThrowForStatus: true,
  });

  if (response.status === 401 || response.status === 403) {
    throw new z.errors.ExpiredAuthError(
      "ContractGate rejected this API key. Create a key at https://app.datacontractgate.com/account and reconnect.",
    );
  }
  if (response.status === 429) {
    const Retry = z.errors.ThrottledError;
    if (typeof Retry === "function") {
      throw new Retry("ContractGate rate limit hit. Zapier will retry this task.", 2);
    }
    throw new z.errors.Error(
      "ContractGate rate limit hit. Zapier will retry this task.",
      "RateLimit",
      429,
    );
  }

  const body = asJson(response);
  if (response.status === 409) {
    throw new z.errors.Error(
      "This contract has no stable version yet. In ContractGate, open the contract and deploy a version to stable, or put a draft version number in Contract Version.",
      "NoStableVersion",
      409,
    );
  }
  if (response.status !== 200 && response.status !== 207 && response.status !== 422) {
    throw new z.errors.Error(errorMessage(body, response.status), "ContractGateError", response.status);
  }

  const result = Array.isArray(body.results) ? body.results[0] : null;
  if (!result) {
    throw new z.errors.Error(errorMessage(body, response.status), "ContractGateError", response.status);
  }

  const passed = result.passed === true;
  const violationsSummary = summarize(result.violations);
  const shaped = {
    passed,
    contract_id: contractId,
    contract_version: result.contract_version || body.resolved_version || null,
    quarantine_id: result.quarantine_id || null,
    violations_summary: violationsSummary,
    // Null on failure so a later step cannot write the rejected record by mapping Payload.
    payload: passed ? (result.transformed_event ?? record) : null,
    record,
    dry_run: body.dry_run === true,
  };

  if (!passed && haltOnFailure) {
    // Quarantine id first: Zapier's run view truncates long halt messages.
    const where = shaped.quarantine_id
      ? `Quarantined as ${shaped.quarantine_id}.`
      : "Dry run: nothing was quarantined.";
    // Replay needs a stored body, which only paid plans with storage on keep
    // (RFC-086), so do not promise it here.
    const replay = shaped.quarantine_id
      ? " See it under Quarantine in ContractGate. Zapier will not retry this task."
      : "";
    // HaltedError stops the Zap without counting as an error that turns the Zap off.
    throw new z.errors.HaltedError(
      `Contract rejected the record. ${where} ${violationsSummary || "See the contract."}${replay}`,
    );
  }

  return shaped;
};

module.exports = {
  key: "validate_record",
  noun: "Record",
  display: {
    label: "Validate Record",
    description:
      "Checks one record against a ContractGate contract. A failure is quarantined and the Zap stops. A pass continues with the payload the next step should write.",
  },
  operation: {
    cleanInputData: false,
    perform,
    inputFields: [
      {
        key: "contract_id",
        label: "Contract",
        required: true,
        dynamic: "contract.id.name",
        helpText:
          "The contract this record has to satisfy, listed from the API key. A contract needs a stable version: deploy one in ContractGate first.",
      },
      {
        key: "record",
        label: "Record",
        type: "text",
        required: true,
        helpText:
          "One JSON object, as text. Most triggers split a record into fields, so put a **Code by Zapier** step first that returns `{ record: JSON.stringify(inputData) }`, and map its Record output here. Typing JSON around mapped fields also works, but quote text values yourself.",
      },
      {
        key: "version",
        label: "Contract Version",
        type: "string",
        required: false,
        helpText: "Leave blank to use the latest stable version.",
      },
      {
        key: "halt_on_failure",
        label: "Stop the Zap when the record fails",
        type: "boolean",
        required: false,
        default: "true",
        helpText:
          "On by default. Turn off only if a later Path or Filter should handle the failure. The record is still quarantined either way, unless Dry run is on.",
      },
      {
        key: "dry_run",
        label: "Dry run",
        type: "boolean",
        required: false,
        default: "false",
        helpText: "Check the record without writing an audit row, a quarantine row, or usage.",
      },
      {
        key: "idempotency_key",
        label: "Idempotency Key",
        type: "string",
        required: false,
        helpText:
          "Optional. Map a stable id from the source (lead id, order id) so a Zap retry does not quarantine the same record twice inside 24 hours.",
      },
    ],
    // Shown in the editor before a test runs and to Zapier's sample consumers.
    sample: SAMPLE,
    outputFields: [
      { key: "passed", label: "Passed", type: "boolean" },
      { key: "contract_id", label: "Contract ID" },
      { key: "contract_version", label: "Contract Version" },
      { key: "quarantine_id", label: "Quarantine ID" },
      { key: "violations_summary", label: "Violations" },
      { key: "payload", label: "Payload", dict: true },
      { key: "dry_run", label: "Dry Run", type: "boolean" },
    ],
  },
};
