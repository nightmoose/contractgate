import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Scorecard",
  "Per-provider pass rates, field violations, and drift signals for DataContractGate.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
