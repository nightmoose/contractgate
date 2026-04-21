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
};

export default nextConfig;
