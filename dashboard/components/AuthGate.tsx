"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { createClient } from "@/lib/supabase/client";
import { DEMO_MODE } from "@/lib/demo";
import { PREVIEWS } from "@/lib/previews";

// ── AuthGate ──────────────────────────────────────────────────────────────────

interface AuthGateProps {
  page: keyof typeof PREVIEWS;
  children: React.ReactNode;
}

export default function AuthGate({ page, children }: AuthGateProps) {
  // Demo mode: no auth wall — render real content immediately.
  if (DEMO_MODE) return <>{children}</>;

  const [authed, setAuthed] = useState<boolean | null>(null);

  useEffect(() => {
    const supabase = createClient();
    supabase.auth.getUser().then(({ data: { user } }) => {
      setAuthed(!!user);
    });
  }, []);

  // Loading — render nothing to avoid flash
  if (authed === null) return (
    <div className="flex items-center justify-center h-64">
      <div className="w-5 h-5 border-2 border-green-600 border-t-transparent rounded-full animate-spin" />
    </div>
  );

  // Authenticated — show real content
  if (authed) return <>{children}</>;

  // Unauthenticated — show compelling preview
  const preview = PREVIEWS[page];
  return (
    <div className="max-w-2xl mx-auto py-16 px-4">
      {/* Illustration */}
      <div className="mb-8 opacity-75">
        {preview.illustration}
      </div>

      {/* Copy */}
      <div className="text-center">
        <h2 className="text-2xl font-bold text-slate-100 mb-3">{preview.title}</h2>
        <p className="text-slate-400 leading-relaxed mb-8 max-w-md mx-auto">
          {preview.description}
        </p>
        <div className="flex gap-3 justify-center">
          <Link
            href="/auth/signup"
            className="px-6 py-2.5 bg-green-600 hover:bg-green-500 text-white rounded-lg text-sm font-medium transition-colors"
          >
            {preview.cta} →
          </Link>
          <Link
            href="/auth/login"
            className="px-6 py-2.5 bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg text-sm font-medium transition-colors border border-[#374151]"
          >
            Sign in
          </Link>
        </div>
        <p className="mt-5 text-xs text-slate-600">
          No credit card required · Free to start ·{" "}
          <Link href="/stream-demo" className="text-green-600 hover:text-green-500">
            Try the live demo first →
          </Link>
        </p>
      </div>
    </div>
  );
}
