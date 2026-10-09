const { listContracts } = require("../lib/contracts");

const perform = async (z, bundle) => {
  const rows = await listContracts(z, bundle);
  return rows.map((row) => ({
    id: row.id,
    // Ingest needs a stable version; say so before the Zap runs, not after.
    name: row.latest_stable_version
      ? `${row.name} (${row.latest_stable_version})`
      : `${row.name} (draft only: deploy a version first)`,
  }));
};

module.exports = {
  key: "contract",
  noun: "Contract",
  display: {
    label: "Contract",
    description: "Lists contracts for the dropdown. Hidden from the Zap editor.",
    hidden: true,
  },
  operation: {
    cleanInputData: false,
    perform,
    sample: {
      id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
      name: "salesforce_lead (1.0.0)",
    },
  },
};
