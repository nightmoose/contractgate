import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Contracts",
  "Create, version, and manage semantic contracts for DataContractGate.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
