import type { Metadata } from "next";
import { cookies } from "next/headers";
import DashboardPage from "./home-page";
import { BRAND_COOKIE, brandFromCookie } from "@/lib/brand";
import { pageMeta } from "@/lib/page-meta";

// The root layout's title template applies to child segments only, so the
// home page sets the full document title itself.
export async function generateMetadata(): Promise<Metadata> {
  const brand = brandFromCookie((await cookies()).get(BRAND_COOKIE)?.value);
  const product = brand === "aisg" ? "AISchemaGate" : "DataContractGate";
  return pageMeta(
    `Dashboard — ${product}`,
    "Real-time ingestion health for DataContractGate: pass rate, violations, and latency.",
  );
}

export default function Page() {
  return <DashboardPage />;
}
