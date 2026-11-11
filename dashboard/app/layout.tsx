import type { Metadata } from "next";
import { cookies } from "next/headers";
import Script from "next/script";
import "./globals.css";
import Sidebar from "@/components/Sidebar";
import ClientOrgProvider from "@/components/ClientOrgProvider";
import { DEMO_MODE } from "@/lib/demo";
import DemoBanner from "@/components/DemoBanner";
import HelpProvider from "@/components/help/HelpProvider";
import BrandProvider from "@/components/BrandProvider";
import LoggedOutHeader from "@/components/LoggedOutHeader";
import { VisitorSessionProvider } from "@/components/VisitorSession";
import { BRAND_COOKIE, brandFromCookie, type Brand } from "@/lib/brand";
import { resolveVisitor } from "@/lib/visitor-session";

const SITE_URL =
  // Absolute URL is required for og:image — a relative path renders as a blank
  // card on X/Slack/LinkedIn. Fallback is the real production origin, never
  // localhost: this value gets baked into shared links.
  process.env.NEXT_PUBLIC_APP_URL ?? "https://app.datacontractgate.com";

const TITLE = "DataContractGate — Semantic Contract Enforcement";
const DESCRIPTION =
  "Trust but verify at every boundary. Semantic data contracts at ingest, between stages, and on egress — enums, patterns, ranges, and required fields validated per event, with quarantine and replay.";

// Shown to visitors who arrived from aischemagate.com and to AISchemaGate
// accounts whose cookie is set (see lib/brand.ts). Same app, family branding.
const AISG_TITLE = "AISchemaGate — by DataContractGate";
const AISG_DESCRIPTION =
  "Validate the JSON your LLMs return against a semantic contract before it reaches your application logic or database.";

async function requestBrand(): Promise<Brand> {
  return brandFromCookie((await cookies()).get(BRAND_COOKIE)?.value);
}

export async function generateMetadata(): Promise<Metadata> {
  if ((await requestBrand()) === "aisg") {
    return {
      ...metadata,
      title: { default: AISG_TITLE, template: "%s — AISchemaGate" },
      description: AISG_DESCRIPTION,
      openGraph: {
        ...metadata.openGraph,
        title: AISG_TITLE,
        description: AISG_DESCRIPTION,
      },
      twitter: {
        ...metadata.twitter,
        title: AISG_TITLE,
        description: AISG_DESCRIPTION,
      },
    };
  }
  return metadata;
}

const metadata: Metadata = {
  // Resolves the opengraph-image.jpg / twitter-image.jpg file conventions in
  // this directory into absolute URLs.
  metadataBase: new URL(SITE_URL),
  title: {
    default: TITLE,
    template: "%s — DataContractGate",
  },
  description: DESCRIPTION,
  openGraph: {
    title: TITLE,
    description: DESCRIPTION,
    url: SITE_URL,
    siteName: "DataContractGate",
    type: "website",
    locale: "en_US",
  },
  twitter: {
    card: "summary_large_image",
    title: TITLE,
    description: DESCRIPTION,
  },
  // Machine-discoverable markdown surfaces for AI coding agents. Agents that
  // scrape a page for `<link rel="alternate" type="text/markdown">` find the
  // LLM index and full documentation bundle without any heuristics.
  alternates: {
    types: {
      "text/markdown": [
        { url: "/llms.txt", title: "LLM Index" },
        { url: "/llms-full.txt", title: "Full LLM Documentation" },
        { url: "/mcp-reference.md", title: "MCP Server" },
      ],
    },
  },
};

export default async function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const brand = await requestBrand();
  const visitor = await resolveVisitor();
  const loggedOut = !DEMO_MODE && !visitor.authed;
  const mainClass = DEMO_MODE
    ? "px-4 pb-4 pt-[calc(36px+4rem)] md:px-8 md:pb-8 md:pt-[calc(36px+2rem)]"
    : loggedOut
      ? "px-4 pb-4 pt-[calc(3.5rem+3rem)] md:px-8 md:pb-8 md:pt-[calc(3rem+2rem)]"
      : "px-4 pb-4 pt-16 md:p-8";
  return (
    <html lang="en" className="dark">
      <body className="min-h-screen bg-[#0a0d12] text-slate-200 flex">
        <BrandProvider initial={brand}>
        <VisitorSessionProvider authed={visitor.authed}>
        <HelpProvider>
        {/* Demo mode: fixed 36px banner at top; main shifts down with pt-9. */}
        {DEMO_MODE && <DemoBanner />}
        <Sidebar />
        <ClientOrgProvider />
        {loggedOut && (
          <LoggedOutHeader
            pathname={visitor.pathname}
            product={brand === "aisg" ? "aischemagate" : null}
          />
        )}
        <main className={`flex-1 min-w-0 ml-0 md:ml-64 min-h-screen ${mainClass}`}>
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
        </HelpProvider>
        </VisitorSessionProvider>
        </BrandProvider>
      </body>
    </html>
  );
}
