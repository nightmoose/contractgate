import type { Metadata } from "next";

/**
 * Document title and description for one route.
 * The root layout appends "— DataContractGate" or "— AISchemaGate".
 */
export function pageMeta(title: string, description: string): Metadata {
  return {
    title,
    description,
    openGraph: { title, description },
    twitter: { title, description },
  };
}
