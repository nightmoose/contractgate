/**
 * Keys the catalog fork page and the contracts page use to hand a contract
 * to the playground. The playground reads them once on mount and clears them.
 *
 * Catalog forks must also hand a published sample. The playground's schema
 * guess (`example_<field>`, null for objects and dates) does not satisfy the
 * catalog contracts' own patterns.
 */
export const PLAYGROUND_YAML_KEY = "playground_yaml";
export const PLAYGROUND_CONTRACT_ID_KEY = "playground_contract_id";
export const PLAYGROUND_EVENT_KEY = "playground_event";

/** First object sample, pretty-printed, or null when the catalog published none. */
export function catalogSampleEvent(samples: unknown[] | null | undefined): string | null {
  if (!Array.isArray(samples)) return null;
  const sample = samples.find(
    (row) => row !== null && typeof row === "object" && !Array.isArray(row)
  );
  if (!sample) return null;
  return JSON.stringify(sample, null, 2);
}
