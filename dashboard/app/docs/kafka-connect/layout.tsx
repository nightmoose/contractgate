import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Kafka Connect",
  "Validate Kafka records with the DataContractGate Connect SMT before they reach your warehouse.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
