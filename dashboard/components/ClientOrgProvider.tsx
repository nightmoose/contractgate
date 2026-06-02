"use client";

/**
 * Thin client boundary that lazy-loads OrgProvider with ssr:false.
 *
 * next/dynamic with ssr:false is only allowed inside Client Components.
 * layout.tsx is a Server Component, so the dynamic() call lives here instead,
 * and layout.tsx imports this wrapper.
 */

import dynamic from "next/dynamic";

const OrgProvider = dynamic(() => import("@/components/OrgProvider"), {
  ssr: false,
});

export default function ClientOrgProvider() {
  return <OrgProvider />;
}
