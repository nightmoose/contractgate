"use client";

/**
 * OrgProvider — resolves the current user's org and registers it with the
 * API client so every Rust API call carries the correct x-org-id header.
 *
 * Rendered once in RootLayout (below the Sidebar).  Has no visible output —
 * it's purely a side-effect component.
 */

import { useEffect } from "react";
import { useOrg } from "@/lib/org";
import { setApiOrgId } from "@/lib/api";

export default function OrgProvider() {
  const { org } = useOrg();

  useEffect(() => {
    if (org?.org_id) {
      setApiOrgId(org.org_id);
    }
  }, [org?.org_id]);

  return null;
}
