import type { MetadataRoute } from "next";
import { fetchPublicCatalog, SITE_URL } from "@/lib/catalog";

// Follow the catalog fetch: new entries show up within about a minute.
export const revalidate = 60;

export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  const base: MetadataRoute.Sitemap = [
    { url: `${SITE_URL}/catalog`, changeFrequency: "daily", priority: 0.8 },
  ];

  try {
    const contracts = await fetchPublicCatalog();
    for (const c of contracts) {
      base.push({
        url: `${SITE_URL}/catalog/${c.slug}`,
        lastModified: c.updated_at ? new Date(c.updated_at) : undefined,
        changeFrequency: "weekly",
        priority: 0.7,
      });
    }
  } catch {
    // If the API is unreachable at build time, still ship the index entry.
  }

  return base;
}
