"use client";

/**
 * PlanGate — RFC-045: feature gating by billing plan tier.
 *
 * Usage:
 *   <PlanGate minTier="growth" feature="Visual Builder">
 *     <VisualBuilder ... />
 *   </PlanGate>
 *
 * Renders children when org.plan meets minTier.
 * Renders an UpsellCard otherwise.
 *
 * If the org is still loading, renders nothing (avoids flash of upsell).
 */

import Link from "next/link";
import { useOrg, planAtLeast, type PlanTier } from "@/lib/org";

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

const TIER_CTA: Record<PlanTier, { label: string; href: string }> = {
  free: { label: "Upgrade to Growth →", href: "/pricing" },
  growth: { label: "Upgrade to Growth →", href: "/pricing" },
  enterprise: { label: "Talk to sales →", href: "mailto:sales@contractgate.io" },
};

// ---------------------------------------------------------------------------
// UpsellCard
// ---------------------------------------------------------------------------

function UpsellCard({
  feature,
  minTier,
}: {
  feature: string;
  minTier: PlanTier;
}) {
  const cta = TIER_CTA[minTier];

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
        href={cta.href}
        className="px-5 py-2.5 bg-green-600 hover:bg-green-500 text-white text-sm font-semibold rounded-lg transition-colors"
      >
        {cta.label}
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
  /** Human-readable feature name shown in the upsell card. */
  feature: string;
  children: React.ReactNode;
}

export default function PlanGate({ minTier, feature, children }: PlanGateProps) {
  const { org, loading } = useOrg();

  // Still resolving — render nothing to avoid flash.
  if (loading || !org) return null;

  if (planAtLeast(org.plan, minTier)) {
    return <>{children}</>;
  }

  return <UpsellCard feature={feature} minTier={minTier} />;
}

// ---------------------------------------------------------------------------
// FreeLimitBanner — shown when a free org is near or at a hard limit.
// ---------------------------------------------------------------------------

interface FreeLimitBannerProps {
  /** Current count of the limited resource. */
  current: number;
  /** Maximum allowed on the free plan. */
  max: number;
  /** E.g. "contracts" or "versions" */
  resource: string;
}

export function FreeLimitBanner({ current, max, resource }: FreeLimitBannerProps) {
  const { org } = useOrg();

  // Only show for free tier orgs that are at or near the limit.
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
