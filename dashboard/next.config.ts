import type { NextConfig } from "next";

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

  // RFC-089: the agent-facing docs are copied into public/ by the prebuild
  // step. Serve them as plain text so browsers render them inline instead of
  // downloading, and so an agent fetching the URL gets markdown, not HTML.
  async headers() {
    return [
      {
        source: "/:path(llms.txt|llms-full.txt|llm-integration.md)",
        headers: [
          { key: "Content-Type", value: "text/plain; charset=utf-8" },
          { key: "Cache-Control", value: "public, max-age=300" },
          { key: "Access-Control-Allow-Origin", value: "*" },
        ],
      },
    ];
  },

  // Standalone output bundles server + node_modules for minimal Docker images.
  // Only active when NEXT_PUBLIC_DEMO_MODE is set (build arg from Dockerfile);
  // Vercel deploys ignore this because it detects the Vercel environment.
  ...(process.env.NEXT_PUBLIC_DEMO_MODE === "1" ? { output: "standalone" } : {}),
};

export default nextConfig;
