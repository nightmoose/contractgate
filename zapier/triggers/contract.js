const { baseUrl } = require("../lib/base_url");

const perform = async (z, bundle) => {
  const response = await z.request({
    url: `${baseUrl(bundle.authData)}/contracts`,
  });
  const rows = Array.isArray(response.data) ? response.data : [];
  return rows.map((row) => ({
    id: row.id,
    name: row.latest_stable_version ? `${row.name} (${row.latest_stable_version})` : row.name,
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
