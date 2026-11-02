import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Playground",
  "Test a contract YAML against a sample JSON event with no ingestion and no storage.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
