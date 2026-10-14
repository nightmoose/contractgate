"use client";

// Landing page after /auth/confirm has verified the recovery token_hash and
// set a session cookie. Older emails still arrive as /auth/reset?code= (PKCE).
// That exchange needs the code verifier from the browser that requested the
// reset; when it fails, say so instead of the generic "open your email" line.

import { useState, useEffect } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { createClient } from "@/lib/supabase/client";
import BrandWordmark from "@/components/BrandWordmark";

export default function ResetPasswordPage() {
  const router = useRouter();
  const [ready, setReady] = useState(false);
  const [linkError, setLinkError] = useState(false);
  const [pendingLink, setPendingLink] = useState(false);
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState("");
  const [done, setDone] = useState(false);
  const [loading, setLoading] = useState(false);

  // Confirm a recovery session is present before showing the form.
  useEffect(() => {
    const supabase = createClient();
    let settled = false;
    const markReady = () => {
      settled = true;
      setReady(true);
    };
    const { data: sub } = supabase.auth.onAuthStateChange((event) => {
      if (event === "PASSWORD_RECOVERY" || event === "SIGNED_IN") markReady();
    });
    // Also check immediately in case the event fired before we subscribed.
    supabase.auth.getSession().then(({ data }) => {
      if (data.session) markReady();
    });

    // PKCE links land here with ?code=. A different browser has no verifier,
    // so the exchange never produces a session. Fragments cover the case
    // where Supabase itself rejected the token before the redirect.
    const q = new URLSearchParams(window.location.search);
    const hash = new URLSearchParams(window.location.hash.replace(/^#/, ""));
    const code = q.get("code");
    const err = q.get("error") || hash.get("error") || hash.get("error_code");
    const alreadyRejected =
      err === "link_invalid" ||
      err === "confirmation_failed" ||
      err === "access_denied" ||
      err === "otp_expired";
    let timer: number | undefined;
    if (alreadyRejected && !code) {
      setLinkError(true);
    } else if (code || err) {
      setPendingLink(true);
      timer = window.setTimeout(() => {
        if (!settled) setLinkError(true);
      }, 2500);
    }

    return () => {
      sub.subscription.unsubscribe();
      if (timer) window.clearTimeout(timer);
    };
  }, []);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    if (password.length < 8) {
      setError("Password must be at least 8 characters.");
      return;
    }
    if (password !== confirm) {
      setError("Passwords do not match.");
      return;
    }
    setLoading(true);
    const supabase = createClient();
    const { error: authError } = await supabase.auth.updateUser({ password });
    setLoading(false);
    if (authError) {
      setError(authError.message);
    } else {
      setDone(true);
      setTimeout(() => router.push("/"), 1500);
    }
  }

  return (
    <div className="min-h-screen bg-[#0a0d12] flex items-center justify-center px-4">
      <div className="w-full max-w-sm">
        <div className="text-center mb-8">
          <Link href="/" className="inline-block">
            <BrandWordmark />
          </Link>
        </div>

        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-8">
          <h1 className="text-xl font-semibold text-slate-100 mb-6">Set a new password</h1>

          {done ? (
            <div className="bg-green-900/20 border border-green-700/40 rounded-lg px-3 py-3 text-sm text-green-400">
              Password updated. Redirecting…
            </div>
          ) : !ready ? (
            <div className="text-sm text-slate-500">
              <p>
                {linkError
                  ? "This link was opened in a different browser or has expired."
                  : pendingLink
                    ? "Checking your reset link…"
                    : "Open this page from the reset link in your email. If you got here directly, request a new link."}
              </p>
              <p className="mt-4">
                <Link href="/auth/forgot" className="text-green-400 hover:text-green-300">
                  Request a reset link
                </Link>
              </p>
            </div>
          ) : (
            <form onSubmit={handleSubmit} className="space-y-4">
              <div>
                <label className="block text-sm text-slate-400 mb-1.5" htmlFor="password">
                  New password
                </label>
                <input
                  id="password"
                  type="password"
                  required
                  autoComplete="new-password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2.5 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-green-600 focus:ring-1 focus:ring-green-600/50 transition-colors"
                  placeholder="••••••••"
                />
              </div>
              <div>
                <label className="block text-sm text-slate-400 mb-1.5" htmlFor="confirm">
                  Confirm new password
                </label>
                <input
                  id="confirm"
                  type="password"
                  required
                  autoComplete="new-password"
                  value={confirm}
                  onChange={(e) => setConfirm(e.target.value)}
                  className="w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2.5 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-green-600 focus:ring-1 focus:ring-green-600/50 transition-colors"
                  placeholder="••••••••"
                />
              </div>

              {error && (
                <div className="bg-red-900/20 border border-red-700/40 rounded-lg px-3 py-2.5 text-sm text-red-400">
                  {error}
                </div>
              )}

              <button
                type="submit"
                disabled={loading}
                className="w-full bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white rounded-lg px-4 py-2.5 text-sm font-medium transition-colors mt-2"
              >
                {loading ? "Updating…" : "Update password"}
              </button>
            </form>
          )}
        </div>
      </div>
    </div>
  );
}
