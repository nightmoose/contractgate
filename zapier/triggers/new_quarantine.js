const { baseUrl } = require("../lib/base_url");
const { summarize } = require("../lib/record");

const perform = async (z, bundle) => {
  const params = new URLSearchParams();
  params.set("limit", "100");
  if (bundle.inputData.contract_id) {
    params.set("contract_id", bundle.inputData.contract_id);
  }
  const response = await z.request({
    url: `${baseUrl(bundle.authData)}/quarantine?${params.toString()}`,
  });
  const rows = Array.isArray(response.data) ? response.data : [];
  return rows.map((row) => ({
    id: row.id,
    contract_id: row.contract_id,
    contract_version: row.contract_version,
    quarantined_at: row.quarantined_at,
    violation_count: row.violation_count,
    violations_summary: summarize(row.violation_details),
    record: row.raw_event,
  }));
};

module.exports = {
  key: "new_quarantine",
  noun: "Quarantined Record",
  display: {
    label: "New Quarantined Record",
    description:
      "Triggers when ContractGate holds a record that failed its contract. Polls the newest 100.",
  },
  operation: {
    cleanInputData: false,
    perform,
    inputFields: [
      {
        key: "contract_id",
        label: "Contract",
        required: false,
        dynamic: "contract.id.name",
        helpText: "Leave blank to hear about every contract on this key.",
      },
    ],
    sample: {
      id: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
      contract_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
      contract_version: "1.0.0",
      quarantined_at: "2026-10-06T15:00:00Z",
      violation_count: 1,
      violations_summary: "email: Field 'email' is required",
      record: { name: "Ada", email: "" },
    },
    outputFields: [
      { key: "id", label: "Quarantine ID" },
      { key: "contract_id", label: "Contract ID" },
      { key: "contract_version", label: "Contract Version" },
      { key: "quarantined_at", label: "Quarantined At" },
      { key: "violation_count", label: "Violation Count", type: "integer" },
      { key: "violations_summary", label: "Violations" },
      { key: "record", label: "Record", dict: true },
    ],
  },
};
