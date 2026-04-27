"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import clsx from "clsx";

const PUBLIC_NAV = [
  { href: "/stream-demo",        label: "Stream Demo",  icon: "⚡" },
  { href: "/docs/kafka-connect", label: "Kafka Connect", icon: "🔗" },
  { href: "/docs/python-sdk",    label: "Python SDK",    icon: "🐍" },
  { href: "/pricing",            label: "Pricing",       icon: "💳" },
];

const ACCOUNT_NAV = [
  { href: "/",           label: "Dashboard",  icon: "⬡" },
  { href: "/contracts",  label: "Contracts",  icon: "📋" },
  { href: "/audit",      label: "Audit Log",  icon: "🔍" },
  { href: "/playground", label: "Playground", icon: "🧪" },
  { href: "/account",    label: "Account",    icon: "🔑" },
];

function NavLink({ href, label, icon }: { href: string; label: string; icon: string }) {
  const pathname = usePathname();
  const active = pathname === href;
  return (
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
  );
}

export default function Sidebar() {
  return (
    <aside className="fixed left-0 top-0 h-full w-64 bg-[#111827] border-r border-[#1f2937] flex flex-col z-50">
      {/* Logo */}
      <div className="p-6 border-b border-[#1f2937]">
        <div className="flex items-center gap-2">
          <span className="text-2xl font-bold text-green-400">ContractGate</span>
        </div>
        <div className="mt-1 flex items-center gap-1">
          <span className="text-xs bg-green-900/40 text-green-400 border border-green-700/50 px-2 py-0.5 rounded-full font-medium">
            Patent Pending
          </span>
        </div>
        <p className="mt-2 text-xs text-slate-500">
          Semantic contract enforcement at ingestion
        </p>
      </div>

      {/* Navigation */}
      <nav className="flex-1 p-4 overflow-y-auto">
        {/* Public section */}
        <div className="space-y-1">
          {PUBLIC_NAV.map((item) => (
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
          {ACCOUNT_NAV.map((item) => (
            <NavLink key={item.href} {...item} />
          ))}
        </div>
      </nav>

      {/* Footer */}
      <div className="p-4 border-t border-[#1f2937]">
        <p className="text-xs text-slate-600">v0.1.0</p>
      </div>
    </aside>
  );
}
