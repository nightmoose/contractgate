const { baseUrl } = require("../lib/base_url");
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
  if (bundle.meta && bundle.meta.isLoadingSample) {
    return SAMPLE;
  }

  const record = parseRecord(bundle.inputData.record);
  const contractId = bundle.inputData.contract_id;
  if (!contractId) {
    throw new z.errors.Error("Pick a contract.", "MissingContract", 400);
  }

  const dryRun = asBool(bundle.inputData.dry_run, false);
  const haltOnFailure = asBool(bundle.inputData.halt_on_failure, true);
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
    const where = shaped.quarantine_id
      ? `Quarantine ${shaped.quarantine_id}. Replay it from ContractGate after the record or the contract is fixed. Zapier will not replay this task.`
      : "Dry run: nothing was quarantined.";
    // HaltedError stops the Zap without counting as an error that turns the Zap off.
    throw new z.errors.HaltedError(
      `Contract rejected the record. ${violationsSummary || "See the contract."} ${where}`,
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
        helpText: "The contract this record has to satisfy. Listed from the API key.",
      },
      {
        key: "record",
        label: "Record",
        type: "text",
        required: true,
        helpText:
          "One JSON object: the record Zapier just pulled. If the trigger split it into fields, add a Code by Zapier step that returns `JSON.stringify(inputData)` and map that string here.",
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
