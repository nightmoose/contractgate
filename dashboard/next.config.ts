import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Standard Next.js deployment on Vercel — no static export.
  // All pages are client-side ("use client" + SWR) so no SSR needed,
  // but keeping the Node.js runtime lets Vercel handle routing
  // for /contracts, /audit, /playground, etc. correctly.
};

export default nextConfig;
