"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { createClient } from "@/lib/supabase/client";

/**
 * Primary + secondary CTAs on a catalog detail page.
 *
 * "Use this contract" sends signed-out visitors to signup with
 * `?next=/catalog/{slug}/fork` (honored by the signup page), and signed-in
 * users straight to the fork step. "Copy curl" copies the public API one-liner.
 */
export default function DetailActions({
  slug,
  yamlUrl,
  curl,
}: {
  slug: string;
  yamlUrl: string;
  curl: string;
}) {
  const router = useRouter();
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);

  const forkPath = `/catalog/${slug}/fork`;

  async function handleUse() {
    setBusy(true);
    try {
      const supabase = createClient();
      const {
        data: { user },
      } = await supabase.auth.getUser();
      if (user) {
        router.push(forkPath);
      } else {
        router.push(`/auth/signup?next=${encodeURIComponent(forkPath)}`);
      }
    } catch {
      router.push(`/auth/signup?next=${encodeURIComponent(forkPath)}`);
    }
  }

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(curl);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard unavailable — ignore */
    }
  }

  return (
    <div className="flex flex-wrap items-center gap-3">
      <button
        onClick={handleUse}
        disabled={busy}
        className="px-5 py-2.5 bg-teal-600 hover:bg-teal-500 disabled:opacity-50 text-white text-sm font-semibold rounded-lg transition-colors"
      >
        {busy ? "Opening…" : "Use this contract"}
      </button>
      <a
        href={yamlUrl}
        className="px-4 py-2.5 bg-[#1f2937] hover:bg-[#2b3647] text-slate-200 text-sm font-medium rounded-lg transition-colors"
      >
        Download YAML
      </a>
      <button
        onClick={handleCopy}
        className="px-4 py-2.5 border border-[#1f2937] hover:border-slate-600 text-slate-300 text-sm font-medium rounded-lg transition-colors"
      >
        {copied ? "Copied!" : "Copy curl"}
      </button>
    </div>
  );
}
