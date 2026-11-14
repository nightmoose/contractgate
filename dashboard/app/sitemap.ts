import type { MetadataRoute } from "next";
import { fetchPublicCatalog, fetchPublicFamilies, SITE_URL } from "@/lib/catalog";

// Follow the catalog fetch: new entries show up within about a minute.
export const revalidate = 60;

const DOC_PAGES = [
  "/docs",
  "/docs/python-sdk",
  "/docs/kafka-connect",
  "/docs/fivetran",
];

// Agent-facing markdown served from public/ (and /catalog.md, generated).
const MARKDOWN_DOCS = [
  "/llms.txt",
  "/llms-full.txt",
  "/llm-integration.md",
  "/mcp-reference.md",
  "/llm-sdk-reference.md",
  "/llm-output-validation.md",
  "/catalog.md",
];

function page(
  path: string,
  changeFrequency: "daily" | "weekly",
  priority: number,
): MetadataRoute.Sitemap[number] {
  return { url: `${SITE_URL}${path}`, changeFrequency, priority };
}

export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  const entries: MetadataRoute.Sitemap = [
    page("/playground", "weekly", 0.9),
    page("/catalog", "daily", 0.8),
  ];
  for (const path of DOC_PAGES) {
    entries.push(page(path, "weekly", path === "/docs" ? 0.6 : 0.5));
  }
  for (const path of MARKDOWN_DOCS) {
    entries.push(
      page(
        path,
        path === "/catalog.md" ? "daily" : "weekly",
        path === "/llms.txt" || path === "/llm-integration.md" ? 0.7 : 0.5,
      ),
    );
  }

  try {
    const contracts = await fetchPublicCatalog();
    for (const c of contracts) {
      entries.push({
        url: `${SITE_URL}/catalog/${c.slug}`,
        lastModified: c.updated_at ? new Date(c.updated_at) : undefined,
        changeFrequency: "weekly",
        priority: 0.7,
      });
    }
  } catch {
    // If the API is unreachable at request time, still ship the static entries.
  }

  try {
    const families = await fetchPublicFamilies();
    for (const family of families) {
      if (!family.slug) continue;
      entries.push({
        url: `${SITE_URL}/catalog/families/${family.slug}`,
        changeFrequency: "weekly",
        priority: 0.7,
      });
    }
  } catch {
    // Family pages are additive. A failed fetch keeps the rest of the sitemap.
  }

  return entries;
}
