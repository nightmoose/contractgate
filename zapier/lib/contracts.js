const { baseUrl } = require("./base_url");

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// GET /contracts, refusing anything that is not the gateway's JSON list. A
// dashboard host answers this path with an HTML page and a 200, which used to
// read as "0 contracts" and a passing connection test.
async function listContracts(z, bundle) {
  const response = await z.request({
    url: `${baseUrl(bundle.authData)}/contracts`,
    skipThrowForStatus: true,
  });
  if (response.status === 401 || response.status === 403) {
    throw new z.errors.ExpiredAuthError(
      "ContractGate rejected this API key. Create a key at https://app.datacontractgate.com/account and reconnect.",
    );
  }
  let rows = response.data;
  if (typeof rows === "string") {
    try {
      rows = JSON.parse(rows);
    } catch {
      rows = null;
    }
  }
  if (response.status !== 200 || !Array.isArray(rows)) {
    throw new z.errors.Error(
      `The Gateway URL did not answer like the ContractGate API (HTTP ${response.status}). Leave Gateway URL blank for ContractGate Cloud.`,
      "NotContractGate",
      response.status,
    );
  }
  return rows;
}

// The dropdown sends the contract id, but a typed or mapped name is accepted
// too: ingest needs the UUID, so a name is looked up here.
async function resolveContractId(z, bundle, value) {
  const raw = String(value).trim();
  if (UUID.test(raw)) return raw;
  const rows = await listContracts(z, bundle);
  const match =
    rows.find((row) => row.name === raw) ||
    rows.find((row) => String(row.name).toLowerCase() === raw.toLowerCase());
  if (!match) {
    const names = rows.map((row) => row.name).join(", ") || "none";
    throw new z.errors.Error(
      `No contract named "${raw}" on this API key. Pick one from the Contract dropdown. Contracts on this key: ${names}.`,
      "UnknownContract",
      404,
    );
  }
  return match.id;
}

module.exports = { UUID, listContracts, resolveContractId };
