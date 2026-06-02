import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Proxy API requests to the Rust backend in development
  async rewrites() {
    return [
      {
        source: "/api/backend/:path*",
        destination: `${process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:3001"}/:path*`,
      },
    ];
  },
};

export default nextConfig;
