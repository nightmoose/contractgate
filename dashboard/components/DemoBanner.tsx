"use client";

/**
 * DemoBanner — RFC-023 Phase 2.
 *
 * Fixed top bar rendered in demo mode only.  Copy: "Self-Hosted Free" so
 * visitors immediately understand the edition.  Upgrade CTA links to the
 * Cloud landing page.
 *
 * Mounted in app/layout.tsx when NEXT_PUBLIC_DEMO_MODE=1.
 * Height: 36px (h-9) — layout.tsx adds pt-9 to <main> to compensate.
 */

import Link from "next/link";

export default function DemoBanner() {
  return (
    <div className="fixed top-0 left-0 right-0 z-50 h-9 flex items-center justify-between px-4 bg-[#0d1117] border-b border-[#1f2937]">
      {/* Left: edition badge */}
      <div className="flex items-center gap-2">
        <span className="text-xs font-semibold text-slate-400 tracking-wide">
          Self-Hosted Free
        </span>
        <span className="hidden sm:inline text-slate-700">·</span>
        <span className="hidden sm:inline text-xs text-slate-600">
          Single tenant · No auth
        </span>
      </div>

      {/* Right: upgrade CTA */}
      <Link
        href="https://contractgate.io/cloud"
        target="_blank"
        rel="noopener noreferrer"
        className="flex items-center gap-1.5 text-xs text-green-500 hover:text-green-400 transition-colors font-medium"
      >
        ContractGate Cloud
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
          <polyline points="15 3 21 3 21 9" />
          <line x1="10" y1="14" x2="21" y2="3" />
        </svg>
      </Link>
    </div>
  );
}
