"use client";

/**
 * Workbench page — RFC-046.
 *
 * Requires authentication. Free tier gets Try It mode (1 endpoint);
 * Growth+ gets full suite, deploy, and export.
 */

import AuthGate from "@/components/AuthGate";
import WorkbenchClient from "./WorkbenchClient";

export default function WorkbenchPage() {
  return (
    <AuthGate page="workbench">
      <WorkbenchClient />
    </AuthGate>
  );
}
