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
      apiVersion: '2026-06-24.dahlia',
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

// Records a webhook event that failed to do useful work into
// stripe_failed_events so it is durable and alertable, instead of vanishing
// into console logs. The webhook still returns 200 to Stripe; this row is the
// signal that a paying customer may be stuck. Best-effort: a logging failure
// here must never mask the original problem.
type FailureReason = 'unresolved_org' | 'db_write_failed' | 'handler_error' | 'unexpected_price';
async function recordFailure(
  event: Stripe.Event,
  reason: FailureReason,
  detail: string,
  orgId?: string | null
): Promise<void> {
  console.error(`[Stripe] FAILED event ${event.id} (${event.type}) reason=${reason}: ${detail}`);
  try {
    const supabaseAdmin = getSupabaseAdmin();
    const { error } = await supabaseAdmin
      .from('stripe_failed_events')
      .upsert(
        {
          event_id: event.id,
          type: event.type,
          reason,
          detail: detail.slice(0, 2000),
          org_id: orgId ?? null,
          resolved: false,
          last_seen: new Date().toISOString(),
        },
        { onConflict: 'event_id' }
      );
    if (error) console.error('[Stripe] recordFailure persist failed:', error);
  } catch (e) {
    console.error('[Stripe] recordFailure threw:', e);
  }
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
    // false means "received but no-op" (e.g. unresolved org, or an event type we
    // don't act on) — leave it unmarked so a later redelivery can succeed once
    // the data exists. Defaults to false so an unhandled branch can never
    // silently mark itself done.
    let handled = false;

    switch (event.type) {
      case 'checkout.session.completed': {
        const session = event.data.object as Stripe.Checkout.Session;
        handled = await handleCheckoutCompleted(event, session);
        break;
      }

      case 'customer.subscription.updated': {
        const subscription = event.data.object as Stripe.Subscription;
        handled = await handleSubscriptionUpdated(subscription);
        break;
      }

      case 'customer.subscription.deleted': {
        const subscription = event.data.object as Stripe.Subscription;
        handled = await handleSubscriptionDeleted(subscription);
        break;
      }

      case 'invoice.payment_failed': {
        const invoice = event.data.object as Stripe.Invoice;
        handled = await handlePaymentFailed(invoice);
        break;
      }

      default:
        // Event type we intentionally don't act on. Not a failure — just a
        // no-op. Leave handled=false so we don't record a dedup row for an
        // event we never processed.
        console.log(`[Stripe Webhook] Ignoring unhandled event type: ${event.type}`);
    }

    if (handled) await markProcessed(event);
  } catch (err: any) {
    // A handler threw. Return 200 so Stripe doesn't retry-storm, but persist the
    // failure so it's visible/alertable rather than swallowed in logs.
    await recordFailure(event, 'handler_error', err?.message || String(err));
    return NextResponse.json({ received: true, error: err.message }, { status: 200 });
  }

  return NextResponse.json({ received: true });
}

// Returns true if the org was upgraded (event should be marked processed),
// false if it was a no-op (unresolved org / wrong price) so the event stays replayable.
async function handleCheckoutCompleted(event: Stripe.Event, session: Stripe.Checkout.Session): Promise<boolean> {
  const stripe = getStripe();
  const supabaseAdmin = getSupabaseAdmin();
  // Option A (in-app-only): org resolution is deterministic via metadata.orgId,
  // stamped by create-checkout-session on both the session and the subscription.
  // There is no public Payment Link funnel, so the old email-match fallback was
  // removed — email matching silently no-ops when the Stripe email differs from
  // the signup email, which is the kind of invisible failure we're eliminating.
  const targetOrgId: string | null = session.metadata?.orgId || null;

  if (!targetOrgId) {
    // No orgId on the session. Under Option A this should never happen for a
    // legitimate in-app checkout; record it so it's visible rather than silent.
    await recordFailure(
      event,
      'unresolved_org',
      `checkout.session.completed ${session.id} had no metadata.orgId (in-app checkout should always set it)`
    );
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
    await recordFailure(
      event,
      'unexpected_price',
      `session ${session.id} used unexpected price ${priceId}; not upgrading`,
      targetOrgId
    );
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
    // DB write failed — leave unmarked so Stripe's retry can succeed, AND
    // record it so a persistent failure (e.g. schema drift like the 2026-06-05
    // plan-constraint bug) is visible instead of silently 200-ing.
    await recordFailure(event, 'db_write_failed', error.message, targetOrgId);
    return false;
  }
  console.log(`[Stripe] Upgraded org ${targetOrgId} to growth via price ${priceId} (session ${session.id})`);
  return true;
}

// Returns true if an org was updated; false if no org matched the subscription
// (a benign no-op — e.g. a subscription we never recorded). DB-write errors
// throw and are caught by the POST handler, which records them.
async function handleSubscriptionUpdated(subscription: Stripe.Subscription): Promise<boolean> {
  const supabaseAdmin = getSupabaseAdmin();
  const { data: org } = await supabaseAdmin
    .from('orgs')
    .select('id, plan')
    .eq('stripe_subscription_id', subscription.id)
    .single();

  if (!org) return false;

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

  const { error } = await supabaseAdmin
    .from('orgs')
    .update({ plan: newPlan, plan_status: status })
    .eq('id', org.id);
  if (error) throw new Error(`subscription.updated org ${org.id}: ${error.message}`);

  console.log(`[Stripe] Subscription ${subscription.id} updated → org ${org.id} plan=${newPlan} status=${status}`);
  return true;
}

async function handleSubscriptionDeleted(subscription: Stripe.Subscription): Promise<boolean> {
  const supabaseAdmin = getSupabaseAdmin();
  const { data: org } = await supabaseAdmin
    .from('orgs')
    .select('id')
    .eq('stripe_subscription_id', subscription.id)
    .single();

  if (!org) return false;

  const { error } = await supabaseAdmin
    .from('orgs')
    .update({
      plan: 'free',
      plan_status: 'canceled',
      stripe_subscription_id: null,
    })
    .eq('id', org.id);
  if (error) throw new Error(`subscription.deleted org ${org.id}: ${error.message}`);

  console.log(`[Stripe] Subscription deleted → downgraded org ${org.id} to free`);
  return true;
}

async function handlePaymentFailed(invoice: Stripe.Invoice): Promise<boolean> {
  // Stripe SDK v22: the subscription moved under parent.subscription_details.
  const subRef = invoice.parent?.subscription_details?.subscription;
  if (!subRef) return false;

  const subId = typeof subRef === 'string' ? subRef : subRef.id;

  const supabaseAdmin = getSupabaseAdmin();
  const { data: org } = await supabaseAdmin
    .from('orgs')
    .select('id')
    .eq('stripe_subscription_id', subId)
    .single();

  if (!org) return false;

  const { error } = await supabaseAdmin
    .from('orgs')
    .update({ plan_status: 'past_due' })
    .eq('id', org.id);
  if (error) throw new Error(`invoice.payment_failed org ${org.id}: ${error.message}`);

  console.log(`[Stripe] Payment failed for org ${org.id} (invoice ${invoice.id})`);
  return true;
}
