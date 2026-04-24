"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { createClient } from "@/lib/supabase/client";

export default function SignupPage() {
  const router = useRouter();

  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [emailSent, setEmailSent] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError("");

    if (password.length < 8) {
      setError("Password must be at least 8 characters.");
      return;
    }

    setLoading(true);
    const supabase = createClient();

    const { error: authError } = await supabase.auth.signUp({
      email,
      password,
      options: {
        data: { display_name: displayName || email.split("@")[0] },
        emailRedirectTo: `${location.origin}/auth/callback`,
      },
    });

    if (authError) {
      setError(authError.message);
      setLoading(false);
    } else {
      setEmailSent(true);
    }
  }

  if (emailSent) {
    return (
      <div className="min-h-screen bg-[#0a0d12] flex items-center justify-center px-4">
        <div className="w-full max-w-sm text-center">
          <div className="text-4xl mb-4">📬</div>
          <h1 className="text-xl font-semibold text-slate-100 mb-2">Check your email</h1>
          <p className="text-slate-400 text-sm mb-6">
            We sent a confirmation link to{" "}
            <span className="text-slate-200">{email}</span>. Click it to activate your account.
          </p>
          <Link href="/auth/login" className="text-green-400 hover:text-green-300 text-sm">
            Back to sign in
          </Link>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-[#0a0d12] flex items-center justify-center px-4">
      <div className="w-full max-w-sm">
        {/* Logo */}
        <div className="text-center mb-8">
          <Link href="/" className="inline-block">
            <span className="text-2xl font-bold text-green-400">ContractGate</span>
          </Link>
          <p className="mt-1 text-xs text-slate-500">Semantic contract enforcement at ingestion</p>
        </div>

        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-8">
          <h1 className="text-xl font-semibold text-slate-100 mb-2">Create your account</h1>
          <p className="text-sm text-slate-500 mb-6">
            Free to start. No credit card required.
          </p>

          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label className="block text-sm text-slate-400 mb-1.5" htmlFor="name">
                Your name <span className="text-slate-600">(optional)</span>
              </label>
              <input
                id="name"
                type="text"
                autoComplete="name"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                className="w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2.5 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-green-600 focus:ring-1 focus:ring-green-600/50 transition-colors"
                placeholder="Alex Suarez"
              />
            </div>

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

            <div>
              <label className="block text-sm text-slate-400 mb-1.5" htmlFor="password">
                Password
              </label>
              <input
                id="password"
                type="password"
                required
                autoComplete="new-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2.5 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-green-600 focus:ring-1 focus:ring-green-600/50 transition-colors"
                placeholder="At least 8 characters"
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
              className="w-full bg-green-600 hover:bg-green-500 disabled:opacity-50 disabled:cursor-not-allowed text-white rounded-lg px-4 py-2.5 text-sm font-medium transition-colors mt-2"
            >
              {loading ? "Creating account…" : "Create account"}
            </button>
          </form>

          <p className="mt-4 text-center text-xs text-slate-600">
            By signing up you agree to our{" "}
            <a href="https://datacontractgate.com/terms" className="text-slate-500 hover:text-slate-400">
              Terms of Service
            </a>
            .
          </p>

          <p className="mt-4 text-center text-sm text-slate-500">
            Already have an account?{" "}
            <Link href="/auth/login" className="text-green-400 hover:text-green-300">
              Sign in
            </Link>
          </p>
        </div>
      </div>
    </div>
  );
}
