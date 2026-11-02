import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Payment successful",
  "Your DataContractGate subscription payment completed.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
