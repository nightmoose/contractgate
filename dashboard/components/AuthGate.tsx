"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { createClient } from "@/lib/supabase/client";
import { DEMO_MODE } from "@/lib/demo";
import { loggedOutAuthHrefs } from "@/lib/logged-out-cta";
import { PREVIEWS } from "@/lib/previews";
import { useBrand } from "@/components/BrandProvider";
import { useVisitorAuthed } from "@/components/VisitorSession";

// ── AuthGate ──────────────────────────────────────────────────────────────────

interface AuthGateProps {
  page: keyof typeof PREVIEWS;
  children: React.ReactNode;
}

export default function AuthGate({ page, children }: AuthGateProps) {
  // Hooks stay above the demo return. DEMO_MODE is a build-time constant, so
  // every render of a given bundle takes the same branch inside useOrg.
  const known = useVisitorAuthed();
  const pathname = usePathname() || "/";
  const brand = useBrand();
  const [authed, setAuthed] = useState<boolean | null>(DEMO_MODE ? true : known);

  useEffect(() => {
    if (DEMO_MODE) return;
    // The root layout already resolved the session, so the preview (and its
    // Sign up link) is in the first HTML. A missing value falls back to the
    // browser session check.
    if (known !== null) {
      setAuthed(known);
      return;
    }
    let cancelled = false;
    const supabase = createClient();
    supabase.auth.getUser().then(({ data: { user } }) => {
      if (!cancelled) setAuthed(!!user);
    });
    return () => {
      cancelled = true;
    };
  }, [known]);

  // Demo mode: no auth wall — render real content immediately.
  if (DEMO_MODE || authed) return <>{children}</>;

  // Loading — render nothing to avoid flash
  if (authed === null) return (
    <div className="flex items-center justify-center h-64">
      <h1 className="sr-only">{PREVIEWS[page].title}</h1>
      <div className="w-5 h-5 border-2 border-green-600 border-t-transparent rounded-full animate-spin" aria-hidden="true" />
    </div>
  );

  // Account has nothing to preview: send signed-out visitors to sign-in.
  if (page === "account") {
    if (typeof window !== "undefined") window.location.replace("/auth/login?next=/account");
    return null;
  }

  // Unauthenticated — sample preview, with the way in above the illustration
  // so it is on screen before the chart.
  const preview = PREVIEWS[page];
  const product = brand === "aisg" ? "aischemagate" : null;
  const { signUpHref, signInHref } = loggedOutAuthHrefs(pathname, product);
  return (
    <div className="max-w-2xl mx-auto py-8 px-4">
      <div className="mb-6 rounded-xl border border-amber-700/40 bg-amber-950/30 px-4 py-3 text-center">
        <p className="text-sm font-medium text-amber-200">
          Demo preview. These figures are a sample, not a live account.
        </p>
      </div>

      <div className="text-center">
        <h1 className="text-2xl font-bold text-slate-100 mb-3">{preview.title}</h1>
        <p className="text-slate-400 leading-relaxed mb-6 max-w-md mx-auto">
          {preview.description}
        </p>
        <div className="flex gap-3 justify-center">
          <Link
            href={signUpHref}
            className="px-6 py-2.5 bg-green-600 hover:bg-green-500 text-white rounded-lg text-sm font-medium transition-colors"
          >
            Sign up
          </Link>
          <Link
            href={signInHref}
            className="px-6 py-2.5 bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg text-sm font-medium transition-colors border border-[#374151]"
          >
            Sign in
          </Link>
        </div>
        <p className="mt-5 text-xs text-slate-600">
          Cloud Free needs no credit card. The Pro trial asks for one at checkout.{" "}
          <Link href="/playground" className="text-green-600 hover:text-green-500">
            Try the live demo first →
          </Link>
        </p>
      </div>

      <div className="mt-8 opacity-75">
        {preview.illustration}
      </div>
    </div>
  );
}
