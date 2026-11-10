import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Sign up",
  "Create a DataContractGate account. Cloud Free needs no credit card. The Pro trial asks for one at checkout.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
