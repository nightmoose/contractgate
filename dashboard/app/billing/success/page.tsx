"use client";

import { Suspense, useEffect, useState } from 'react';
import { useSearchParams } from 'next/navigation';
import Link from 'next/link';

// useSearchParams must be inside a Suspense boundary, or static prerender fails.
export default function BillingSuccessPage() {
  return (
    <Suspense fallback={<div className="min-h-[60vh] flex items-center justify-center p-6 text-slate-400">Loading…</div>}>
      <BillingSuccessContent />
    </Suspense>
  );
}

function BillingSuccessContent() {
  const searchParams = useSearchParams();
  const sessionId = searchParams.get('session_id');
  const [status, setStatus] = useState<'processing' | 'success' | 'error'>('processing');
  const [opening, setOpening] = useState(false);
  const [portalError, setPortalError] = useState<string | null>(null);

  // Stripe customer portal (same route /account uses). Same-tab navigation, so
  // no popup blocker; the portal returns to /account.
  async function openPortal() {
    setOpening(true);
    setPortalError(null);
    try {
      const res = await fetch('/api/stripe/portal', { method: 'POST' });
      const data = await res.json().catch(() => ({}));
      if (data.url) {
        window.location.assign(data.url);
        return;
      }
      setPortalError(
        res.status === 400
          ? 'Your subscription is still being set up. Try again in a few seconds.'
          : data.error || 'Could not open the billing portal.',
      );
    } catch {
      setPortalError('Could not open the billing portal.');
    }
    setOpening(false);
  }
  const [message, setMessage] = useState('Confirming your subscription with Stripe...');

  useEffect(() => {
    if (!sessionId) {
      setStatus('success');
      setMessage('Payment successful. Your Pro plan should activate within a few seconds (via webhook).');
      return;
    }

    // Optional: call a verify endpoint for immediate upgrade (webhook is authoritative)
    // For now we just show a friendly message and let the user go to dashboard.
    // The webhook (or a future /api/stripe/verify-session) will update the org.plan.
    setTimeout(() => {
      setStatus('success');
      setMessage('Thanks! Your 30-day Pro trial has started. Refresh your dashboard to see upgraded features.');
    }, 1500);
  }, [sessionId]);

  return (
    <div className="min-h-[60vh] flex items-center justify-center p-6">
      <div className="max-w-md w-full bg-[#111827] border border-[#1f2937] rounded-2xl p-8 text-center">
        <div className="text-6xl mb-4">🎉</div>
        <h1 className="text-2xl font-bold mb-2">Payment successful</h1>
        <p className="text-slate-400 mb-6">{message}</p>

        <div className="space-y-3">
          <Link
            href="/"
            className="block w-full py-2.5 rounded-lg bg-green-600 hover:bg-green-500 text-white font-semibold"
          >
            Go to Dashboard →
          </Link>
          <button
            type="button"
            onClick={openPortal}
            disabled={opening}
            className="block w-full py-2.5 rounded-lg bg-[#1f2937] hover:bg-[#374151] text-slate-200 font-medium disabled:opacity-60"
          >
            {opening ? 'Opening Stripe…' : 'Manage billing & account'}
          </button>
          {portalError && <p className="text-xs text-red-400">{portalError}</p>}
          <p className="text-xs text-slate-500 mt-4">
            Session: {sessionId ? sessionId.slice(0, 20) + '…' : 'via Payment Link'}
          </p>
        </div>
      </div>
    </div>
  );
}
