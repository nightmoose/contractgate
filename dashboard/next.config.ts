import type { NextConfig } from "next";
import pkg from "./package.json";

// Security headers (tester polish round). Enforced now: the low-risk
// directives (no framing, no plugins, no <base> hijack, nosniff). The full
// source allow-list ships as Report-Only first: Next.js inline bootstrap
// scripts, Turnstile, Supabase (https + wss), the gateway and the API
// workbench (which fetches arbitrary user URLs) all need to be confirmed in
// the browser console before it is enforced.
const CSP_ENFORCED = "frame-ancestors 'none'; object-src 'none'; base-uri 'self'";
const CSP_REPORT_ONLY = [
  "default-src 'self'",
  "script-src 'self' 'unsafe-inline' https://challenges.cloudflare.com",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob: https:",
  "font-src 'self' data:",
  "connect-src 'self' https: wss://*.supabase.co",
  "frame-src https://challenges.cloudflare.com https://checkout.stripe.com https://billing.stripe.com",
  "form-action 'self' https://checkout.stripe.com https://billing.stripe.com",
  "worker-src 'self' blob:",
  "frame-ancestors 'none'",
  "object-src 'none'",
  "base-uri 'self'",
].join("; ");

const SECURITY_HEADERS = [
  { key: "Content-Security-Policy", value: CSP_ENFORCED },
  { key: "Content-Security-Policy-Report-Only", value: CSP_REPORT_ONLY },
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
];

const nextConfig: NextConfig = {
  // NOTE: output: "export" was removed to enable API route handlers (e.g.
  // /api/early-access). Static export disables all server-side routes.
  // Vercel will deploy "use client" pages as static and API routes as
  // serverless functions automatically.

  // trailingSlash was removed — it was only needed for static export mode
  // (to generate /contracts/index.html). In server mode it causes redirects
  // that flip POST requests to GET, breaking API routes like /api/early-access.

  // Keep images unoptimized to avoid requiring a server image optimizer.
  images: { unoptimized: true },

  // Sidebar shows this; package.json is the single source of the dashboard version.
  env: { NEXT_PUBLIC_APP_VERSION: pkg.version },

  // RFC-089: the agent-facing docs are copied into public/ by the prebuild
  // step. Serve them as plain text so browsers render them inline instead of
  // downloading, and so an agent fetching the URL gets markdown, not HTML.
  async headers() {
    return [
      { source: "/:path*", headers: SECURITY_HEADERS },
      {
        source: "/:path(llms.txt|llms-full.txt|llm-integration.md|mcp-reference.md|llm-output-validation.md|llm-sdk-reference.md)",
        headers: [
          { key: "Content-Type", value: "text/plain; charset=utf-8" },
          { key: "Cache-Control", value: "public, max-age=300" },
          { key: "Access-Control-Allow-Origin", value: "*" },
        ],
      },
    ];
  },

  // Standalone output bundles server + node_modules for the Docker image
  // (demo and non-demo). Vercel sets VERCEL and uses its own output.
  ...(process.env.VERCEL ? {} : { output: "standalone" as const }),
};

export default nextConfig;
