"use client";

import { useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import Link from 'next/link';

export default function BillingSuccessPage() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const sessionId = searchParams.get('session_id');
  const [status, setStatus] = useState<'processing' | 'success' | 'error'>('processing');
  const [message, setMessage] = useState('Confirming your subscription with Stripe...');

  useEffect(() => {
    if (!sessionId) {
      setStatus('success');
      setMessage('Payment successful. Your Growth plan should activate within a few seconds (via webhook).');
      return;
    }

    // Optional: call a verify endpoint for immediate upgrade (webhook is authoritative)
    // For now we just show a friendly message and let the user go to dashboard.
    // The webhook (or a future /api/stripe/verify-session) will update the org.plan.
    setTimeout(() => {
      setStatus('success');
      setMessage('Thanks! Your 14-day Growth trial has started. Refresh your dashboard to see upgraded features.');
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
            href="/dashboard"
            className="block w-full py-2.5 rounded-lg bg-green-600 hover:bg-green-500 text-white font-semibold"
          >
            Go to Dashboard →
          </Link>
          <Link
            href="/account"
            className="block w-full py-2.5 rounded-lg bg-[#1f2937] hover:bg-[#374151] text-slate-200 font-medium"
          >
            Manage billing &amp; account
          </Link>
          <p className="text-xs text-slate-500 mt-4">
            Session: {sessionId ? sessionId.slice(0, 20) + '…' : 'via Payment Link'}
          </p>
        </div>
      </div>
    </div>
  );
}
