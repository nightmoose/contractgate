// Fails when app/ gains a top-level route that proxy.ts KNOWN_TOP_LEVEL lacks
// (that route would 404-pass anonymous visitors instead of redirecting them).
import { readdirSync, readFileSync, statSync } from "node:fs";
const dirs = readdirSync("app").filter(
  (d) => statSync(`app/${d}`).isDirectory() && !d.startsWith("_") && !d.startsWith("("),
);
const src = readFileSync("proxy.ts", "utf8");
const missing = dirs.filter((d) => !src.includes(`"${d}"`));
if (missing.length) {
  console.error(`proxy.ts KNOWN_TOP_LEVEL is missing: ${missing.join(", ")}`);
  process.exit(1);
}
console.log(`known routes OK (${dirs.length} top-level dirs)`);
