"use client";

/**
 * DemoFeatureUnavailable — RFC-023 Phase 2.
 *
 * Reusable gate component for pages/sections that require real auth or
 * cloud-only features.  Rendered instead of page content when DEMO_MODE=1.
 *
 * Usage:
 *   import DemoFeatureUnavailable from "@/components/DemoFeatureUnavailable";
 *   if (DEMO_MODE) return <DemoFeatureUnavailable feature="GitHub sync" />;
 */

import Link from "next/link";

interface Props {
  feature: string;
  /** Optional extra sentence explaining why it needs Cloud. */
  reason?: string;
}

export default function DemoFeatureUnavailable({ feature, reason }: Props) {
  return (
    <div className="flex flex-col items-center justify-center min-h-[320px] py-16 px-4 text-center">
      {/* Lock icon */}
      <div className="mb-5 w-12 h-12 rounded-full bg-[#111827] border border-[#1f2937] flex items-center justify-center">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="20"
          height="20"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          className="text-slate-500"
          aria-hidden="true"
        >
          <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
          <path d="M7 11V7a5 5 0 0 1 10 0v4" />
        </svg>
      </div>

      {/* Heading */}
      <h2 className="text-lg font-semibold text-slate-200 mb-2">{feature}</h2>
      <p className="text-sm text-slate-500 mb-1 max-w-sm">
        Not available in <span className="text-slate-400 font-medium">Self-Hosted Free</span> mode.
      </p>
      {reason && (
        <p className="text-xs text-slate-600 mb-0 max-w-xs">{reason}</p>
      )}

      {/* CTA */}
      <div className="mt-6 flex flex-col items-center gap-2">
        <Link
          href="https://contractgate.io/cloud"
          target="_blank"
          rel="noopener noreferrer"
          className="px-5 py-2 bg-green-600 hover:bg-green-500 text-white rounded-lg text-sm font-medium transition-colors"
        >
          Upgrade to ContractGate Cloud →
        </Link>
        <span className="text-xs text-slate-600">
          Multi-tenancy · SSO · GitHub sync · API key management
        </span>
      </div>
    </div>
  );
}
