import "server-only";
import Stripe from "stripe";

// Stripe client with the SECRET key. SERVER ONLY.
let _stripe: Stripe | null = null;

export function stripe(): Stripe {
  if (_stripe) return _stripe;
  const key = process.env.STRIPE_SECRET_KEY;
  if (!key) throw new Error("STRIPE_SECRET_KEY must be set");
  _stripe = new Stripe(key);
  return _stripe;
}

/** Live subscription status for a subscription id, or null if unknown/error. */
export async function subscriptionStatus(
  subId: string | null | undefined
): Promise<string | null> {
  if (!subId) return null;
  try {
    const sub = await stripe().subscriptions.retrieve(subId);
    return sub.status;
  } catch {
    return null;
  }
}
