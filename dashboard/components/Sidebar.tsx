"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import clsx from "clsx";
import { HelpTarget } from "@/components/help/HelpTarget";
import { HelpToggle } from "@/components/help/HelpToggle";
import { useHelp } from "@/components/help/HelpProvider";
import { useBrand } from "@/components/BrandProvider";
import { AISG_DOCS_URL, BRAND_NAME, BRAND_TAGLINE } from "@/lib/brand";

// AISchemaGate accounts only see what their product uses (LLM output
// validation). Hidden: DataContractGate docs/demos and the data-pipeline tools
// (Catalog, Scorecard, Scaffold, Workbench). The pages still exist by URL.
const AISG_HIDDEN = new Set([
  "/stream-demo",
  "/docs/kafka-connect",
  "/docs/python-sdk",
  "/catalog",
  "/scorecard",
  "/scaffold",
  "/workbench",
]);
const AISG_DOCS = { href: AISG_DOCS_URL, label: "Docs", icon: "📘", helpId: "nav.docs" };

const PUBLIC_NAV = [
  { href: "/stream-demo",        label: "Stream Demo",  icon: "⚡", helpId: "nav.stream-demo" },
  { href: "/docs/kafka-connect", label: "Kafka Connect", icon: "🔗", helpId: "nav.kafka-connect" },
  { href: "/docs/fivetran", label: "Fivetran", icon: "⟳", helpId: "nav.fivetran" },
  { href: "/docs/python-sdk",    label: "Python SDK",    icon: "🐍", helpId: "nav.python-sdk" },
  { href: "/pricing",            label: "Pricing",       icon: "💳", helpId: "nav.pricing" },
];

const ACCOUNT_NAV = [
  { href: "/",           label: "Dashboard",  icon: "⬡", helpId: "nav.dashboard" },
  { href: "/contracts",  label: "Contracts",  icon: "📋", helpId: "nav.contracts" },
  { href: "/catalog",    label: "Catalog",    icon: "📥", helpId: "nav.catalog" },
  { href: "/scorecard",  label: "Scorecard",  icon: "📊", helpId: "nav.scorecard" },
  { href: "/audit",      label: "Audit Log",  icon: "🔍", helpId: "nav.audit" },
  { href: "/scaffold",   label: "Scaffold",   icon: "🏗️", helpId: "nav.scaffold" },
  { href: "/workbench",  label: "Workbench",  icon: "🔬", helpId: "nav.workbench" },
  { href: "/playground", label: "Playground", icon: "🧪", helpId: "nav.playground" },
  { href: "/account",    label: "Account",    icon: "🔑", helpId: "nav.account" },
];

function NavLink({ href, label, icon, helpId }: { href: string; label: string; icon: string; helpId: string }) {
  const pathname = usePathname();
  const active = pathname === href;
  return (
    <HelpTarget id={helpId}>
      <Link
        href={href}
        className={clsx(
          "flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm font-medium transition-colors",
          active
            ? "bg-green-900/30 text-green-400 border border-green-800/50"
            : "text-slate-400 hover:text-slate-200 hover:bg-[#1f2937]"
        )}
      >
        <span className="text-base">{icon}</span>
        {label}
      </Link>
    </HelpTarget>
  );
}

export default function Sidebar() {
  const { mode, toggle } = useHelp();
  const brand = useBrand();
  const aisg = brand === "aisg";
  const publicNav = aisg ? [AISG_DOCS, ...PUBLIC_NAV.filter((i) => !AISG_HIDDEN.has(i.href))] : PUBLIC_NAV;
  const accountNav = aisg ? ACCOUNT_NAV.filter((i) => !AISG_HIDDEN.has(i.href)) : ACCOUNT_NAV;
  return (
    <aside className="hidden md:flex fixed left-0 top-0 h-full w-64 bg-[#111827] border-r border-[#1f2937] flex-col z-50">
      {/* Logo */}
      <div className="p-6 border-b border-[#1f2937]">
        <div className="flex flex-col">
          <span className="text-xl font-bold text-green-400">{BRAND_NAME[brand]}</span>
          {aisg && <span className="text-[11px] text-slate-500">by DataContractGate</span>}
        </div>
        <div className="mt-1 flex items-center gap-1">
          <span className="text-xs bg-green-900/40 text-green-400 border border-green-700/50 px-2 py-0.5 rounded-full font-medium">
            Patent Pending
          </span>
        </div>
        <p className="mt-2 text-xs text-slate-500">
          {BRAND_TAGLINE[brand]}
        </p>
      </div>

      {/* Navigation */}
      <nav className="flex-1 p-4 overflow-y-auto">
        {/* Public section */}
        <div className="space-y-1">
          {publicNav.map((item) => (
            <NavLink key={item.href} {...item} />
          ))}
        </div>

        {/* Divider */}
        <div className="my-4 flex items-center gap-2">
          <div className="flex-1 h-px bg-[#1f2937]" />
          <span className="text-[10px] text-slate-600 uppercase tracking-widest font-medium">
            Your Account
          </span>
          <div className="flex-1 h-px bg-[#1f2937]" />
        </div>

        {/* Account section */}
        <div className="space-y-1">
          {accountNav.map((item) => (
            <NavLink key={item.href} {...item} />
          ))}
        </div>
      </nav>

      {/* Footer */}
      <div className="p-4 border-t border-[#1f2937] space-y-3">
        <HelpToggle mode={mode} onToggle={toggle} />
        <p className="text-xs text-slate-600">v{process.env.NEXT_PUBLIC_APP_VERSION}</p>
      </div>
    </aside>
  );
}
