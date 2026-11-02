import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Sign in",
  "Sign in to DataContractGate to manage contracts, keys, and validation.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
