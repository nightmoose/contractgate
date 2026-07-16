import { createServerClient } from "@supabase/ssr";
import { NextResponse, type NextRequest } from "next/server";
import { DEMO_MODE } from "@/lib/demo";

// Routes that don't require authentication at the middleware level.
// Pages that use <AuthGate> for client-side gating are listed here so the
// middleware doesn't hard-redirect them — they show a compelling preview to
// unauthenticated visitors instead of a bare login wall.
const PUBLIC_ROUTES = [
  "/auth/login",
  "/auth/signup",
  "/auth/callback",
  "/pricing",
  "/docs",
  "/stream-demo",
  // AuthGate pages — show feature preview to unauthenticated users
  "/",
  "/contracts",
  "/audit",
  "/playground",
  "/account",
];

function isPublic(pathname: string) {
  return PUBLIC_ROUTES.some((r) => pathname === r || pathname.startsWith(r + "/"));
}

export async function proxy(request: NextRequest) {
  // Stripe webhooks are unauthenticated server-to-server POSTs (verified by
  // signature in the route handler, not by a Supabase session). They must skip
  // the auth gate, or the middleware 307-redirects them to /auth/login and the
  // event is never processed.
  if (request.nextUrl.pathname.startsWith("/api/stripe/webhooks")) {
    return NextResponse.next({ request });
  }

  // Slack Events API and announce endpoint are unauthenticated server-to-server
  // POSTs. The events route verifies Slack's HMAC signature; the announce route
  // uses a shared secret. Both must bypass the Supabase session gate.
  if (request.nextUrl.pathname.startsWith("/api/slack/")) {
    return NextResponse.next({ request });
  }

  // Demo mode: no Supabase env vars required — skip auth entirely.
  if (DEMO_MODE) {
    return NextResponse.next({ request });
  }

  let supabaseResponse = NextResponse.next({ request });

  const supabase = createServerClient(
    process.env.NEXT_PUBLIC_SUPABASE_URL!,
    process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY!,
    {
      cookies: {
        getAll() {
          return request.cookies.getAll();
        },
        setAll(cookiesToSet) {
          cookiesToSet.forEach(({ name, value }) =>
            request.cookies.set(name, value)
          );
          supabaseResponse = NextResponse.next({ request });
          cookiesToSet.forEach(({ name, value, options }) =>
            supabaseResponse.cookies.set(name, value, options)
          );
        },
      },
    }
  );

  // Refresh session — MUST be called before any redirect logic
  const { data: { user } } = await supabase.auth.getUser();

  const { pathname } = request.nextUrl;

  // Redirect unauthenticated users to login
  if (!user && !isPublic(pathname)) {
    const url = request.nextUrl.clone();
    url.pathname = "/auth/login";
    url.searchParams.set("next", pathname);
    return NextResponse.redirect(url);
  }

  // Redirect authenticated users away from auth pages
  if (user && pathname.startsWith("/auth/") && pathname !== "/auth/callback") {
    const url = request.nextUrl.clone();
    url.pathname = "/";
    return NextResponse.redirect(url);
  }

  return supabaseResponse;
}

export const config = {
  matcher: [
    "/((?!_next/static|_next/image|favicon.ico|logo.png|logo.svg|.*\\.(?:svg|png|jpg|jpeg|gif|webp)$).*)",
  ],
};
