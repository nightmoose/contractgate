import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Audit log",
  "Every DataContractGate validation attempt, pass or fail, recorded at ingestion time.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
