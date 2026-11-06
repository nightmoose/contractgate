import type { EmailOtpType } from "@supabase/supabase-js";

// Types verifyOtp accepts for an email token_hash. "email" is the alias some
// magic-link templates still send.
const EMAIL_OTP_TYPES = [
  "signup",
  "invite",
  "magiclink",
  "recovery",
  "email_change",
  "email",
] as const satisfies readonly EmailOtpType[];

export type EmailLinkType = (typeof EMAIL_OTP_TYPES)[number];

export function isEmailOtpType(value: string): value is EmailLinkType {
  return (EMAIL_OTP_TYPES as readonly string[]).includes(value);
}

// Where a failed link sends the person. Recovery stays on the reset page so
// the message there can name the cross-browser / expired case.
export function failurePath(type: string): string {
  if (type === "recovery") return "/auth/reset?error=link_invalid";
  return "/auth/login?error=confirmation_failed";
}

export function defaultNext(type: string): string {
  if (type === "recovery") return "/auth/reset";
  if (type === "email_change") return "/account";
  return "/";
}

// Only this exact value is the AISchemaGate product signal. Anything else is
// ignored so a crafted query cannot set the brand cookie.
const AISG_PRODUCT = "aischemagate";

// Confirmation email template reads this from user metadata (`{{ .Data.signup_next }}`).
// The token_hash template does not interpolate `emailRedirectTo`, so the return
// path has to travel in metadata or the link stays `next=/`.
export const SIGNUP_NEXT_METADATA_KEY = "signup_next";

// `next` comes from a query string or an email link. Only same-app paths are
// allowed, so a crafted link cannot bounce the new session to another site.
// One percent-encoding layer is decoded: the confirmation template escapes
// slashes inside the href (`%2fcatalog%2f...`).
export function safeNextPath(raw: string | null | undefined, fallback: string): string {
  if (!raw) return fallback;
  const value = raw.trim();
  if (!value || value.startsWith("//") || value.includes("://")) return fallback;
  if (value.includes("\\") || value.includes("\0") || /\s/.test(value)) return fallback;
  let decoded: string;
  try {
    decoded = decodeURIComponent(value);
  } catch {
    return fallback;
  }
  if (!decoded.startsWith("/") || decoded.startsWith("//")) return fallback;
  if (
    decoded.includes("\\") ||
    decoded.includes("\0") ||
    decoded.includes("://") ||
    /\s/.test(decoded)
  ) {
    return fallback;
  }
  return decoded;
}

// A path that stays on the site even when the URL parser treats `\` or `//`
// as a host change. `redirect("/\\evil.example")` would otherwise leave the app.
const PATH_BASE = "https://return-path.invalid";

function resolveOnBase(candidate: string): string | null {
  try {
    const dest = new URL(candidate, PATH_BASE);
    if (dest.origin !== PATH_BASE) return null;
    return dest.pathname + dest.search + dest.hash;
  } catch {
    return null;
  }
}

export function safeInternalPath(raw: string | null | undefined, fallback: string): string {
  const candidate = safeNextPath(raw, fallback);
  const resolved = resolveOnBase(candidate);
  if (resolved) return resolved;
  if (candidate !== fallback) {
    const fb = resolveOnBase(fallback);
    if (fb) return fb;
  }
  return "/";
}

export function safeProduct(raw: string | null | undefined): string {
  return raw === AISG_PRODUCT ? AISG_PRODUCT : "";
}

// Absolute URL of an auth page, keeping a safe `next` and the AISchemaGate
// product signal. `next=/` is omitted; login and confirm already default to it.
export function withAuthReturn(
  path: string,
  nextRaw: string | null | undefined,
  productRaw: string | null | undefined,
): string {
  const params = new URLSearchParams();
  const next = safeInternalPath(nextRaw, "/");
  if (next !== "/") params.set("next", next);
  const product = safeProduct(productRaw);
  if (product) params.set("product", product);
  const query = params.toString();
  return query ? `${path}?${query}` : path;
}

// Supabase `redirectTo` / `emailRedirectTo` for GitHub OAuth and the signup
// allow-list. The confirmation email itself reads `signup_next`, not this URL.
export function callbackRedirectUrl(
  origin: string,
  nextRaw: string | null | undefined,
  productRaw: string | null | undefined,
): string {
  const url = new URL("/auth/callback", origin);
  url.searchParams.set("next", safeInternalPath(nextRaw, "/"));
  const product = safeProduct(productRaw);
  if (product) url.searchParams.set("product", product);
  return url.toString();
}

export function signupUserMetadata(input: {
  displayName: string;
  email: string;
  next: string | null | undefined;
  product: string | null | undefined;
}): Record<string, string> {
  const metadata: Record<string, string> = {
    display_name: input.displayName || input.email.split("@")[0],
  };
  const next = safeInternalPath(input.next, "/");
  if (next !== "/") metadata[SIGNUP_NEXT_METADATA_KEY] = next;
  // Migration 036 reads `product` and assigns the AISchemaGate Developer plan.
  const product = safeProduct(input.product);
  if (product) metadata.product = product;
  return metadata;
}

// Absolute same-origin URL for the post-auth redirect. A `next` that the URL
// parser would send to another host falls back to `/` on `origin`.
export function sameOriginRedirect(
  origin: string,
  raw: string | null | undefined,
  fallback = "/",
): string {
  const base = new URL(origin);
  const path = safeInternalPath(raw, fallback);
  const dest = new URL(path, base);
  if (dest.origin !== base.origin) return new URL("/", base).toString();
  return dest.toString();
}
