"use client";

import { useState } from "react";
import Link from "next/link";
import { createClient } from "@/lib/supabase/client";

export default function SignupPage() {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [githubLoading, setGithubLoading] = useState(false);
  const [emailSent, setEmailSent] = useState(false);

  async function handleGitHubSignUp() {
    setError("");
    setGithubLoading(true);
    const supabase = createClient();
    const { error: authError } = await supabase.auth.signInWithOAuth({
      provider: "github",
      options: {
        redirectTo: `${location.origin}/auth/callback?next=/`,
      },
    });
    if (authError) {
      setError(authError.message);
      setGithubLoading(false);
    }
    // On success Supabase redirects — no manual push needed.
  }

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

          {/* GitHub OAuth */}
          <button
            type="button"
            onClick={handleGitHubSignUp}
            disabled={githubLoading || loading}
            className="w-full flex items-center justify-center gap-2.5 bg-[#24292e] hover:bg-[#2f363d] disabled:opacity-50 disabled:cursor-not-allowed text-white rounded-lg px-4 py-2.5 text-sm font-medium transition-colors border border-[#374151] mb-4"
          >
            <svg height="18" viewBox="0 0 16 16" width="18" fill="currentColor" aria-hidden="true">
              <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
            </svg>
            {githubLoading ? "Redirecting…" : "Sign up with GitHub"}
          </button>

          {/* Divider */}
          <div className="flex items-center gap-3 mb-4">
            <div className="flex-1 h-px bg-[#1f2937]" />
            <span className="text-xs text-slate-600">or sign up with email</span>
            <div className="flex-1 h-px bg-[#1f2937]" />
          </div>

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
              disabled={loading || githubLoading}
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
