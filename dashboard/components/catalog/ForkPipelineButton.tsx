"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { createClient } from "@/lib/supabase/client";

/**
 * "Fork this pipeline" on a catalog family.
 *
 * Signed-out visitors go to signup with `?next=/catalog/families/{slug}/fork`.
 * Signed-in visitors go straight to that fork step, which copies the chain
 * into their org.
 */
export default function ForkPipelineButton({
  slug,
  label = "Fork this pipeline",
}: {
  slug: string;
  label?: string;
}) {
  const router = useRouter();
  const [busy, setBusy] = useState(false);
  const forkPath = `/catalog/families/${slug}/fork`;

  async function handleFork() {
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

  return (
    <button
      type="button"
      onClick={handleFork}
      disabled={busy}
      className="px-5 py-2.5 bg-teal-600 hover:bg-teal-500 disabled:opacity-50 text-white text-sm font-semibold rounded-lg transition-colors"
    >
      {busy ? "Opening…" : label}
    </button>
  );
}
