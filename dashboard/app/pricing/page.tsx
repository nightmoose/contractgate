"use client";

import { useState } from "react";
import clsx from "clsx";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface Feature {
  label: string;
  selfHosted: string | boolean;
  free: string | boolean;
  growth: string | boolean;
  enterprise: string | boolean;
}

// ---------------------------------------------------------------------------
// Pricing data
// ---------------------------------------------------------------------------

const TIERS = [
  {
    key: "selfHosted",
    name: "Self-Hosted Free",
    tagline: "Run it yourself, forever",
    price: { monthly: "Free", annual: "Free" },
    priceSub: { monthly: "git clone && make demo", annual: "git clone && make demo" },
    cta: "Get the repo",
    ctaHref: "https://github.com/nightmoose/contractgate",
    highlight: false,
    color: "border-[#1f2937]",
    badge: "OSS",
  },
  {
    key: "free",
    name: "Cloud Free",
    tagline: "Explore & prototype",
    price: { monthly: "$0", annual: "$0" },
    cta: "Start free",
    ctaHref: "https://app.datacontractgate.com/signup",
    highlight: false,
    color: "border-[#1f2937]",
    badge: null,
  },
  {
    key: "growth",
    name: "Growth",
    tagline: "Production workloads",
    price: { monthly: "$299", annual: "$249" },
    priceSub: { monthly: "per month", annual: "per month, billed annually" },
    cta: "Start 14-day trial",
    ctaHref: "https://app.datacontractgate.com/signup?plan=growth",
    highlight: true,
    color: "border-green-700/60",
    badge: "Most popular",
  },
  {
    key: "enterprise",
    name: "Enterprise",
    tagline: "Scale without limits",
    price: { monthly: "Custom", annual: "Custom" },
    priceSub: { monthly: "contact us", annual: "contact us" },
    cta: "Talk to us",
    ctaHref: "mailto:sales@contractgate.io",
    highlight: false,
    color: "border-[#1f2937]",
    badge: null,
  },
];

const FEATURES: Feature[] = [
  { label: "Events validated / month",         selfHosted: "Unlimited (local)", free: "1M",              growth: "50M",        enterprise: "Unlimited" },
  { label: "Active contracts",                 selfHosted: "3 (starter)",       free: "3",               growth: "Unlimited",  enterprise: "Unlimited" },
  { label: "Contract versions",                selfHosted: true,                free: "3 per contract",  growth: "Unlimited",  enterprise: "Unlimited" },
  { label: "Ingest API",                       selfHosted: true,                free: true,              growth: true,         enterprise: true },
  { label: "Playground (test without save)",   selfHosted: true,                free: true,              growth: true,         enterprise: true },
  { label: "Audit log retention",              selfHosted: "Local Postgres",    free: "7 days",          growth: "90 days",    enterprise: "Custom" },
  { label: "Stream demo dashboard",            selfHosted: true,                free: true,              growth: true,         enterprise: true },
  { label: "Batch ingest",                     selfHosted: true,                free: false,             growth: true,         enterprise: true },
  { label: "Data-parallel validation (Rayon)", selfHosted: true,                free: false,             growth: true,         enterprise: true },
  { label: "Auth & API key management",        selfHosted: false,               free: true,              growth: true,         enterprise: true },
  { label: "Multi-tenancy",                    selfHosted: false,               free: false,             growth: true,         enterprise: true },
  { label: "PII transform rules (RFC-004)",    selfHosted: false,               free: false,             growth: true,         enterprise: true },
  { label: "Quarantine replay (RFC-003)",      selfHosted: false,               free: false,             growth: true,         enterprise: true },
  { label: "Visual contract builder",          selfHosted: false,               free: false,             growth: true,         enterprise: true },
  { label: "Contract generator (JSON → YAML)", selfHosted: false,               free: false,             growth: true,         enterprise: true },
  { label: "Semantic versioning + promotion",  selfHosted: false,               free: false,             growth: true,         enterprise: true },
  { label: "GitHub sync",                      selfHosted: false,               free: false,             growth: true,         enterprise: true },
  { label: "Team invites & roles",             selfHosted: false,               free: false,             growth: true,         enterprise: true },
  { label: "SSO / SAML",                       selfHosted: false,               free: false,             growth: false,        enterprise: true },
  { label: "Custom SLA",                       selfHosted: false,               free: false,             growth: false,        enterprise: true },
  { label: "Audit log export (S3 / GCS)",      selfHosted: false,               free: false,             growth: false,        enterprise: true },
  { label: "Dedicated deployment",             selfHosted: false,               free: false,             growth: false,        enterprise: true },
  { label: "Priority support + SRE on-call",   selfHosted: false,               free: false,             growth: false,        enterprise: true },
  { label: "Custom contract templates",        selfHosted: false,               free: false,             growth: false,        enterprise: true },
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function CheckIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" aria-hidden>
      <circle cx="8" cy="8" r="8" fill="rgba(61,220,132,0.15)" />
      <path d="M4.5 8l2.5 2.5 4.5-4.5" stroke="#3ddc84" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function CrossIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" aria-hidden>
      <circle cx="8" cy="8" r="8" fill="rgba(255,255,255,0.04)" />
      <path d="M5.5 5.5l5 5M10.5 5.5l-5 5" stroke="#374151" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}

function FeatureCell({ value }: { value: string | boolean }) {
  if (value === true)  return <div className="flex justify-center"><CheckIcon /></div>;
  if (value === false) return <div className="flex justify-center"><CrossIcon /></div>;
  return <div className="text-center text-xs text-slate-300 font-medium">{value}</div>;
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

export default function PricingPage() {
  const [annual, setAnnual] = useState(false);

  return (
    <div className="space-y-12 pb-16">
      {/* Header */}
      <div className="text-center pt-4">
        <div className="inline-flex items-center gap-2 bg-green-900/30 border border-green-700/40 rounded-full px-3 py-1 text-xs text-green-400 font-medium mb-4">
          <span className="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse" />
          Patent Pending · Rust-native · &lt;15 ms p99
        </div>
        <h1 className="text-4xl font-bold tracking-tight">
          Simple, transparent pricing
        </h1>
        <p className="mt-3 text-slate-400 text-lg max-w-xl mx-auto">
          Start free. Scale without surprises. Enterprise teams get dedicated deployments and custom SLAs.
        </p>

        {/* Annual toggle */}
        <div className="flex items-center justify-center gap-3 mt-6">
          <span className={clsx("text-sm", !annual ? "text-slate-200" : "text-slate-500")}>Monthly</span>
          <button
            onClick={() => setAnnual((a) => !a)}
            className={clsx(
              "relative w-11 h-6 rounded-full transition-colors",
              annual ? "bg-green-600" : "bg-slate-700"
            )}
            aria-label="Toggle annual billing"
          >
            <span className={clsx(
              "absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white transition-transform",
              annual && "translate-x-5"
            )} />
          </button>
          <span className={clsx("text-sm", annual ? "text-slate-200" : "text-slate-500")}>
            Annual
            <span className="ml-1.5 text-xs bg-green-900/40 text-green-400 border border-green-700/40 px-1.5 py-0.5 rounded-full font-medium">
              Save 17%
            </span>
          </span>
        </div>
      </div>

      {/* Edition labels */}
      <div className="flex items-center gap-4 -mb-6">
        <div className="flex items-center gap-2">
          <span className="w-2.5 h-2.5 rounded-full bg-slate-600" />
          <span className="text-xs text-slate-500 font-medium uppercase tracking-wider">Self-Hosted</span>
        </div>
        <div className="flex-1 border-t border-dashed border-[#1f2937]" />
        <div className="flex items-center gap-2">
          <span className="w-2.5 h-2.5 rounded-full bg-green-600" />
          <span className="text-xs text-slate-500 font-medium uppercase tracking-wider">Cloud</span>
        </div>
      </div>

      {/* Tier cards */}
      <div className="grid grid-cols-1 md:grid-cols-4 gap-5">
        {TIERS.map((tier) => (
          <div
            key={tier.key}
            className={clsx(
              "relative bg-[#111827] border rounded-2xl p-7 flex flex-col",
              tier.color,
              tier.highlight && "ring-1 ring-green-700/40"
            )}
          >
            {tier.badge === "OSS" && (
              <div className="absolute -top-3 left-1/2 -translate-x-1/2">
                <span className="bg-[#1f2937] border border-[#374151] text-slate-300 text-xs font-semibold px-3 py-1 rounded-full shadow">
                  Open Source
                </span>
              </div>
            )}
            {tier.badge && tier.badge !== "OSS" && (
              <div className="absolute -top-3 left-1/2 -translate-x-1/2">
                <span className="bg-green-600 text-white text-xs font-semibold px-3 py-1 rounded-full shadow">
                  {tier.badge}
                </span>
              </div>
            )}

            <div className="mb-6">
              <h2 className="text-lg font-semibold">{tier.name}</h2>
              <p className="text-sm text-slate-500 mt-0.5">{tier.tagline}</p>
            </div>

            <div className="mb-6">
              <div className="flex items-end gap-1">
                <span className="text-4xl font-bold">
                  {annual ? tier.price.annual : tier.price.monthly}
                </span>
              </div>
              {tier.priceSub && (
                <p className="text-xs text-slate-500 mt-1">
                  {annual ? tier.priceSub.annual : tier.priceSub.monthly}
                </p>
              )}
            </div>

            <a
              href={tier.ctaHref}
              className={clsx(
                "block text-center py-2.5 rounded-lg text-sm font-semibold transition-colors mb-6",
                tier.highlight
                  ? "bg-green-600 hover:bg-green-500 text-white"
                  : "bg-[#1f2937] hover:bg-[#2a3449] text-slate-200"
              )}
            >
              {tier.cta}
            </a>

            {/* Quick feature list for card */}
            <ul className="space-y-2.5 text-sm flex-1">
              {FEATURES.slice(0, 8).map((f) => {
                const val = f[tier.key as keyof Feature];
                if (val === false) return null;
                return (
                  <li key={f.label} className="flex items-start gap-2 text-slate-400">
                    <span className="mt-0.5 shrink-0"><CheckIcon size={14} /></span>
                    <span>
                      {typeof val === "string" && val !== "true" ? (
                        <><span className="text-slate-200 font-medium">{val}</span> {f.label.replace(/^[0-9M]+\s*/, "")}</>
                      ) : f.label}
                    </span>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>

      {/* Full feature comparison table */}
      <div>
        <h2 className="text-xl font-semibold mb-6 text-center text-slate-300">Full feature comparison</h2>
        <div className="bg-[#111827] border border-[#1f2937] rounded-2xl overflow-hidden">
          {/* Table header */}
          <div className="grid grid-cols-5 border-b border-[#1f2937]">
            <div className="px-5 py-4 text-xs text-slate-500 uppercase tracking-wider">Feature</div>
            {TIERS.map((t) => (
              <div key={t.key} className={clsx("px-4 py-4 text-center", t.highlight && "bg-green-900/10")}>
                <p className="text-sm font-semibold">{t.name}</p>
                {t.key === "selfHosted" && (
                  <span className="inline-block mt-1 text-xs text-slate-500">self-hosted</span>
                )}
                {t.key !== "selfHosted" && (
                  <span className="inline-block mt-1 text-xs text-slate-500">cloud</span>
                )}
              </div>
            ))}
          </div>

          {/* Table rows */}
          {FEATURES.map((f, i) => (
            <div
              key={f.label}
              className={clsx(
                "grid grid-cols-5 border-b border-[#1f2937]/50 last:border-0",
                i % 2 === 1 && "bg-[#0a0d12]/40"
              )}
            >
              <div className="px-5 py-3.5 text-sm text-slate-400">{f.label}</div>
              {TIERS.map((t) => (
                <div key={t.key} className={clsx("px-4 py-3.5 flex items-center justify-center", t.highlight && "bg-green-900/5")}>
                  <FeatureCell value={f[t.key as keyof Feature]} />
                </div>
              ))}
            </div>
          ))}
        </div>
      </div>

      {/* FAQ strip */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-5">
        {[
          {
            q: "What counts as an event?",
            a: "Any JSON object submitted to a POST /ingest/{contract_id} endpoint. Batch calls count each item in the batch individually.",
          },
          {
            q: "Can I switch plans mid-month?",
            a: "Yes. Upgrades take effect immediately (prorated). Downgrades apply at the start of your next billing cycle.",
          },
          {
            q: "Is the Free tier time-limited?",
            a: "No. The Free tier is permanent. You only need to upgrade when you exceed 1M events/month or need Growth features.",
          },
          {
            q: "How does Enterprise deployment work?",
            a: "We support dedicated cloud (VPC), on-prem (Kubernetes), and air-gapped deployments. Contact us for architecture details.",
          },
          {
            q: "Do you offer a trial?",
            a: "Growth comes with a 14-day free trial, no credit card required. Enterprise prospects get a POC environment on request.",
          },
          {
            q: "What does 'patent pending' mean for my stack?",
            a: "The semantic contract enforcement methodology is patent pending (US filing). Your use is licensed; you are protected from third-party IP claims.",
          },
          {
            q: "What's the difference between Self-Hosted Free and Cloud Free?",
            a: "Self-Hosted Free runs the full gateway binary on your machine — unlimited local events, no account needed. Cloud Free is the managed SaaS tier with auth, API keys, and cloud retention. Self-Hosted Free has no Cloud features (multi-tenancy, GitHub sync, etc.).",
          },
        ].map((item) => (
          <div key={item.q} className="bg-[#111827] border border-[#1f2937] rounded-xl p-5">
            <p className="text-sm font-semibold text-slate-200 mb-1.5">{item.q}</p>
            <p className="text-sm text-slate-500 leading-relaxed">{item.a}</p>
          </div>
        ))}
      </div>

      {/* Enterprise CTA */}
      <div className="bg-gradient-to-br from-[#0d1f2d] to-[#111827] border border-[#1f2937] rounded-2xl p-8 text-center">
        <div className="text-2xl mb-2">🏢</div>
        <h3 className="text-xl font-bold mb-2">Building something large-scale?</h3>
        <p className="text-slate-400 text-sm mb-5 max-w-md mx-auto">
          Enterprises get custom event volumes, dedicated deployments, SSO, audit export, and a
          named SRE. We&apos;ll scope your POC in one call.
        </p>
        <a
          href="mailto:sales@contractgate.io"
          className="inline-block bg-green-600 hover:bg-green-500 text-white font-semibold text-sm px-6 py-3 rounded-lg transition-colors"
        >
          Talk to sales →
        </a>
      </div>
    </div>
  );
}
