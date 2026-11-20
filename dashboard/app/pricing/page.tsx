import PricingClient from "./PricingClient";
import { impactCheckoutAvailability } from "@/lib/stripe-prices";

// Reads server-only Stripe price configuration per request, so the
// Pro + Impact card switches from "Talk to us" to checkout as soon as
// STRIPE_PRICE_IMPACT_* is set on a deployment (no price ids reach the client).
export const dynamic = "force-dynamic";

export default function PricingPage() {
  return <PricingClient impactCheckout={impactCheckoutAvailability()} />;
}
