"use client";

import { useEffect } from "react";
import Link from "next/link";

export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    // Log to server-side error tracking when available
    console.error(error);
  }, [error]);

  return (
    <div className="min-h-screen bg-[#0a0d12] flex flex-col items-center justify-center px-4 text-center">
      <div className="mb-6 w-14 h-14 rounded-full bg-[#111827] border border-red-900/40 flex items-center justify-center">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="22"
          height="22"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          className="text-red-400"
          aria-hidden="true"
        >
          <circle cx="12" cy="12" r="10" />
          <line x1="12" y1="8" x2="12" y2="12" />
          <line x1="12" y1="16" x2="12.01" y2="16" />
        </svg>
      </div>

      <h1 className="text-2xl font-bold text-slate-100 mb-2">Something went wrong</h1>
      <p className="text-slate-500 text-sm mb-8 max-w-sm">
        An unexpected error occurred. Our team has been notified. You can try
        again or return to the dashboard.
      </p>
      {error.digest && (
        <p className="text-xs text-slate-700 font-mono mb-6">
          Error ID: {error.digest}
        </p>
      )}

      <div className="flex gap-3">
        <button
          onClick={reset}
          className="px-4 py-2 bg-[#111827] hover:bg-[#1f2937] border border-[#1f2937] text-slate-300 rounded-lg text-sm transition-colors"
        >
          Try again
        </button>
        <Link
          href="/contracts"
          className="px-4 py-2 bg-green-600 hover:bg-green-500 text-white rounded-lg text-sm transition-colors"
        >
          Back to dashboard
        </Link>
      </div>
    </div>
  );
}
