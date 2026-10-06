function parseRecord(value) {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return value;
  }
  if (typeof value !== "string") {
    throw new Error("Record must be one JSON object.");
  }
  const text = value.trim();
  if (!text) {
    throw new Error("Record is empty. Map the object Zapier just pulled.");
  }
  if (text === "[object Object]") {
    throw new Error(
      "Zapier passed the record as [object Object]. Put a Code by Zapier step in front of this one that returns JSON.stringify(inputData), and map that string here.",
    );
  }
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error("Record is not valid JSON. Send one object, not a list or plain text.");
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("Record must be one JSON object. Validate one pulled record per Zap step.");
  }
  return parsed;
}

function asBool(value, fallback) {
  if (value === undefined || value === null || value === "") return fallback;
  if (typeof value === "boolean") return value;
  const s = String(value).toLowerCase();
  if (s === "true" || s === "yes" || s === "1") return true;
  if (s === "false" || s === "no" || s === "0") return false;
  return fallback;
}

function summarize(violations) {
  if (!Array.isArray(violations) || violations.length === 0) return "";
  return violations
    .map((v) => {
      const field = (v && (v.field || v.rule)) || "record";
      const message = (v && (v.message || v.kind)) || "failed validation";
      return `${field}: ${message}`;
    })
    .join("\n");
}

module.exports = { parseRecord, asBool, summarize };
