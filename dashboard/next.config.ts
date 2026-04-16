import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Static export — all pages are "use client" + SWR so no server
  // components or SSR needed. Deployed to Vercel as a static site.
  output: "export",

  // trailingSlash causes Next.js to emit /contracts/index.html instead
  // of /contracts.html, which Vercel's static hosting resolves correctly
  // when you navigate to /contracts.
  trailingSlash: true,

  // Image optimisation is not available in static export mode.
  images: { unoptimized: true },
};

export default nextConfig;
