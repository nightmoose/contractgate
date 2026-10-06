const CLOUD = "https://app.datacontractgate.com";

function baseUrl(authData) {
  const raw = authData && authData.base_url != null ? String(authData.base_url).trim() : "";
  if (!raw) return CLOUD;
  const trimmed = raw.replace(/\/+$/, "");
  if (!/^https?:\/\//i.test(trimmed)) {
    throw new Error("Base URL must start with https:// or http://.");
  }
  return trimmed;
}

module.exports = { CLOUD, baseUrl };
