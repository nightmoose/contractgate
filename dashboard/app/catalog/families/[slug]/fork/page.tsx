"use client";

/**
 * Catalog family fork — /catalog/families/{slug}/fork
 *
 * Signed-out visitors are sent to signup with this path as `next`. Signed-in
 * visitors copy the whole chain into their org, then open the private family.
 */
import { use, useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { createClient } from "@/lib/supabase/client";
import { forkPublicFamily } from "@/lib/api";

export default function CatalogFamilyForkPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = use(params);
  const router = useRouter();
  const [status, setStatus] = useState("Setting up your pipeline…");
  const [error, setError] = useState<string | null>(null);
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return;
    started.current = true;

    (async () => {
      const forkPath = `/catalog/families/${slug}/fork`;
      try {
        const supabase = createClient();
        const {
          data: { user },
        } = await supabase.auth.getUser();
        if (!user) {
          router.replace(`/auth/signup?next=${encodeURIComponent(forkPath)}`);
          return;
        }

        setStatus("Forking the pipeline into your org…");
        const fork = await forkPublicFamily(slug, {});
        setStatus("Opening your family…");
        router.replace(`/contracts/families/${fork.family.id}`);
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
            Couldn&rsquo;t fork this pipeline
          </h1>
          <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-3 mb-4">
            {error}
          </p>
          <Link
            href={`/catalog/families/${slug}`}
            className="text-teal-400 hover:text-teal-300 text-sm"
          >
            ← Back to the pipeline
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
