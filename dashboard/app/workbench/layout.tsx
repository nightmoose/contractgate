import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Workbench",
  "Explore an API, infer a contract from live responses, and deploy enforcement.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
