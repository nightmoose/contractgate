import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Stream demo",
  "Run the DataContractGate validation engine on a live stream. No Kafka and no database writes.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
