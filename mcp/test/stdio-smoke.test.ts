import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createServer as createHttpServer, type Server } from "node:http";
import { test } from "node:test";

type Rpc = { id?: number; result?: Record<string, unknown>; error?: { message?: string } };

function listen(server: Server): Promise<number> {
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address();
      if (!addr || typeof addr === "string") {
        reject(new Error("mock gateway did not bind a TCP port"));
        return;
      }
      resolve(addr.port);
    });
  });
}

function close(server: Server): Promise<void> {
  return new Promise((resolve, reject) => {
    server.close((err) => (err ? reject(err) : resolve()));
  });
}

class StdioClient {
  private buf = "";
  private readonly pending = new Map<number, (msg: Rpc) => void>();
  readonly stderr: string[] = [];

  constructor(private readonly child: ChildProcessWithoutNullStreams) {
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      this.buf += chunk;
      let nl = this.buf.indexOf("\n");
      while (nl >= 0) {
        const line = this.buf.slice(0, nl).trim();
        this.buf = this.buf.slice(nl + 1);
        if (line) {
          const msg = JSON.parse(line) as Rpc;
          if (msg.id != null) {
            const wait = this.pending.get(msg.id);
            if (wait) {
              this.pending.delete(msg.id);
              wait(msg);
            }
          }
        }
        nl = this.buf.indexOf("\n");
      }
    });
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      this.stderr.push(chunk);
    });
  }

  request(id: number, method: string, params: Record<string, unknown>): Promise<Rpc> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`timed out waiting for ${method} (#${id})\n${this.stderr.join("")}`));
      }, 8000);
      this.pending.set(id, (msg) => {
        clearTimeout(timer);
        resolve(msg);
      });
      this.child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  }

  notify(method: string, params?: Record<string, unknown>): void {
    this.child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
  }

  stop(): void {
    this.child.kill();
  }
}

test("stdio speaks initialize, tools/list, tools/call, and prompts/get", async () => {
  const http = createHttpServer((req, res) => {
    if (req.method === "GET" && req.url === "/contracts") {
      assert.equal(req.headers["x-api-key"], "cg_live_smoke");
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify([{ id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", name: "user_events" }]));
      return;
    }
    res.writeHead(404);
    res.end("missing");
  });
  const port = await listen(http);
  const child = spawn("npx", ["tsx", "src/index.ts"], {
    cwd: new URL("..", import.meta.url),
    env: {
      ...process.env,
      CONTRACTGATE_API_KEY: "cg_live_smoke",
      CONTRACTGATE_BASE_URL: `http://127.0.0.1:${port}`,
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const client = new StdioClient(child);

  try {
    const init = await client.request(1, "initialize", {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "contractgate-mcp-smoke", version: "0.0.0" },
    });
    assert.equal(init.error, undefined);
    const serverInfo = init.result?.serverInfo as { name?: string } | undefined;
    assert.equal(serverInfo?.name, "contractgate");
    assert.equal(init.result?.protocolVersion, "2025-06-18");

    client.notify("notifications/initialized");

    const listed = await client.request(2, "tools/list", {});
    assert.equal(listed.error, undefined);
    const tools = listed.result?.tools as { name: string; inputSchema?: { type?: string } }[];
    assert.deepEqual(
      tools.map((tool) => tool.name).sort(),
      [
        "deploy_contract",
        "egress_validate",
        "get_quarantine",
        "infer_contract",
        "list_contracts",
        "validate_events",
      ],
    );
    const infer = tools.find((tool) => tool.name === "infer_contract");
    assert.equal(infer?.inputSchema?.type, "object");

    const called = await client.request(3, "tools/call", {
      name: "list_contracts",
      arguments: {},
    });
    assert.equal(called.error, undefined);
    const content = called.result?.content as { type: string; text: string }[];
    assert.equal(content[0]?.type, "text");
    assert.match(content[0]?.text ?? "", /user_events/);
    assert.equal(called.result?.isError, undefined);

    const prompt = await client.request(4, "prompts/get", { name: "integrate-contractgate" });
    assert.equal(prompt.error, undefined);
    const messages = prompt.result?.messages as { role: string; content: { type: string; text: string } }[];
    assert.equal(messages[0]?.role, "user");
    assert.equal(messages[0]?.content.type, "text");
    assert.match(messages[0]?.content.text ?? "", /https:\/\/app\.datacontractgate\.com\/llm-integration\.md/);
    assert.match(messages[0]?.content.text ?? "", /infer_contract/);
  } finally {
    client.stop();
    await close(http);
  }
});
