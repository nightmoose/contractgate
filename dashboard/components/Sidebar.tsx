"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import clsx from "clsx";

const NAV = [
  { href: "/",            label: "Dashboard",  icon: "⬡" },
  { href: "/contracts",   label: "Contracts",  icon: "📋" },
  { href: "/audit",       label: "Audit Log",  icon: "🔍" },
  { href: "/playground",  label: "Playground", icon: "🧪" },
];

export default function Sidebar() {
  const pathname = usePathname();

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
      <nav className="flex-1 p-4 space-y-1">
        {NAV.map(({ href, label, icon }) => (
          <Link
            key={href}
            href={href}
            className={clsx(
              "flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm font-medium transition-colors",
              pathname === href
                ? "bg-green-900/30 text-green-400 border border-green-800/50"
                : "text-slate-400 hover:text-slate-200 hover:bg-[#1f2937]"
            )}
          >
            <span className="text-base">{icon}</span>
            {label}
          </Link>
        ))}
      </nav>

      {/* Footer */}
      <div className="p-4 border-t border-[#1f2937]">
        <p className="text-xs text-slate-600">v0.1.0</p>
      </div>
    </aside>
  );
}
