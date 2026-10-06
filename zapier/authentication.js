const { baseUrl } = require("./lib/base_url");

const test = async (z, bundle) => {
  const response = await z.request({
    url: `${baseUrl(bundle.authData)}/contracts`,
  });
  const rows = Array.isArray(response.data) ? response.data : [];
  return { contract_count: rows.length };
};

module.exports = {
  type: "custom",
  test,
  connectionLabel: "{{contract_count}} contracts",
  fields: [
    {
      key: "api_key",
      label: "API Key",
      type: "password",
      required: true,
      helpText:
        "From [Account → API keys](https://app.datacontractgate.com/account). The key starts with `cg_live_`.",
    },
    {
      key: "base_url",
      label: "Gateway URL",
      type: "string",
      required: false,
      default: "https://app.datacontractgate.com",
      helpText:
        "Leave the default for ContractGate Cloud. Set this only if you self-host the gateway. Must be an `https://` or `http://` URL. See [the setup doc](https://github.com/nightmoose/contractgate/blob/main/docs/zapier.md).",
    },
  ],
};
