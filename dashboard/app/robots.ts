import type { MetadataRoute } from "next";

const SITE_URL =
  process.env.NEXT_PUBLIC_APP_URL ?? "https://app.datacontractgate.com";

// Named AI crawlers are allowed explicitly so operators reading robots.txt can
// see intent, even though `User-agent: *` already covers them. The playbook
// and llms-full.txt are the primary discovery surface for these agents.
const AI_CRAWLERS = [
  "GPTBot",
  "ClaudeBot",
  "Claude-Web",
  "PerplexityBot",
  "Google-Extended",
  "CCBot",
  "anthropic-ai",
];

export default function robots(): MetadataRoute.Robots {
  return {
    rules: [
      { userAgent: "*", allow: "/" },
      ...AI_CRAWLERS.map((userAgent) => ({ userAgent, allow: "/" })),
    ],
    host: SITE_URL,
  };
}
