// Fill manifest.json "tools" with the server's real tools/list (Smithery needs inputSchema).
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2];
const child = spawn("node", [join(dir, "dist/index.js")], {
  env: { ...process.env, CONTRACTGATE_API_KEY: "cg_live_bundle_scan" },
  stdio: ["pipe", "pipe", "inherit"],
});

const send = (msg) => child.stdin.write(JSON.stringify(msg) + "\n");
let buf = "";
child.stdout.on("data", (chunk) => {
  buf += chunk;
  for (const line of buf.split("\n").slice(0, -1)) {
    const msg = JSON.parse(line);
    if (msg.id === 1) {
      send({ jsonrpc: "2.0", method: "notifications/initialized" });
      send({ jsonrpc: "2.0", id: 2, method: "tools/list" });
    } else if (msg.id === 2) {
      const path = join(dir, "manifest.json");
      const manifest = JSON.parse(readFileSync(path, "utf8"));
      manifest.tools = msg.result.tools.map(({ name, description, inputSchema }) => ({
        name,
        description,
        inputSchema,
      }));
      writeFileSync(path, JSON.stringify(manifest, null, 2) + "\n");
      console.log(`injected ${manifest.tools.length} tools`);
      child.kill();
    }
  }
  buf = buf.slice(buf.lastIndexOf("\n") + 1);
});

send({
  jsonrpc: "2.0",
  id: 1,
  method: "initialize",
  params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "bundle", version: "1" } },
});
