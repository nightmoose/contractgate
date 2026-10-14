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

// `next` comes from the email link. Only same-app paths are allowed, so a
// crafted link cannot bounce the new session to another site.
export function safeNextPath(raw: string | null | undefined, fallback: string): string {
  if (!raw) return fallback;
  const value = raw.trim();
  if (!value.startsWith("/") || value.startsWith("//")) return fallback;
  if (value.includes("\\") || value.includes("\0") || /\s/.test(value)) return fallback;
  let decoded: string;
  try {
    decoded = decodeURIComponent(value);
  } catch {
    return fallback;
  }
  if (!decoded.startsWith("/") || decoded.startsWith("//")) return fallback;
  if (decoded.includes("\\") || decoded.includes("://")) return fallback;
  return value;
}
