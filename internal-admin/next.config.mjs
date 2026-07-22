/** @type {import('next').NextConfig} */
const nextConfig = {
  // This app holds the service-role + Stripe secret keys. Never expose them to
  // the client — nothing here should be referenced via NEXT_PUBLIC_*.
  reactStrictMode: true,
};

export default nextConfig;
