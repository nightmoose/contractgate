import { NextRequest, NextResponse } from 'next/server';
import Stripe from 'stripe';
import { createClient } from '@/lib/supabase/server';

export const dynamic = 'force-dynamic';

// Lazy: constructing Stripe at module scope throws during `next build` when
// STRIPE_SECRET_KEY isn't present.
let _stripe: Stripe | null = null;
function getStripe(): Stripe {
  if (!_stripe) {
    _stripe = new Stripe(process.env.STRIPE_SECRET_KEY!, {
      apiVersion: '2026-05-27.dahlia',
    });
  }
  return _stripe;
}

export async function POST(req: NextRequest) {
  try {
    const stripe = getStripe();
    const supabase = await createClient();
    const { data: { user } } = await supabase.auth.getUser();

    if (!user) {
      return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
    }

    // Resolve the caller's primary org (same logic as useOrg)
    const { data: membership } = await supabase
      .from('org_memberships')
      .select('org_id, role, orgs(id, name, slug, plan, stripe_customer_id)')
      .eq('user_id', user.id)
      .order('joined_at', { ascending: true })
      .limit(1)
      .single();

    const org = membership?.orgs as any;
    if (!org) {
      return NextResponse.json({ error: 'No org found for user' }, { status: 400 });
    }

    if (org.plan === 'growth' || org.plan === 'enterprise') {
      return NextResponse.json({ error: 'Org is already on a paid plan' }, { status: 400 });
    }

    const { priceId, annual } = await req.json().catch(() => ({}));

    // Default to the env-configured Growth price (supports toggle from UI)
    const price = priceId ||
      (annual ? process.env.STRIPE_PRICE_GROWTH_ANNUAL : process.env.STRIPE_PRICE_GROWTH_MONTHLY);

    if (!price) {
      return NextResponse.json({ error: 'Missing Stripe price configuration' }, { status: 500 });
    }

    // Trial length for new Growth subscriptions started via in-app Checkout.
    // This is the code-controlled path (used when a logged-in user upgrades from /pricing).
    // Note: Direct Stripe Payment Links (if any still exist) have their own trial setting
    // configured in the Stripe dashboard and are not affected by this value.
    const TRIAL_PERIOD_DAYS = 30;

    const successUrl = `${process.env.NEXT_PUBLIC_APP_URL}/billing/success?session_id={CHECKOUT_SESSION_ID}`;
    const cancelUrl = `${process.env.NEXT_PUBLIC_APP_URL}/pricing`;

    const session = await stripe.checkout.sessions.create({
      mode: 'subscription',
      payment_method_types: ['card'],
      customer: org.stripe_customer_id || undefined,
      customer_email: org.stripe_customer_id ? undefined : user.email,
      line_items: [{ price, quantity: 1 }],
      subscription_data: {
        trial_period_days: TRIAL_PERIOD_DAYS,
        metadata: { orgId: org.id },
      },
      metadata: { orgId: org.id },
      success_url: successUrl,
      cancel_url: cancelUrl,
    });

    return NextResponse.json({ url: session.url });
  } catch (err: any) {
    console.error('create-checkout-session error:', err);
    return NextResponse.json({ error: err.message || 'Internal error' }, { status: 500 });
  }
}
