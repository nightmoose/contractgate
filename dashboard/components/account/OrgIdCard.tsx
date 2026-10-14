"use client";

// Shows the caller's org ID with a copy button. Contract collaboration grants
// access by org ID, and the app never displayed it (tester polish round).

import { useState } from "react";
import { useOrg } from "@/lib/org";

export default function OrgIdCard({ compact = false }: { compact?: boolean }) {
  const { org } = useOrg();
  const [copied, setCopied] = useState(false);
  if (!org) return null;
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(org.org_id);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard blocked: the ID is selectable */
    }
  };
  return (
    <div className={compact ? "text-xs" : "bg-[#111827] border border-[#1f2937] rounded-2xl p-6 mb-8"}>
      {!compact && <div className="text-sm font-semibold text-slate-200 mb-1">Organization ID</div>}
      <p className="text-xs text-slate-500 mb-2">
        {compact ? "Your org ID" : "Share this with a contract owner in another organization so they can invite you to collaborate."}
      </p>
      <div className="flex items-center gap-2">
        <code className="flex-1 font-mono text-xs text-slate-300 bg-[#0a0d12] border border-[#374151] rounded px-2 py-1.5 select-all break-all">
          {org.org_id}
        </code>
        <button type="button" onClick={copy}
          className="text-xs px-2.5 py-1.5 rounded bg-[#1f2937] hover:bg-[#374151] text-slate-200">
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
    </div>
  );
}
