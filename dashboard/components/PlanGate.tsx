"use client";

/**
 * PlanGate — RFC-045: feature gating by billing plan tier.
 *
 * Usage (simple lock card):
 *   <PlanGate minTier="growth" feature="Visual Builder">
 *     <VisualBuilder ... />
 *   </PlanGate>
 *
 * Usage (illustrated preview — recommended for full pages):
 *   <PlanGate minTier="growth" feature="Scorecard" previewKey="scorecard">
 *     <ScorecardContent />
 *   </PlanGate>
 *
 * When previewKey is supplied, free-tier users see the same illustrated
 * preview that logged-out users see in AuthGate, with an "Upgrade to Growth"
 * CTA instead of a "Sign in" button.
 *
 * Renders nothing while org is loading (avoids flash of upsell).
 */

import Link from "next/link";
import { useOrg, planAtLeast, type PlanTier } from "@/lib/org";
import { PREVIEWS } from "@/lib/previews";

// ---------------------------------------------------------------------------
// Tier display helpers
// ---------------------------------------------------------------------------

const TIER_LABEL: Record<PlanTier, string> = {
  free: "Cloud Free",
  growth: "Growth",
  enterprise: "Enterprise",
};

const TIER_COLOR: Record<PlanTier, string> = {
  free: "text-slate-400",
  growth: "text-green-400",
  enterprise: "text-indigo-400",
};

// ---------------------------------------------------------------------------
// PreviewUpsell — illustrated card shown to free-tier users on full pages
// ---------------------------------------------------------------------------

function PreviewUpsell({
  previewKey,
  minTier,
}: {
  previewKey: string;
  minTier: PlanTier;
}) {
  const preview = PREVIEWS[previewKey];
  if (!preview) return <SimpleUpsell feature={previewKey} minTier={minTier} />;

  return (
    <div className="max-w-2xl mx-auto py-16 px-4">
      {/* Illustration — same as AuthGate, slightly dimmed */}
      <div className="mb-8 opacity-75">
        {preview.illustration}
      </div>

      <div className="text-center">
        <h2 className="text-2xl font-bold text-slate-100 mb-3">{preview.title}</h2>
        <p className="text-slate-400 leading-relaxed mb-8 max-w-md mx-auto">
          {preview.description}
        </p>

        {/* Plan badge */}
        <p className="text-sm text-slate-500 mb-5">
          Available on the{" "}
          <span className={`font-semibold ${TIER_COLOR[minTier]}`}>
            {TIER_LABEL[minTier]}
          </span>{" "}
          plan and above.
        </p>

        <div className="flex gap-3 justify-center">
          <Link
            href="/pricing"
            className="px-6 py-2.5 bg-green-600 hover:bg-green-500 text-white rounded-lg text-sm font-medium transition-colors"
          >
            Upgrade to {TIER_LABEL[minTier]} →
          </Link>
          <Link
            href="/pricing"
            className="px-6 py-2.5 bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg text-sm font-medium transition-colors border border-[#374151]"
          >
            See all plans
          </Link>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// SimpleUpsell — compact lock card for tab-level gates (no illustration)
// ---------------------------------------------------------------------------

function SimpleUpsell({
  feature,
  minTier,
}: {
  feature: string;
  minTier: PlanTier;
}) {
  return (
    <div className="flex flex-col items-center justify-center h-64 text-center px-6">
      <div className="mb-3 text-4xl select-none">🔒</div>
      <p className="text-base font-semibold text-slate-200 mb-1">{feature}</p>
      <p className="text-sm text-slate-500 mb-5">
        Available on the{" "}
        <span className={`font-semibold ${TIER_COLOR[minTier]}`}>
          {TIER_LABEL[minTier]}
        </span>{" "}
        plan and above.
      </p>
      <Link
        href="/pricing"
        className="px-5 py-2.5 bg-green-600 hover:bg-green-500 text-white text-sm font-semibold rounded-lg transition-colors"
      >
        Upgrade to {TIER_LABEL[minTier]} →
      </Link>
    </div>
  );
}

// ---------------------------------------------------------------------------
// PlanGate
// ---------------------------------------------------------------------------

interface PlanGateProps {
  /** Minimum plan required to see the children. */
  minTier: PlanTier;
  /** Human-readable feature name shown in the upsell (used as fallback title). */
  feature: string;
  /**
   * Optional key into the PREVIEWS registry.  When supplied, free-tier users
   * see the illustrated preview (same as AuthGate's logged-out view) with an
   * upgrade CTA instead of the sign-in buttons.
   *
   * Use this for full pages.  Leave unset for tab/section-level gates.
   */
  previewKey?: keyof typeof PREVIEWS;
  children: React.ReactNode;
}

export default function PlanGate({ minTier, feature, previewKey, children }: PlanGateProps) {
  const { org, loading } = useOrg();

  // Still resolving — render nothing to avoid flash.
  if (loading || !org) return null;

  if (planAtLeast(org.plan, minTier)) {
    return <>{children}</>;
  }

  // Gated: show illustrated preview if key provided, otherwise compact lock.
  if (previewKey) {
    return <PreviewUpsell previewKey={previewKey} minTier={minTier} />;
  }

  return <SimpleUpsell feature={feature} minTier={minTier} />;
}

// ---------------------------------------------------------------------------
// FreeLimitBanner — shown when a free org is at a hard limit.
// ---------------------------------------------------------------------------

interface FreeLimitBannerProps {
  current: number;
  max: number;
  resource: string;
}

export function FreeLimitBanner({ current, max, resource }: FreeLimitBannerProps) {
  const { org } = useOrg();

  if (!org || org.plan !== "free" || current < max) return null;

  return (
    <div className="mb-4 flex items-center gap-3 bg-amber-900/20 border border-amber-700/40 rounded-xl px-4 py-3">
      <span className="text-amber-400 text-lg shrink-0">⚠</span>
      <p className="text-sm text-amber-300 flex-1">
        You&apos;re using{" "}
        <span className="font-semibold">
          {current}/{max} {resource}
        </span>{" "}
        on the Free plan.{" "}
        <Link href="/pricing" className="underline hover:text-amber-200 transition-colors">
          Upgrade to Growth
        </Link>{" "}
        for unlimited {resource}.
      </p>
    </div>
  );
}
