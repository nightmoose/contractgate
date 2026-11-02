import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Python SDK",
  "Python client and local validator for DataContractGate semantic contracts.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
