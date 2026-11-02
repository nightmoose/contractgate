// Drop installed packages that are only devDependencies or peerDependencies.
// npm ci --omit=dev --omit=peer still installs some devOptional peers.
// Optional dependencies of production packages (sharp under Next.js) stay.
import { existsSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";

const lock = JSON.parse(readFileSync("package-lock.json", "utf8"));
const packages = lock.packages ?? {};
const root = packages[""] ?? {};

function prodDeps(entry) {
  return [
    ...Object.keys(entry.dependencies ?? {}),
    ...Object.keys(entry.optionalDependencies ?? {}),
  ];
}

const byName = new Map();
for (const path of Object.keys(packages)) {
  if (!path.startsWith("node_modules/")) continue;
  const name = path.slice(path.lastIndexOf("node_modules/") + "node_modules/".length);
  const list = byName.get(name) ?? [];
  list.push(path);
  byName.set(name, list);
}

const keep = new Set();
const queue = prodDeps(root);
while (queue.length > 0) {
  const name = queue.pop();
  if (keep.has(name)) continue;
  keep.add(name);
  for (const path of byName.get(name) ?? []) {
    queue.push(...prodDeps(packages[path] ?? {}));
  }
}

let removed = 0;
for (const path of Object.keys(packages)) {
  if (!path.startsWith("node_modules/")) continue;
  const name = path.slice(path.lastIndexOf("node_modules/") + "node_modules/".length);
  if (keep.has(name)) continue;
  const dir = join(process.cwd(), path);
  if (existsSync(dir)) {
    rmSync(dir, { recursive: true, force: true });
    removed += 1;
  }
}

console.error(`pruned ${removed} dev or peer package directories`);
