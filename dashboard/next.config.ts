import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Static export — all pages are client-side ("use client" + SWR),
  // so a fully static build works and deploys to Vercel without needing
  // a Node.js server. The API client in lib/api.ts uses NEXT_PUBLIC_API_URL
  // to reach the Rust backend directly from the browser.
  output: "export",

  // Disable image optimisation (not supported with static export)
  images: { unoptimized: true },
};

export default nextConfig;
