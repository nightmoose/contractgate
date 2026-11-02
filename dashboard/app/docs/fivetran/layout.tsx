import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Fivetran",
  "Load contract-passed DataContractGate events into your warehouse with Fivetran.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
