"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import type { OpenDataContract } from "@/lib/api";
import { catalogCardLine } from "@/lib/catalog-copy";
import { CATALOG_CATEGORIES } from "@/lib/catalog-categories";

/** Card grid with client-side search + category filter over the catalog list. */
export default function CatalogBrowser({
  contracts,
}: {
  contracts: OpenDataContract[];
}) {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<string | null>(null);

  // Only show category chips that actually have contracts behind them.
  const categories = useMemo(
    () => CATALOG_CATEGORIES.filter((c) => contracts.some((k) => k.category === c)),
    [contracts]
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return contracts.filter((c) => {
      if (category && c.category !== category) return false;
      if (!q) return true;
      return (
        (c.vendor ?? "").toLowerCase().includes(q) ||
        (c.description ?? "").toLowerCase().includes(q) ||
        (c.slug ?? "").toLowerCase().includes(q) ||
        c.name.toLowerCase().includes(q)
      );
    });
  }, [contracts, query, category]);

  return (
    <div>
      {/* Controls */}
      <div className="flex flex-col sm:flex-row sm:items-center gap-3 mb-5">
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search Stripe, Shopify, invoices…"
          className="w-full sm:max-w-xs bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-200 placeholder-slate-600 outline-none focus:border-teal-600 transition-colors"
          aria-label="Search the catalog"
        />
        <div className="flex flex-wrap gap-1.5">
          <button
            onClick={() => setCategory(null)}
            className={`px-3 py-1 text-xs font-medium rounded-full border transition-colors ${
              category === null
                ? "bg-teal-900/40 text-teal-300 border-teal-700/50"
                : "text-slate-400 border-[#1f2937] hover:text-slate-200"
            }`}
          >
            All
          </button>
          {categories.map((c) => (
            <button
              key={c}
              onClick={() => setCategory(c)}
              className={`px-3 py-1 text-xs font-medium rounded-full border transition-colors ${
                category === c
                  ? "bg-teal-900/40 text-teal-300 border-teal-700/50"
                  : "text-slate-400 border-[#1f2937] hover:text-slate-200"
              }`}
            >
              {c}
            </button>
          ))}
        </div>
      </div>

      {/* Cards */}
      {filtered.length === 0 ? (
        <p className="text-sm text-slate-500 py-10 text-center">
          No contracts match that search.
        </p>
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
          {filtered.map((c) => (
            <Link
              key={c.id}
              href={`/catalog/${c.slug}`}
              className="group bg-[#111827] border border-[#1f2937] rounded-xl p-5 hover:border-teal-700/50 transition-colors flex flex-col"
            >
              <div className="flex items-center justify-between gap-2 mb-2">
                <span className="text-[10px] uppercase tracking-wider text-teal-400 bg-teal-900/20 border border-teal-800/40 rounded-full px-2 py-0.5">
                  {c.category ?? "Contract"}
                </span>
              </div>
              <h3 className="text-sm font-semibold text-slate-100 group-hover:text-teal-300 transition-colors">
                {c.vendor ?? c.name}
              </h3>
              <p className="text-xs text-slate-500 mt-1.5 line-clamp-3 flex-1">
                {c.description}
              </p>
              <p className="text-[11px] text-slate-600 mt-3">
                {catalogCardLine(c.slug, c.vendor)}
              </p>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
