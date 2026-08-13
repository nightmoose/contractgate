/**
 * Build-time environment assertion.
 *
 * Why this exists: on 2026-08-07 a signup captcha shipped whose site key came
 * from NEXT_PUBLIC_TURNSTILE_SITE_KEY, with a dev fallback when unset. The
 * variable was never added to Vercel, so the production bundle silently
 * embedded a dummy key and every signup failed for six days. Nothing in dev,
 * CI, or the build log showed a problem.
 *
 * NEXT_PUBLIC_* values are inlined into client JS at build time, so a missing
 * one cannot be noticed at runtime — it is baked in. The only place to catch it
 * is here, before the bundle is produced.
 *
 * Enforced only for production builds (VERCEL_ENV=production). Preview and
 * local builds warn instead, so nobody is blocked from iterating.
 */

// Missing → the deployed app is broken in a way users cannot report usefully.
const REQUIRED_PUBLIC = [
  ["NEXT_PUBLIC_SUPABASE_URL", "auth and every database read from the browser"],
  ["NEXT_PUBLIC_SUPABASE_ANON_KEY", "auth and every database read from the browser"],
  ["NEXT_PUBLIC_API_URL", "gateway calls — falls back to http://localhost:3001"],
  ["NEXT_PUBLIC_APP_URL", "Stripe success/cancel/portal URLs — becomes 'undefined/...'"],
];

// Missing → one feature is dead, but the app still serves. Warn, don't block.
const RECOMMENDED_SERVER = [
  ["SUPABASE_SERVICE_ROLE_KEY", "API key issuance, org member management"],
  ["STRIPE_SECRET_KEY", "checkout and billing portal"],
  ["STRIPE_WEBHOOK_SECRET", "subscription state sync"],
  ["STRIPE_PRICE_GROWTH_MONTHLY", "monthly plan checkout"],
  ["STRIPE_PRICE_GROWTH_ANNUAL", "annual plan checkout"],
  ["SLACK_BOT_TOKEN", "the Slack lead-intake bot posting any reply at all"],
];

// Values that are correct locally and catastrophic in production.
const DEV_ONLY_MARKERS = ["localhost", "127.0.0.1", "0.0.0.0"];

const isProd = process.env.VERCEL_ENV === "production";
const label = isProd ? "production" : (process.env.VERCEL_ENV ?? "local");

const errors = [];
const warnings = [];

for (const [name, impact] of REQUIRED_PUBLIC) {
  const value = process.env[name];

  if (!value) {
    (isProd ? errors : warnings).push(`${name} is not set — breaks ${impact}.`);
    continue;
  }

  const marker = DEV_ONLY_MARKERS.find((m) => value.includes(m));
  if (marker && isProd) {
    // Never print the value: some of these are marked sensitive in Vercel.
    errors.push(`${name} contains "${marker}" in a production build — breaks ${impact}.`);
  }
}

if (process.env.NEXT_PUBLIC_APP_URL && !/^https?:\/\/.+/.test(process.env.NEXT_PUBLIC_APP_URL)) {
  // Stripe rejects a session whose success_url is not an absolute URL, so this
  // fails checkout outright rather than degrading.
  errors.push("NEXT_PUBLIC_APP_URL must be an absolute http(s) URL — Stripe rejects anything else.");
}

for (const [name, impact] of RECOMMENDED_SERVER) {
  if (!process.env[name]) {
    warnings.push(`${name} is not set — ${impact} will fail at runtime.`);
  }
}

for (const w of warnings) {
  console.warn(`check-required-env: WARNING (${label}): ${w}`);
}

if (errors.length > 0) {
  console.error(`\ncheck-required-env: FAILED — ${errors.length} problem(s) in this ${label} build:\n`);
  for (const e of errors) console.error(`  ✗ ${e}`);
  console.error(
    "\nSet these in Vercel → Project → Settings → Environment Variables (Production),\n" +
      "then redeploy. NEXT_PUBLIC_* is inlined at build time, so adding a variable\n" +
      "does NOT fix an already-deployed build — it needs a new one.\n",
  );
  process.exit(1);
}

console.log(`check-required-env: OK (${label}) — ${REQUIRED_PUBLIC.length} required vars present.`);
