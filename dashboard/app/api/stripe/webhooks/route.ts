import { NextRequest, NextResponse } from 'next/server';
import Stripe from 'stripe';
import { createClient as createServiceClient } from '@supabase/supabase-js';

const stripe = new Stripe(process.env.STRIPE_SECRET_KEY!, {
  apiVersion: '2026-05-27.dahlia',
});

const supabaseAdmin = createServiceClient(
  process.env.NEXT_PUBLIC_SUPABASE_URL!,
  process.env.SUPABASE_SERVICE_ROLE_KEY!,
  { auth: { persistSession: false } }
);

// supabase-js admin exposes no email lookup, so page through listUsers.
// Capped at 10 pages (1000 users) to bound webhook latency; metadata.orgId
// is the primary path, so this fallback runs only for Payment Link buyers.
async function findUserIdByEmail(email: string): Promise<string | null> {
  for (let page = 1; page <= 10; page++) {
    const { data, error } = await supabaseAdmin.auth.admin.listUsers({ page, perPage: 100 });
    if (error || !data?.users?.length) break;
    const match = data.users.find((u) => u.email?.toLowerCase() === email);
    if (match) return match.id;
    if (data.users.length < 100) break; // last page
  }
  return null;
}

// Idempotency: skip events we've already processed (Stripe redelivers on retry).
// Returns true if this event is new and should be handled.
async function claimEvent(event: Stripe.Event): Promise<boolean> {
  const { error } = await supabaseAdmin
    .from('stripe_processed_events')
    .insert({ event_id: event.id, type: event.type });
  if (error) {
    // 23505 = unique_violation → already processed
    if ((error as any).code === '23505') return false;
    // On unexpected error, fail open (process) rather than drop the event.
    console.error('[Stripe] claimEvent insert failed (processing anyway):', error);
  }
  return true;
}

export async function POST(req: NextRequest) {
  const sig = req.headers.get('stripe-signature');
  if (!sig) {
    return NextResponse.json({ error: 'Missing signature' }, { status: 400 });
  }

  let event: Stripe.Event;
  try {
    const body = await req.text();
    event = stripe.webhooks.constructEvent(body, sig, process.env.STRIPE_WEBHOOK_SECRET!);
  } catch (err: any) {
    console.error('Webhook signature verification failed:', err.message);
    return NextResponse.json({ error: `Webhook Error: ${err.message}` }, { status: 400 });
  }

  console.log(`[Stripe Webhook] Received event: ${event.type} (${event.id})`);

  if (!(await claimEvent(event))) {
    console.log(`[Stripe Webhook] Duplicate event ${event.id}, skipping`);
    return NextResponse.json({ received: true, duplicate: true });
  }

  try {
    switch (event.type) {
      case 'checkout.session.completed': {
        const session = event.data.object as Stripe.Checkout.Session;
        await handleCheckoutCompleted(session);
        break;
      }

      case 'customer.subscription.updated': {
        const subscription = event.data.object as Stripe.Subscription;
        await handleSubscriptionUpdated(subscription);
        break;
      }

      case 'customer.subscription.deleted': {
        const subscription = event.data.object as Stripe.Subscription;
        await handleSubscriptionDeleted(subscription);
        break;
      }

      case 'invoice.payment_failed': {
        const invoice = event.data.object as Stripe.Invoice;
        await handlePaymentFailed(invoice);
        break;
      }

      default:
        // Ignore other events for now
        console.log(`[Stripe Webhook] Ignoring unhandled event type: ${event.type}`);
    }
  } catch (err: any) {
    console.error(`[Stripe Webhook] Error handling ${event.type}:`, err);
    // Still return 200 so Stripe doesn't retry endlessly; log for manual review
    return NextResponse.json({ received: true, error: err.message }, { status: 200 });
  }

  return NextResponse.json({ received: true });
}

async function handleCheckoutCompleted(session: Stripe.Checkout.Session) {
  // For Payment Links + in-app checkouts we set metadata.orgId when possible.
  // Fallback: look up by customer email (Payment Links collect email).
  // orgId is set in metadata by create-checkout-session (and mirrored onto the
  // subscription via subscription_data.metadata, which surfaces here as session.metadata).
  const orgId = session.metadata?.orgId;

  let targetOrgId: string | null = orgId || null;

  if (!targetOrgId) {
    // Fallback for marketing-site Payment Link flows: match on email.
    // supabase-js admin has no getUserByEmail; look the user up via listUsers.
    const email = (session.customer_email || session.customer_details?.email)?.toLowerCase();
    if (email) {
      const userId = await findUserIdByEmail(email);
      if (userId) {
        const { data: membership } = await supabaseAdmin
          .from('org_memberships')
          .select('org_id')
          .eq('user_id', userId)
          .order('joined_at', { ascending: true })
          .limit(1)
          .single();
        targetOrgId = membership?.org_id || null;
      }
    }
  }

  if (!targetOrgId) {
    console.warn('[Stripe] Could not resolve org for checkout.session.completed', session.id);
    return;
  }

  // Validate this checkout used one of our Growth prices (extra safety).
  // line_items isn't included on the webhook payload, so always expand it.
  const expanded = await stripe.checkout.sessions.retrieve(session.id, {
    expand: ['line_items.data.price'],
  });
  const priceId = expanded.line_items?.data?.[0]?.price?.id as string | undefined;

  const expectedMonthly = process.env.STRIPE_PRICE_GROWTH_MONTHLY;
  const expectedAnnual = process.env.STRIPE_PRICE_GROWTH_ANNUAL;

  if (priceId && expectedMonthly && expectedAnnual && priceId !== expectedMonthly && priceId !== expectedAnnual) {
    console.warn(`[Stripe] Checkout session ${session.id} used unexpected price ${priceId}. Not upgrading org ${targetOrgId}.`);
    return;
  }

  const customerId = typeof session.customer === 'string' ? session.customer : session.customer?.id;
  const subscriptionId = typeof session.subscription === 'string' ? session.subscription : session.subscription?.id;

  // Use the real subscription status (trialing/active/...) rather than inferring
  // from the checkout session. Falls back to 'trialing' since checkout sets a trial.
  let planStatus = 'trialing';
  if (subscriptionId) {
    try {
      const sub = await stripe.subscriptions.retrieve(subscriptionId);
      planStatus = sub.status;
    } catch (e) {
      console.warn(`[Stripe] Could not retrieve subscription ${subscriptionId}, defaulting plan_status=trialing`);
    }
  }

  const { error } = await supabaseAdmin
    .from('orgs')
    .update({
      plan: 'growth',
      plan_status: planStatus,
      stripe_customer_id: customerId,
      stripe_subscription_id: subscriptionId,
    })
    .eq('id', targetOrgId);

  if (error) {
    console.error('[Stripe] Failed to upgrade org on checkout.completed:', error);
  } else {
    console.log(`[Stripe] Upgraded org ${targetOrgId} to growth via price ${priceId} (session ${session.id})`);
  }
}

async function handleSubscriptionUpdated(subscription: Stripe.Subscription) {
  const { data: org } = await supabaseAdmin
    .from('orgs')
    .select('id, plan')
    .eq('stripe_subscription_id', subscription.id)
    .single();

  if (!org) return;

  const status = subscription.status; // active, trialing, past_due, canceled, etc.

  // Enterprise is managed manually (not self-serve); never auto-downgrade it to growth.
  let newPlan = org.plan;
  if (org.plan !== 'enterprise') {
    if (status === 'canceled' || status === 'unpaid') {
      newPlan = 'free';
    } else if (['active', 'trialing', 'past_due'].includes(status)) {
      newPlan = 'growth';
    }
  }

  await supabaseAdmin
    .from('orgs')
    .update({
      plan: newPlan,
      plan_status: status,
    })
    .eq('id', org.id);

  console.log(`[Stripe] Subscription ${subscription.id} updated → org ${org.id} plan=${newPlan} status=${status}`);
}

async function handleSubscriptionDeleted(subscription: Stripe.Subscription) {
  const { data: org } = await supabaseAdmin
    .from('orgs')
    .select('id')
    .eq('stripe_subscription_id', subscription.id)
    .single();

  if (!org) return;

  await supabaseAdmin
    .from('orgs')
    .update({
      plan: 'free',
      plan_status: 'canceled',
      stripe_subscription_id: null,
    })
    .eq('id', org.id);

  console.log(`[Stripe] Subscription deleted → downgraded org ${org.id} to free`);
}

async function handlePaymentFailed(invoice: Stripe.Invoice) {
  // Stripe SDK v22: the subscription moved under parent.subscription_details.
  const subRef = invoice.parent?.subscription_details?.subscription;
  if (!subRef) return;

  const subId = typeof subRef === 'string' ? subRef : subRef.id;

  const { data: org } = await supabaseAdmin
    .from('orgs')
    .select('id')
    .eq('stripe_subscription_id', subId)
    .single();

  if (!org) return;

  await supabaseAdmin
    .from('orgs')
    .update({ plan_status: 'past_due' })
    .eq('id', org.id);

  console.log(`[Stripe] Payment failed for org ${org.id} (invoice ${invoice.id})`);
}
