/**
 * RFC-089 — copy the agent-facing docs into public/ so they are served raw at
 *   https://app.datacontractgate.com/llms.txt
 *   https://app.datacontractgate.com/llm-integration.md
 *   https://app.datacontractgate.com/llms-full.txt
 *
 * Canonical source is docs/ at the repo root. The copies in public/ are
 * gitignored so the repo file can never drift from what is served.
 *
 * llms-full.txt is a concatenation of the integration playbook plus every
 * reference doc linked from llms.txt. Large-context agents (Claude, Gemini)
 * can ingest it in one HTTP round-trip instead of following links.
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

const FILES = ["llms.txt", "llm-integration.md"];

// Order matches the "Reference" section of llms.txt. Playbook first so agents
// hit the executable flow before the deep reference material.
const FULL_BUNDLE = [
  "llm-integration.md",
  "v1-ingest-reference.md",
  "deploy-contract-reference.md",
  "csv-inference-reference.md",
  "quarantine-replay-reference.md",
  "pii-masking-reference.md",
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

const bundleHeader = `# ContractGate — Full LLM Documentation Bundle

> One-shot ingestion for large-context agents. Concatenates the integration
> playbook and every reference doc linked from llms.txt in the order an agent
> is likely to need them. For a lightweight index, use llms.txt instead.

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
