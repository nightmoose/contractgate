import { NextRequest, NextResponse } from 'next/server';
import Stripe from 'stripe';
import { createClient as createServiceClient } from '@supabase/supabase-js';

// This route must run per-request; never statically collected at build time.
export const dynamic = 'force-dynamic';

// Lazy singletons: constructing these at module scope throws during `next build`
// page-data collection, when the env vars aren't present.
let _stripe: Stripe | null = null;
function getStripe(): Stripe {
  if (!_stripe) {
    _stripe = new Stripe(process.env.STRIPE_SECRET_KEY!, {
      apiVersion: '2026-05-27.dahlia',
    });
  }
  return _stripe;
}

function makeSupabaseAdmin() {
  return createServiceClient(
    process.env.NEXT_PUBLIC_SUPABASE_URL!,
    process.env.SUPABASE_SERVICE_ROLE_KEY!,
    { auth: { persistSession: false } }
  );
}
let _supabaseAdmin: ReturnType<typeof makeSupabaseAdmin> | null = null;
function getSupabaseAdmin() {
  if (!_supabaseAdmin) _supabaseAdmin = makeSupabaseAdmin();
  return _supabaseAdmin;
}

// supabase-js admin exposes no email lookup, so page through listUsers.
// Capped at 10 pages (1000 users) to bound webhook latency; metadata.orgId
// is the primary path, so this fallback runs only for Payment Link buyers.
async function findUserIdByEmail(email: string): Promise<string | null> {
  const supabaseAdmin = getSupabaseAdmin();
  for (let page = 1; page <= 10; page++) {
    const { data, error } = await supabaseAdmin.auth.admin.listUsers({ page, perPage: 100 });
    if (error || !data?.users?.length) break;
    const match = data.users.find((u) => u.email?.toLowerCase() === email);
    if (match) return match.id;
    if (data.users.length < 100) break; // last page
  }
  return null;
}

// Idempotency: Stripe redelivers events on retry. We record an event id only
// AFTER its handler completes successfully, so an event that failed to do useful
// work (e.g. could not resolve an org) is NOT marked done and remains replayable.
// Returns true if this event was already fully processed and should be skipped.
async function alreadyProcessed(eventId: string): Promise<boolean> {
  const supabaseAdmin = getSupabaseAdmin();
  const { data } = await supabaseAdmin
    .from('stripe_processed_events')
    .select('event_id')
    .eq('event_id', eventId)
    .maybeSingle();
  return !!data;
}

async function markProcessed(event: Stripe.Event): Promise<void> {
  const supabaseAdmin = getSupabaseAdmin();
  // upsert: a successful re-handle of an unmarked event must not 23505-fail.
  const { error } = await supabaseAdmin
    .from('stripe_processed_events')
    .upsert({ event_id: event.id, type: event.type }, { onConflict: 'event_id' });
  if (error) console.error('[Stripe] markProcessed failed:', error);
}

export async function POST(req: NextRequest) {
  const sig = req.headers.get('stripe-signature');
  if (!sig) {
    return NextResponse.json({ error: 'Missing signature' }, { status: 400 });
  }

  let event: Stripe.Event;
  try {
    const body = await req.text();
    event = getStripe().webhooks.constructEvent(body, sig, process.env.STRIPE_WEBHOOK_SECRET!);
  } catch (err: any) {
    console.error('Webhook signature verification failed:', err.message);
    return NextResponse.json({ error: `Webhook Error: ${err.message}` }, { status: 400 });
  }

  console.log(`[Stripe Webhook] Received event: ${event.type} (${event.id})`);

  if (await alreadyProcessed(event.id)) {
    console.log(`[Stripe Webhook] Duplicate event ${event.id}, skipping`);
    return NextResponse.json({ received: true, duplicate: true });
  }

  try {
    // handled = the event did real work and should be recorded as processed.
    // false means "received but no-op" (e.g. unresolved org) — leave it
    // unmarked so a later redelivery can succeed once the data exists.
    let handled = true;

    switch (event.type) {
      case 'checkout.session.completed': {
        const session = event.data.object as Stripe.Checkout.Session;
        handled = await handleCheckoutCompleted(session);
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

    if (handled) await markProcessed(event);
  } catch (err: any) {
    console.error(`[Stripe Webhook] Error handling ${event.type}:`, err);
    // Still return 200 so Stripe doesn't retry endlessly; log for manual review
    return NextResponse.json({ received: true, error: err.message }, { status: 200 });
  }

  return NextResponse.json({ received: true });
}

// Returns true if the org was upgraded (event should be marked processed),
// false if it was a no-op (unresolved org / wrong price) so the event stays replayable.
async function handleCheckoutCompleted(session: Stripe.Checkout.Session): Promise<boolean> {
  const stripe = getStripe();
  const supabaseAdmin = getSupabaseAdmin();
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
    return false;
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
    return false;
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
    return false; // DB write failed — leave unmarked so Stripe's retry can succeed.
  }
  console.log(`[Stripe] Upgraded org ${targetOrgId} to growth via price ${priceId} (session ${session.id})`);
  return true;
}

async function handleSubscriptionUpdated(subscription: Stripe.Subscription) {
  const supabaseAdmin = getSupabaseAdmin();
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
  const supabaseAdmin = getSupabaseAdmin();
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

  const supabaseAdmin = getSupabaseAdmin();
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
