/**
 * RFC-089 — copy the agent-facing docs into public/ so they are served raw at
 *   https://app.datacontractgate.com/llms.txt
 *   https://app.datacontractgate.com/llm-integration.md
 *   https://app.datacontractgate.com/mcp-reference.md
 *   https://app.datacontractgate.com/llms-full.txt
 *
 * Canonical source is docs/ at the repo root. The copies in public/ are
 * gitignored so the repo file can never drift from what is served.
 *
 * llms-full.txt leads with the anonymous playground, then the integration
 * playbook, then the reference docs linked from llms.txt. Large-context
 * agents (Claude, Gemini) can ingest it in one HTTP round-trip.
 *
 * The Docker demo image builds with context ./dashboard, so ../docs does not
 * exist there. That build is allowed to proceed without the copies; a Vercel
 * build is not (missing files there means the paste-a-URL flow 404s in prod).
 */
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const docsDir = join(here, "..", "..", "docs");
const publicDir = join(here, "..", "public");

const FILES = [
  "llms.txt",
  "llm-integration.md",
  "mcp-reference.md",
  // RFC-095: AISchemaGate docs, served here until aischemagate.dev is live.
  "llm-output-validation.md",
  "llm-sdk-reference.md",
];

// Anonymous playground first, then the deploy playbook, then the reference
// docs linked from llms.txt. Agents that ingest the bundle see the no-key
// path before the steps that need an API key.
const FULL_BUNDLE = [
  "public-playground-reference.md",
  "llm-integration.md",
  "mcp-reference.md",
  "llm-output-validation.md",
  "llm-sdk-reference.md",
  "v1-ingest-reference.md",
  "zapier.md",
  "deploy-contract-reference.md",
  "csv-inference-reference.md",
  "quarantine-replay-reference.md",
  "drop-mode-reference.md",
  "pii-masking-reference.md",
  "contract-families-reference.md",
];

const missing = [...new Set([...FILES, ...FULL_BUNDLE])].filter(
  (f) => !existsSync(join(docsDir, f)),
);

if (missing.length > 0) {
  const msg = `sync-llm-docs: missing in ${docsDir}: ${missing.join(", ")}`;
  if (process.env.VERCEL) {
    console.error(`${msg} — failing the build (RFC-089).`);
    process.exit(1);
  }
  console.warn(`${msg} — skipping (no repo root in this build context).`);
  process.exit(0);
}

mkdirSync(publicDir, { recursive: true });
for (const f of FILES) {
  copyFileSync(join(docsDir, f), join(publicDir, f));
  console.log(`sync-llm-docs: docs/${f} -> public/${f}`);
}

const bundleHeader = `# DataContractGate — Full LLM Documentation Bundle

> Try a contract with no API key. Page: https://app.datacontractgate.com/playground
> POST https://api.datacontractgate.com/public/playground/validate
> Body: yaml_content (contract YAML) and event (one object). No Authorization
> and no x-api-key. 200 returns passed, violations, and disposition
> (pass, quarantine, or drop). 422 invalid YAML. 413 body over 64 KiB.
> 429 is the per-IP limit: 1 request/second, burst 8, with Retry-After.
> Nothing is stored.
>
> Curated contracts (the dashboard catalog): GET https://api.datacontractgate.com/public-contracts
> and GET https://api.datacontractgate.com/public-contract-families.
> GET https://api.datacontractgate.com/catalog lists org publications with
> public visibility and returns [] on the hosted API.
>
> Forking a family needs an API key from https://app.datacontractgate.com/account:
> POST https://api.datacontractgate.com/public-contract-families/{id}/fork
>
> MCP registry name: io.github.nightmoose/contractgate
> npx -y @nightmoose/contractgate-mcp-server
>
> Sections below start with the anonymous playground, then the deploy playbook
> and the reference docs. Short index: https://app.datacontractgate.com/llms.txt

Source of each section: https://github.com/nightmoose/contractgate/blob/main/docs/<file>

`;

const bundleBody = FULL_BUNDLE.map((f) => {
  const body = readFileSync(join(docsDir, f), "utf8").trimEnd();
  return `\n\n---\n\n<!-- source: docs/${f} -->\n\n${body}\n`;
}).join("");

writeFileSync(join(publicDir, "llms-full.txt"), bundleHeader + bundleBody);
console.log(
  `sync-llm-docs: bundled ${FULL_BUNDLE.length} docs -> public/llms-full.txt`,
);
