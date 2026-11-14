import { withAuthReturn } from "./auth-email-link";

// Stamped on the forwarded request by proxy.ts after the session check.
// The root layout reads them so the header is in the first HTML. They are
// not an authorization decision — proxy.ts still gates private routes.
export const VISITOR_PATH_HEADER = "x-dcg-pathname";
export const VISITOR_AUTH_HEADER = "x-dcg-auth";

// `current` is the path the visitor is on, optionally with a query string.
// Auth screens keep the `next` they were opened with, so the header does not
// send someone back to the form they are filling in. `/` is omitted by
// withAuthReturn; login and confirm already default to it.
export function loggedOutAuthHrefs(
  current: string | null | undefined,
  product: string | null | undefined,
): { signUpHref: string; signInHref: string } {
  const value = (current ?? "").trim();
  const q = value.indexOf("?");
  const path = q === -1 ? value : value.slice(0, q);
  const query = q === -1 ? "" : value.slice(q + 1);
  const onAuth = path === "/auth" || path.startsWith("/auth/");
  const nextRaw = onAuth ? new URLSearchParams(query).get("next") : value || "/";
  return {
    signUpHref: withAuthReturn("/auth/signup", nextRaw, product),
    signInHref: withAuthReturn("/auth/login", nextRaw, product),
  };
}

// Logged-out preview pages offer the playground. The playground itself is the
// live demo, so the link is omitted there instead of pointing at the current page.
export function liveDemoHref(current: string | null | undefined): string | null {
  const value = (current ?? "").trim();
  const q = value.indexOf("?");
  const path = q === -1 ? value : value.slice(0, q);
  if (path === "/playground" || path.startsWith("/playground/")) return null;
  return "/playground";
}

// proxy.ts percent-encodes the path so the header value stays ASCII.
export function decodeVisitorPath(raw: string | null | undefined): string {
  if (!raw) return "/";
  try {
    const decoded = decodeURI(raw);
    if (!decoded.startsWith("/") || decoded.startsWith("//")) return "/";
    return decoded;
  } catch {
    return "/";
  }
}
