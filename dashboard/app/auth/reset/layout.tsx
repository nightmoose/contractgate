import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Set a new password",
  "Choose a new password for your DataContractGate account.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
