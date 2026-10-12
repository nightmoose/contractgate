"use client";

import { useBrand } from "@/components/BrandProvider";
import { BRAND_NAME, BRAND_TAGLINE } from "@/lib/brand";

/** Product name for the auth pages; AISchemaGate adds "by DataContractGate". */
export default function BrandWordmark() {
  const brand = useBrand();
  return (
    <span className="inline-flex flex-col items-center">
      <span className="text-2xl font-bold text-green-400">{BRAND_NAME[brand]}</span>
      {brand === "aisg" && <span className="text-xs text-slate-500 mt-0.5">by DataContractGate</span>}
    </span>
  );
}

/** Tagline under the wordmark on the auth pages. */
export function BrandTagline({ className }: { className?: string }) {
  const brand = useBrand();
  return <p className={className}>{BRAND_TAGLINE[brand]}</p>;
}
