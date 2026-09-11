"use client";

import Link from "next/link";

const STEPS = [
  {
    n: "1",
    text: "Create a contract — YAML, Visual Builder, or infer from a sample.",
  },
  {
    n: "2",
    text: "Promote a version to Stable so unpinned ingest has somewhere to route.",
  },
  {
    n: "3",
    text: "POST an event to /v1/ingest/{id}. Pass lands in Audit; fail lands in Quarantine.",
  },
];

export function FirstRunLoop({
  title,
  hint,
  cta,
}: {
  title: string;
  hint?: string;
  cta?: { href: string; label: string };
}) {
  return (
    <div className="max-w-md mx-auto text-center py-4">
      <p className="text-sm text-slate-300">{title}</p>
      {hint && <p className="text-xs text-slate-500 mt-2 leading-relaxed">{hint}</p>}
      <ol className="mt-4 text-left space-y-2">
        {STEPS.map((s) => (
          <li key={s.n} className="flex gap-2 text-xs text-slate-400 leading-relaxed">
            <span className="text-green-500 font-semibold shrink-0">{s.n}.</span>
            <span>{s.text}</span>
          </li>
        ))}
      </ol>
      {cta && (
        <Link
          href={cta.href}
          className="inline-block mt-4 text-xs text-green-400 hover:text-green-300"
        >
          {cta.label}
        </Link>
      )}
    </div>
  );
}
