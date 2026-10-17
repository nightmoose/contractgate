"use client";

import { useState } from "react";
import { useOrg, isPaidPlan } from "@/lib/org";
import { useSelfHosted } from "@/lib/use-paid-features";

// AISchemaGate tiers: the same plan ladder and prices as DataContractGate
// (decided 2026-10-12), in LLM wording. Limits are enforced
// by the gateway (src/plan.rs); keep these numbers in sync with it and with
// aischemagate-www/app/pricing. List only what ships today; see
// PROJECT_STATUS.md "AISchemaGate near-term TODO" for what is not built yet.
const TIERS = [
  {
    key: "developer",
    name: "Developer",
    tag: "Side projects and staging",
    price: "$0",
    priceSub: "free forever",
    features: [
      "100,000 validations / month",
      "Python & TypeScript LLMGate SDKs",
      "Hosted Rust validation engine (sub-15ms)",
      "Up to 3 schema contracts",
      "PII masking",
      "7-day validation log",
    ],
  },
  {
    key: "pro",
    name: "Pro",
    tag: "Agents in production",
    price: "$199",
    priceSub: "per month, or $166/mo billed annually ($1,992/yr)",
    features: [
      "25,000,000 validations / month",
      "Unlimited schema contracts",
      "Auto-repair re-prompt loop",
      "Payload quarantine & replay",
      "90-day validation log",
      "Everything in Developer",
    ],
  },
  {
    key: "enterprise",
    name: "Enterprise",
    tag: "Dedicated infrastructure",
    price: "Custom",
    priceSub: "contact us",
    features: [
      "Volume to match your traffic",
      "Dedicated or self-hosted deployment in your VPC",
      "Dedicated Slack or Teams support channel",
      "Everything in Pro",
    ],
  },
] as const;

export default function AisgPricing() {
  return (
    <div className="max-w-5xl mx-auto px-6 py-12">
      <div className="text-center mb-10">
        <p className="text-xs font-semibold uppercase tracking-widest text-green-400 mb-3">AISchemaGate</p>
        <h1 className="text-3xl font-bold text-slate-100">Pricing</h1>
        <p className="text-slate-400 mt-3 max-w-xl mx-auto">
          One validation is one LLM output checked against a contract. Pro stops at its monthly limit; upgrade to
          Enterprise for more.
        </p>
      </div>

      <div className="grid gap-5 md:grid-cols-3">
        {TIERS.map((t) => (
          <div
            key={t.key}
            className={`rounded-2xl border p-6 flex flex-col ${
              t.key === "pro" ? "border-green-600/60 bg-green-950/10" : "border-[#1f2937] bg-[#111827]"
            }`}
          >
            <h2 className="text-lg font-semibold text-slate-100">{t.name}</h2>
            <p className="text-xs text-slate-500 mt-0.5">{t.tag}</p>
            <p className="mt-5 text-4xl font-bold text-slate-100">{t.price}</p>
            <p className="text-xs text-slate-500 mt-1 mb-5">{t.priceSub}</p>
            {t.key === "developer" && <DeveloperCta />}
            {t.key === "pro" && <ProCta />}
            {t.key === "enterprise" && (
              <a
                href="mailto:datacontractgate@nightmoose.com?subject=AISchemaGate%20Enterprise"
                className="block text-center py-2.5 rounded-lg text-sm font-semibold mb-6 bg-[#1f2937] hover:bg-[#2a3449] text-slate-200"
              >
                Contact us
              </a>
            )}
            <ul className="space-y-2.5 text-sm text-slate-300">
              {t.features.map((f) => (
                <li key={f} className="flex gap-2">
                  <span className="text-green-400 shrink-0" aria-hidden="true">✓</span>
                  <span>{f}</span>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>

      <p className="text-center text-xs text-slate-500 mt-8">
        Enterprise inquiries: datacontractgate@nightmoose.com · Same account and API key as DataContractGate.
      </p>
    </div>
  );
}

function DeveloperCta() {
  const { org } = useOrg();
  if (org) {
    return (
      <a
        href="/"
        className="block text-center py-2.5 rounded-lg text-sm font-semibold mb-6 bg-[#1f2937] hover:bg-[#2a3449] text-slate-200"
      >
        Go to dashboard
      </a>
    );
  }
  return (
    <a
      href="/auth/signup?product=aischemagate"
      className="block text-center py-2.5 rounded-lg text-sm font-semibold mb-6 bg-[#1f2937] hover:bg-[#2a3449] text-slate-200"
    >
      Get an API key
    </a>
  );
}

function ProCta() {
  const { org, loading } = useOrg();
  const selfHosted = useSelfHosted();
  const [busy, setBusy] = useState(false);
  const [annual, setAnnual] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (loading) {
    // Same button, disabled until the account loads: no "Loading…" text in
    // the server-rendered page.
    return (
      <div className="mb-6">
        <button
          disabled
          className="w-full py-2.5 rounded-lg text-sm font-semibold bg-green-600 text-white opacity-60"
        >
          Start 30-day trial
        </button>
      </div>
    );
  }

  if (!org) {
    return (
      <a
        href="/auth/signup?product=aischemagate"
        className="block text-center py-2.5 rounded-lg text-sm font-semibold mb-6 bg-green-600 hover:bg-green-500 text-white"
      >
        Sign up, then upgrade
      </a>
    );
  }

  if (isPaidPlan(org.plan) || selfHosted === true) {
    return (
      <a
        href="/account"
        className="block text-center py-2.5 rounded-lg text-sm font-semibold mb-6 bg-[#1f2937] hover:bg-[#2a3449] text-slate-200"
      >
        Manage subscription
      </a>
    );
  }

  async function start() {
    setBusy(true);
    setError(null);
    try {
      const res = await fetch("/api/stripe/create-checkout-session", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ plan: "aisg_pro", annual }),
      });
      const data = await res.json();
      if (data.url) window.location.href = data.url;
      else setError(data.error || "Could not start checkout. Try again, or contact datacontractgate@nightmoose.com.");
    } catch {
      setError("Network error starting checkout. Try again.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mb-6">
      <button
        onClick={start}
        disabled={busy}
        className="w-full py-2.5 rounded-lg text-sm font-semibold transition-colors bg-green-600 hover:bg-green-500 text-white disabled:opacity-60"
      >
        {busy ? "Starting checkout…" : `Start 30-day trial (${annual ? "annual" : "monthly"})`}
      </button>
      <button
        onClick={() => setAnnual(!annual)}
        disabled={busy}
        className="w-full mt-2 py-1.5 text-xs rounded-lg text-green-400 hover:text-green-300 underline disabled:opacity-60"
      >
        {annual ? "or switch to monthly billing" : "or annual billing (save 17%)"}
      </button>
      {error && <p className="text-xs text-red-400 mt-2">{error}</p>}
      <p className="text-[10px] text-slate-400 mt-2 leading-snug">
        Free for 30 days, then {annual ? "$1,992 billed annually ($166/mo)" : "$199 billed monthly"}. Your
        subscription renews automatically until you cancel. Cancel
        anytime online from Account &rarr; Billing; cancel before the trial ends and you won&rsquo;t be charged. See
        our{" "}
        <a href="/terms" className="underline hover:text-slate-300">
          Terms
        </a>
        .
      </p>
      <p className="text-[10px] text-slate-500 mt-1">You will be redirected to Stripe Checkout.</p>
    </div>
  );
}
