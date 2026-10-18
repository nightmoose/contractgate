import { NextResponse } from 'next/server';
import Stripe from 'stripe';
import { createClient } from '@/lib/supabase/server';

export const dynamic = 'force-dynamic';

// Live subscription details for /account (trial end, pending cancellation).
// Read from Stripe on demand so no new columns/migration are needed.
let _stripe: Stripe | null = null;
function getStripe(): Stripe {
  if (!_stripe) {
    _stripe = new Stripe(process.env.STRIPE_SECRET_KEY!, { apiVersion: '2026-06-24.dahlia' });
  }
  return _stripe;
}

export async function GET() {
  try {
    const supabase = await createClient();
    const { data: { user } } = await supabase.auth.getUser();
    if (!user) return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });

    const { data: membership } = await supabase
      .from('org_memberships')
      .select('orgs(stripe_subscription_id)')
      .eq('user_id', user.id)
      .order('joined_at', { ascending: true })
      .limit(1)
      .single();
    const orgs = membership?.orgs as { stripe_subscription_id?: string | null } | { stripe_subscription_id?: string | null }[] | null | undefined;
    const subId = Array.isArray(orgs) ? orgs[0]?.stripe_subscription_id : orgs?.stripe_subscription_id;
    if (!subId) return NextResponse.json({ subscription: null });

    const sub = await getStripe().subscriptions.retrieve(subId);
    const periodEnd = sub.items?.data?.[0]?.current_period_end ?? null;
    return NextResponse.json({
      subscription: {
        status: sub.status,
        trial_end: sub.trial_end ?? null,
        cancel_at_period_end: sub.cancel_at_period_end,
        cancel_at: sub.cancel_at ?? null,
        current_period_end: periodEnd,
      },
    });
  } catch (err: unknown) {
    console.error('subscription lookup error:', err);
    return NextResponse.json({ error: 'Could not load subscription' }, { status: 500 });
  }
}
