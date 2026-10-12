/**
 * Which product the visitor came for. The app is one DataContractGate
 * dashboard; AISchemaGate accounts (migration 036) and visitors arriving from
 * aischemagate.com see AISchemaGate branding and a narrower navigation.
 *
 * Resolution: a signed-in org's plan decides; before that, the
 * BRAND_COOKIE set by proxy.ts from ?product=aischemagate does.
 */
export type Brand = "dcg" | "aisg";

export const BRAND_COOKIE = "cg_product";

/** Cookie value → brand. Anything but "aisg" is DataContractGate. */
export function brandFromCookie(value: string | undefined | null): Brand {
  return value === "aisg" ? "aisg" : "dcg";
}

export const BRAND_NAME: Record<Brand, string> = {
  dcg: "DataContractGate",
  aisg: "AISchemaGate",
};

export const BRAND_TAGLINE: Record<Brand, string> = {
  dcg: "Semantic contract enforcement at ingestion",
  aisg: "Semantic validation for LLM outputs",
};

export const AISG_DOCS_URL = "https://aischemagate.dev";
