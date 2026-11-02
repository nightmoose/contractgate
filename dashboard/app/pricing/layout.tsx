import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Pricing",
  "DataContractGate and AISchemaGate plans: self-hosted, cloud free, Pro, and Enterprise.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
