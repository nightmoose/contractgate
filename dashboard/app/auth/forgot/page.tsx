"use client";

import { useState } from "react";
import Link from "next/link";
import { createClient } from "@/lib/supabase/client";

export default function ForgotPasswordPage() {
  const [email, setEmail] = useState("");
  const [error, setError] = useState("");
  const [sent, setSent] = useState(false);
  const [loading, setLoading] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setLoading(true);
    const supabase = createClient();
    const { error: authError } = await supabase.auth.resetPasswordForEmail(email, {
      redirectTo: `${location.origin}/auth/reset`,
    });
    setLoading(false);
    if (authError) {
      setError(authError.message);
    } else {
      // Always show success (don't reveal whether the email exists).
      setSent(true);
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
          <h1 className="text-xl font-semibold text-slate-100 mb-2">Reset your password</h1>

          {sent ? (
            <div className="mt-4">
              <div className="bg-green-900/20 border border-green-700/40 rounded-lg px-3 py-3 text-sm text-green-400">
                If an account exists for <strong>{email}</strong>, a password-reset
                link is on its way. Check your inbox.
              </div>
              <p className="mt-6 text-center text-sm text-slate-500">
                <Link href="/auth/login" className="text-green-400 hover:text-green-300">
                  Back to sign in
                </Link>
              </p>
            </div>
          ) : (
            <>
              <p className="text-sm text-slate-500 mb-6">
                Enter your email and we&apos;ll send you a link to set a new password.
              </p>
              <form onSubmit={handleSubmit} className="space-y-4">
                <div>
                  <label className="block text-sm text-slate-400 mb-1.5" htmlFor="email">
                    Email address
                  </label>
                  <input
                    id="email"
                    type="email"
                    required
                    autoComplete="email"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    className="w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2.5 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-green-600 focus:ring-1 focus:ring-green-600/50 transition-colors"
                    placeholder="you@company.com"
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
                  {loading ? "Sending…" : "Send reset link"}
                </button>
              </form>

              <p className="mt-6 text-center text-sm text-slate-500">
                <Link href="/auth/login" className="text-green-400 hover:text-green-300">
                  Back to sign in
                </Link>
              </p>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
