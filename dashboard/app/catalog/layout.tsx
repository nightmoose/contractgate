import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Catalog",
  "Browse open-data contracts and validate outbound payloads before they leave your API.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
