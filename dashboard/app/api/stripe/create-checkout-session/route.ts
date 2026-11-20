import { NextRequest, NextResponse } from 'next/server';
import Stripe from 'stripe';
import { createClient } from '@/lib/supabase/server';
import { impactPriceAnnual, impactPriceMonthly, proPriceAnnual, proPriceMonthly } from '@/lib/stripe-prices';
import { checkoutPrice, isAisgPlan, isPaidStoredPlan } from '@/lib/billing-plan';

export const dynamic = 'force-dynamic';

// Lazy: constructing Stripe at module scope throws during `next build` when
// STRIPE_SECRET_KEY isn't present.
let _stripe: Stripe | null = null;
function getStripe(): Stripe {
  if (!_stripe) {
    _stripe = new Stripe(process.env.STRIPE_SECRET_KEY!, {
      apiVersion: '2026-06-24.dahlia',
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

    if (isPaidStoredPlan(org.plan)) {
      return NextResponse.json({ error: 'Org is already on a paid plan' }, { status: 400 });
    }

    const { priceId, annual, plan } = await req.json().catch(() => ({}));

    // Pro + Impact (stored plan 'impact') is a DataContractGate plan with its
    // own Stripe prices. The client never picks its price id.
    if (plan === 'impact') {
      if (isAisgPlan(org.plan)) {
        return NextResponse.json(
          { error: 'Pro + Impact is a DataContractGate plan. Contact hello@datacontractgate.com.' },
          { status: 400 }
        );
      }
      const impactPrice = checkoutPrice('impact', !!annual, {
        impactMonthly: impactPriceMonthly(),
        impactAnnual: impactPriceAnnual(),
      });
      if (!impactPrice) {
        // Not an error: the price is created in Stripe separately. The pricing
        // card already shows "Talk to us" in this state.
        return NextResponse.json(
          {
            error: 'Self-serve checkout for Pro + Impact is not open yet. Email hello@datacontractgate.com.',
            code: 'impact_price_unconfigured',
          },
          { status: 503 }
        );
      }
      return await startCheckout({ stripe, org, user, price: impactPrice, product: 'datacontractgate', plan: 'impact' });
    }

    // One Pro plan and one set of Stripe prices for both products (decided
    // 2026-10-12). Which product the buyer is on travels as checkout
    // metadata; the webhook uses it to set 'aisg_pro' vs 'growth'.
    const aisgPro = plan === 'aisg_pro' || org.plan === 'aisg_free';
    const product = aisgPro ? 'aischemagate' : 'datacontractgate';
    // A caller-supplied price id may not buy Pro + Impact through the Pro path.
    if (priceId && [impactPriceMonthly(), impactPriceAnnual()].includes(priceId)) {
      return NextResponse.json({ error: 'Use plan "impact" for Pro + Impact checkout' }, { status: 400 });
    }
    const price = priceId ||
      checkoutPrice('pro', !!annual, { proMonthly: proPriceMonthly(), proAnnual: proPriceAnnual() });

    if (!price) {
      return NextResponse.json({ error: 'Missing Stripe price configuration' }, { status: 500 });
    }

    return await startCheckout({ stripe, org, user, price, product, plan: 'pro' });
  } catch (err: any) {
    console.error('create-checkout-session error:', err);
    return NextResponse.json({ error: err.message || 'Internal error' }, { status: 500 });
  }
}

async function startCheckout({
  stripe,
  org,
  user,
  price,
  product,
  plan,
}: {
  stripe: Stripe;
  org: { id: string; stripe_customer_id?: string | null };
  user: { email?: string | null };
  price: string;
  product: 'datacontractgate' | 'aischemagate';
  plan: 'pro' | 'impact';
}) {
  // Trial length for new Growth subscriptions started via in-app Checkout.
  // This is the code-controlled path (used when a logged-in user upgrades from /pricing).
  // Note: Direct Stripe Payment Links (if any still exist) have their own trial setting
  // configured in the Stripe dashboard and are not affected by this value.
  const TRIAL_PERIOD_DAYS = 30;

  const successUrl = `${process.env.NEXT_PUBLIC_APP_URL}/billing/success?session_id={CHECKOUT_SESSION_ID}`;
  const cancelUrl = `${process.env.NEXT_PUBLIC_APP_URL}/pricing${
    plan === 'impact' ? '?plan=impact' : product === 'aischemagate' ? '?product=aischemagate' : ''
  }`;

  const session = await stripe.checkout.sessions.create({
    mode: 'subscription',
    payment_method_types: ['card'],
    customer: org.stripe_customer_id || undefined,
    customer_email: org.stripe_customer_id ? undefined : user.email ?? undefined,
    line_items: [{ price, quantity: 1 }],
    subscription_data: {
      trial_period_days: TRIAL_PERIOD_DAYS,
      metadata: { orgId: org.id, product, plan },
    },
    metadata: { orgId: org.id, product, plan },
    success_url: successUrl,
    cancel_url: cancelUrl,
  });

  return NextResponse.json({ url: session.url });
}
