"use client";

/**
 * Catalog fork step — /catalog/{slug}/fork
 *
 * Reached from a detail page's "Use this contract" CTA (directly when signed in,
 * via /auth/signup?next=... when not). Forks the public contract into the
 * caller's org, pre-loads the new YAML into the validation playground via
 * sessionStorage (the playground reads `playground_yaml` / `playground_contract_id`
 * on mount), and sends the user there to upload their client's file.
 */
import { use, useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { createClient } from "@/lib/supabase/client";
import { getOpenDataContract, forkPublicContract } from "@/lib/api";
import {
  catalogSampleEvent,
  PLAYGROUND_CONTRACT_ID_KEY,
  PLAYGROUND_EVENT_KEY,
  PLAYGROUND_YAML_KEY,
} from "@/lib/playground-prefill";

export default function CatalogForkPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = use(params);
  const router = useRouter();
  const [status, setStatus] = useState("Setting up your contract…");
  const [error, setError] = useState<string | null>(null);
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return;
    started.current = true;

    (async () => {
      const forkPath = `/catalog/${slug}/fork`;
      try {
        const supabase = createClient();
        const {
          data: { user },
        } = await supabase.auth.getUser();
        if (!user) {
          router.replace(`/auth/signup?next=${encodeURIComponent(forkPath)}`);
          return;
        }

        setStatus("Loading the contract…");
        const detail = await getOpenDataContract(slug);

        setStatus("Forking into your org…");
        const fork = await forkPublicContract(detail.id, {
          name: detail.slug ?? detail.name,
          ...(detail.description ? { description: detail.description } : {}),
        });

        // Hand the forked YAML to the playground (same keys the "Test in
        // Playground" flow uses). Also hand the first published synthetic
        // sample so Validate passes — the playground's example_* guess does
        // not satisfy these contracts.
        sessionStorage.setItem(PLAYGROUND_YAML_KEY, detail.contract_yaml);
        sessionStorage.setItem(PLAYGROUND_CONTRACT_ID_KEY, fork.contract_id);
        const sample = catalogSampleEvent(detail.sample_json);
        if (sample) sessionStorage.setItem(PLAYGROUND_EVENT_KEY, sample);

        setStatus("Opening the validation playground…");
        router.replace("/playground");
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
  }, [slug, router]);

  return (
    <div className="max-w-md mx-auto mt-24 text-center">
      {error ? (
        <div>
          <p className="text-4xl mb-4">⚠️</p>
          <h1 className="text-lg font-semibold text-slate-100 mb-2">
            Couldn&rsquo;t fork this contract
          </h1>
          <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-3 mb-4">
            {error}
          </p>
          <Link href={`/catalog/${slug}`} className="text-teal-400 hover:text-teal-300 text-sm">
            ← Back to the contract
          </Link>
        </div>
      ) : (
        <div>
          <div className="animate-spin h-8 w-8 border-2 border-teal-500 border-t-transparent rounded-full mx-auto mb-5" />
          <p className="text-sm text-slate-400">{status}</p>
        </div>
      )}
    </div>
  );
}
