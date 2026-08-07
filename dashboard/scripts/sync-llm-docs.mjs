/**
 * RFC-089 — copy the agent-facing docs into public/ so they are served raw at
 *   https://app.datacontractgate.com/llms.txt
 *   https://app.datacontractgate.com/llm-integration.md
 *
 * Canonical source is docs/ at the repo root. The copies in public/ are
 * gitignored so the repo file can never drift from what is served.
 *
 * The Docker demo image builds with context ./dashboard, so ../docs does not
 * exist there. That build is allowed to proceed without the copies; a Vercel
 * build is not (missing files there means the paste-a-URL flow 404s in prod).
 */
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const docsDir = join(here, "..", "..", "docs");
const publicDir = join(here, "..", "public");

const FILES = ["llms.txt", "llm-integration.md"];

const missing = FILES.filter((f) => !existsSync(join(docsDir, f)));

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
