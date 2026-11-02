import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Account",
  "API keys, team members, and billing for your DataContractGate organization.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
