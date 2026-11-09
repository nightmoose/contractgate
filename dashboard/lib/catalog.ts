/**
 * Server-side helpers for the public catalog pages.
 *
 * The browser `apiFetch` in lib/api.ts injects a Supabase session and can't run
 * in a Server Component. The public catalog endpoints need no auth, so these
 * helpers hit the Rust gateway directly with plain `fetch` + ISR revalidation.
 */
import * as yaml from "js-yaml";
import type { OpenDataContract, OpenDataContractDetail } from "@/lib/api";

// Server-side origin for the Rust gateway. NEXT_PUBLIC_API_URL is the value the
// browser client already uses; CONTRACTGATE_API_ORIGIN mirrors proxy.ts.
const API_ORIGIN =
  process.env.NEXT_PUBLIC_API_URL ??
  process.env.CONTRACTGATE_API_ORIGIN ??
  "https://api.datacontractgate.com";

export const SITE_URL =
  process.env.NEXT_PUBLIC_APP_URL ?? "https://app.datacontractgate.com";

// Revalidate catalog data roughly hourly (ISR).
const REVALIDATE_SECONDS = 3600;

export { CATALOG_CATEGORIES } from "@/lib/catalog-categories";

/** Only rows that have been published to the public directory (have a slug). */
export async function fetchPublicCatalog(): Promise<OpenDataContract[]> {
  const res = await fetch(`${API_ORIGIN}/public-contracts`, {
    next: { revalidate: REVALIDATE_SECONDS },
  });
  if (!res.ok) {
    throw new Error(`catalog list failed: HTTP ${res.status}`);
  }
  const all = (await res.json()) as OpenDataContract[];
  return all.filter((c) => !!c.slug);
}

/** Full detail (YAML + samples) for one slug, or null if not found/unpublished. */
export async function fetchCatalogDetail(
  slug: string
): Promise<OpenDataContractDetail | null> {
  const res = await fetch(
    `${API_ORIGIN}/public-contracts/${encodeURIComponent(slug)}`,
    { next: { revalidate: REVALIDATE_SECONDS } }
  );
  if (res.status === 404) return null;
  if (!res.ok) {
    throw new Error(`catalog detail failed: HTTP ${res.status}`);
  }
  const detail = (await res.json()) as OpenDataContractDetail;
  if (!detail.slug) return null;
  return detail;
}

/** Public download URL for the DataContractGate YAML (served by the gateway). */
export function yamlDownloadUrl(slug: string): string {
  return `${API_ORIGIN}/public-contracts/${encodeURIComponent(slug)}/yaml`;
}

/** Public download URL for the ODCS v3.1.0 export (served by the gateway). */
export function odcsDownloadUrl(slug: string): string {
  return `${API_ORIGIN}/public-contracts/${encodeURIComponent(slug)}/odcs`;
}

/** curl one-liner shown on the detail page for the public API. */
export function curlSnippet(slug: string): string {
  return `curl ${API_ORIGIN}/public-contracts/${slug}/yaml`;
}

// ---------------------------------------------------------------------------
// Contract YAML → field table
// ---------------------------------------------------------------------------

export interface CatalogField {
  name: string;
  type: string;
  required: boolean;
  rule: string;
  description: string;
}

interface RawField {
  name?: string;
  type?: string;
  required?: boolean;
  pattern?: string;
  enum?: unknown[];
  min?: number;
  max?: number;
  min_length?: number;
  max_length?: number;
  format?: string;
}

interface RawContract {
  ontology?: { entities?: RawField[] };
  glossary?: { field?: string; description?: string }[];
}

function fieldRule(f: RawField): string {
  const parts: string[] = [];
  if (f.enum && f.enum.length > 0) {
    parts.push(`one of: ${f.enum.map((v) => String(v)).join(", ")}`);
  }
  if (f.pattern) parts.push(`pattern /${f.pattern}/`);
  if (f.format) parts.push(`${f.format} format`);
  if (f.min !== undefined && f.max !== undefined) parts.push(`${f.min}–${f.max}`);
  else if (f.min !== undefined) parts.push(`≥ ${f.min}`);
  else if (f.max !== undefined) parts.push(`≤ ${f.max}`);
  if (f.min_length !== undefined) parts.push(`min length ${f.min_length}`);
  if (f.max_length !== undefined) parts.push(`max length ${f.max_length}`);
  return parts.join("; ");
}

/** Parse the contract YAML into a flat field table for the detail page. */
export function parseContractFields(contractYaml: string): CatalogField[] {
  let doc: RawContract;
  try {
    doc = (yaml.load(contractYaml) ?? {}) as RawContract;
  } catch {
    return [];
  }
  const entities = doc.ontology?.entities ?? [];
  const glossary = new Map<string, string>();
  for (const g of doc.glossary ?? []) {
    if (g.field) glossary.set(g.field, g.description ?? "");
  }
  return entities
    .filter((f): f is RawField & { name: string } => typeof f.name === "string")
    .map((f) => ({
      name: f.name,
      type: f.type ?? "string",
      // The validator defaults `required` to true when omitted.
      required: f.required !== false,
      rule: fieldRule(f),
      description: glossary.get(f.name) ?? "",
    }));
}
