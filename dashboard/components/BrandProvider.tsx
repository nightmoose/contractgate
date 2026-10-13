"use client";

import { createContext, useContext, useEffect } from "react";
import { useOrg, isAisgPlan } from "@/lib/org";
import { BRAND_COOKIE, type Brand } from "@/lib/brand";

const BrandContext = createContext<Brand>("dcg");

/** `initial` comes from the request cookie (root layout), so the server render is already right. */
export default function BrandProvider({ initial, children }: { initial: Brand; children: React.ReactNode }) {
  return <BrandContext.Provider value={initial}>{children}</BrandContext.Provider>;
}

/**
 * The brand to show.
 *
 * RFC-102 (E4): the `cg_product` cookie is the session's declared product
 * intent — set by `proxy.ts` from `?product=aischemagate`, and only ever
 * cleared by an explicit `?product=datacontractgate`. The signed-in org's plan
 * is allowed to *upgrade* the brand from `dcg → aisg` (so new AISG signups
 * flip immediately once their plan is `aisg_*`), but is never allowed to
 * downgrade a cookie-declared `aisg` session back to `dcg`. That makes an
 * explicit URL param stick across the whole session instead of only the first
 * page render, and keeps the brand consistent for AISG free-tier users whose
 * plan still reads `free` while migration 038 catches up.
 */
export function useBrand(): Brand {
  const initial = useContext(BrandContext);
  const { org } = useOrg();
  const planBrand: Brand | null = org ? (isAisgPlan(org.plan) ? "aisg" : "dcg") : null;
  const brand: Brand = initial === "aisg" ? "aisg" : (planBrand ?? initial);

  useEffect(() => {
    // Only *upgrade* the cookie — never clear it based on a dcg plan. Users
    // opt out of AISG by appending `?product=datacontractgate`, which the
    // middleware handles directly.
    if (brand === "aisg" && initial !== "aisg") {
      document.cookie = `${BRAND_COOKIE}=aisg; Path=/; Max-Age=${60 * 60 * 24 * 30}; SameSite=Lax; Secure`;
    }
  }, [brand, initial]);

  return brand;
}
