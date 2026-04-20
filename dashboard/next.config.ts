import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // NOTE: output: "export" was removed to enable API route handlers (e.g.
  // /api/early-access). Static export disables all server-side routes.
  // Vercel will deploy "use client" pages as static and API routes as
  // serverless functions automatically.

  // trailingSlash causes Next.js to emit /contracts/index.html instead
  // of /contracts.html, which Vercel's static hosting resolves correctly
  // when you navigate to /contracts.
  trailingSlash: true,

  // Keep images unoptimized to avoid requiring a server image optimizer.
  images: { unoptimized: true },
};

export default nextConfig;
