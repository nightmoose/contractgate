import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Scaffold",
  "Turn a JSON sample, NDJSON stream, Avro schema, or Protobuf definition into contract YAML.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
