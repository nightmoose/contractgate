import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Sign up",
  "Create a DataContractGate account. Free to start, no credit card required.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
