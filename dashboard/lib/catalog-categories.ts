// Catalog category filters, in display order. Kept in its own module so the
// client-side CatalogBrowser can import it without pulling the server-only
// fetch/yaml helpers in lib/catalog.ts into the client bundle.
export const CATALOG_CATEGORIES = [
  "Payments",
  "Commerce",
  "Accounting",
  "CRM",
  "Analytics",
  "Spreadsheets",
  "Public data",
] as const;
