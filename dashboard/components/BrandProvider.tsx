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
 * The brand to show. A signed-in org's plan wins over the cookie; the cookie
 * is then kept in step so the next server render matches without a flash.
 */
export function useBrand(): Brand {
  const initial = useContext(BrandContext);
  const { org } = useOrg();
  const brand: Brand = org ? (isAisgPlan(org.plan) ? "aisg" : "dcg") : initial;

  useEffect(() => {
    if (!org || brand === initial) return;
    document.cookie =
      brand === "aisg"
        ? `${BRAND_COOKIE}=aisg; Path=/; Max-Age=${60 * 60 * 24 * 30}; SameSite=Lax; Secure`
        : `${BRAND_COOKIE}=; Path=/; Max-Age=0; SameSite=Lax; Secure`;
  }, [org, brand, initial]);

  return brand;
}
