"use client";

/**
 * /auth/accept-invite?token=<uuid>
 *
 * Redemption page for the org-invite flow (RFC-001 §Invite Flow).
 *
 * Lifecycle:
 *  - No token → "invalid invite link" message.
 *  - Not signed in → bounce to /auth/login?next=/auth/accept-invite?token=...
 *    Login flow returns here, at which point we POST the token to
 *    /api/invites/accept.
 *  - Signed in, token valid, email matches → membership created, redirect to /.
 *  - Signed in, email mismatch → tell the user which account the invite is for
 *    and offer a sign-out button.
 *  - Expired / revoked / already-accepted → terminal error message.
 */

import { useEffect, useState, Suspense } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { createClient } from "@/lib/supabase/client";

type Status =
  | { kind: "loading" }
  | { kind: "success"; orgId: string }
  | { kind: "no-token" }
  | { kind: "error"; message: string; inviteEmail?: string };

function AcceptInviteInner() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const token = searchParams.get("token");
  const [status, setStatus] = useState<Status>({ kind: "loading" });

  useEffect(() => {
    if (!token) {
      setStatus({ kind: "no-token" });
      return;
    }

    let cancelled = false;
    (async () => {
      const supabase = createClient();
      const {
        data: { user },
      } = await supabase.auth.getUser();

      if (!user) {
        // Bounce through login; on success the user lands back here and the
        // effect runs again with a session.
        const next = `/auth/accept-invite?token=${encodeURIComponent(token)}`;
        router.replace(`/auth/login?next=${encodeURIComponent(next)}`);
        return;
      }

      try {
        const res = await fetch("/api/invites/accept", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ token }),
        });
        const body = (await res.json()) as {
          org_id?: string;
          error?: string;
          invite_email?: string;
        };
        if (cancelled) return;

        if (res.ok && body.org_id) {
          setStatus({ kind: "success", orgId: body.org_id });
          // Brief pause so the user sees the success state, then bounce to /.
          setTimeout(() => router.replace("/"), 1200);
        } else {
          setStatus({
            kind: "error",
            message: body.error ?? "Failed to accept invite",
            inviteEmail: body.invite_email,
          });
        }
      } catch (e) {
        if (!cancelled) {
          setStatus({
            kind: "error",
            message: e instanceof Error ? e.message : "Network error",
          });
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [token, router]);

  async function handleSignOut() {
    const supabase = createClient();
    await supabase.auth.signOut();
    if (token) {
      const next = `/auth/accept-invite?token=${encodeURIComponent(token)}`;
      router.replace(`/auth/login?next=${encodeURIComponent(next)}`);
    } else {
      router.replace("/auth/login");
    }
  }

  return (
    <div className="min-h-screen bg-[#0a0d12] flex items-center justify-center px-4">
      <div className="w-full max-w-sm">
        <div className="text-center mb-8">
          <Link href="/" className="inline-block">
            <span className="text-2xl font-bold text-green-400">ContractGate</span>
          </Link>
        </div>

        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-8">
          <h1 className="text-xl font-semibold text-slate-100 mb-4">Accept invite</h1>

          {status.kind === "loading" && (
            <p className="text-sm text-slate-400">Verifying your invite…</p>
          )}

          {status.kind === "no-token" && (
            <p className="text-sm text-red-400">
              This link is missing its invite token. Ask the person who invited you for a new link.
            </p>
          )}

          {status.kind === "success" && (
            <div>
              <p className="text-sm text-green-400 mb-2">✓ You&apos;re in.</p>
              <p className="text-xs text-slate-500">Redirecting to your dashboard…</p>
            </div>
          )}

          {status.kind === "error" && (
            <div className="space-y-4">
              <p className="text-sm text-red-400">{status.message}</p>

              {status.inviteEmail && (
                <div className="text-xs text-slate-400 bg-[#0a0d12] border border-[#1f2937] rounded-lg p-3">
                  This invite is for <strong className="text-slate-200">{status.inviteEmail}</strong>.
                  You can sign out and sign back in with that address.
                </div>
              )}

              <div className="flex gap-2">
                {status.inviteEmail ? (
                  <button
                    type="button"
                    onClick={handleSignOut}
                    className="flex-1 bg-green-600 hover:bg-green-500 text-white rounded-lg px-4 py-2.5 text-sm font-medium transition-colors"
                  >
                    Sign out and switch
                  </button>
                ) : (
                  <Link
                    href="/"
                    className="flex-1 text-center bg-[#1f2937] hover:bg-[#2d3748] text-slate-200 rounded-lg px-4 py-2.5 text-sm font-medium transition-colors"
                  >
                    Back to dashboard
                  </Link>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default function AcceptInvitePage() {
  return (
    <Suspense
      fallback={
        <div className="min-h-screen bg-[#0a0d12] flex items-center justify-center">
          <p className="text-sm text-slate-500">Loading…</p>
        </div>
      }
    >
      <AcceptInviteInner />
    </Suspense>
  );
}
