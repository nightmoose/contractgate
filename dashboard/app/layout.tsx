import type { Metadata } from "next";
import Script from "next/script";
import "./globals.css";
import Sidebar from "@/components/Sidebar";
import ClientOrgProvider from "@/components/ClientOrgProvider";
import { DEMO_MODE } from "@/lib/demo";
import DemoBanner from "@/components/DemoBanner";

const SITE_URL =
  // Absolute URL is required for og:image — a relative path renders as a blank
  // card on X/Slack/LinkedIn. Fallback is the real production origin, never
  // localhost: this value gets baked into shared links.
  process.env.NEXT_PUBLIC_APP_URL ?? "https://app.datacontractgate.com";

const TITLE = "ContractGate — Semantic Contract Enforcement";
const DESCRIPTION =
  "Stop bad data before it hits your warehouse. Semantic data contracts enforced at ingestion — enums, patterns, ranges, and required fields validated per event, with quarantine and replay.";

export const metadata: Metadata = {
  // Resolves the opengraph-image.jpg / twitter-image.jpg file conventions in
  // this directory into absolute URLs.
  metadataBase: new URL(SITE_URL),
  title: TITLE,
  description: DESCRIPTION,
  openGraph: {
    title: TITLE,
    description: DESCRIPTION,
    url: SITE_URL,
    siteName: "ContractGate",
    type: "website",
    locale: "en_US",
  },
  twitter: {
    card: "summary_large_image",
    title: TITLE,
    description: DESCRIPTION,
  },
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" className="dark">
      <body className="min-h-screen bg-[#0a0d12] text-slate-200 flex">
        {/* Demo mode: fixed 36px banner at top; main shifts down with pt-9. */}
        {DEMO_MODE && <DemoBanner />}
        <Sidebar />
        <ClientOrgProvider />
        <main className={`flex-1 ml-0 md:ml-64 p-8 min-h-screen${DEMO_MODE ? " pt-[calc(2rem+36px)]" : ""}`}>
          {children}
        </main>
        {/* Vercel Web Analytics.
            This is the same script the @vercel/analytics package injects; it is
            referenced directly because that package declares an optional peer on
            @remix-run/react@^2 (which peers react@^18) and fails to install on
            React 19 — a dependency we do not need for one script tag.
            Serves 404 harmlessly until Web Analytics is enabled for the project
            in the Vercel dashboard. Skipped in demo builds, which have no
            Vercel edge to serve it. */}
        {!DEMO_MODE && (
          <Script src="/_vercel/insights/script.js" strategy="afterInteractive" defer />
        )}
      </body>
    </html>
  );
}
