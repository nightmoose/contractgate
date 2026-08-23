import assert from "node:assert/strict";
import { test } from "node:test";
import { ConfigError, Gateway, GatewayError } from "../src/gateway.ts";
import { createServer } from "../src/server.ts";
import {
  deployContract,
  inferContract,
  listContracts,
  getQuarantine,
  validateEvents,
  xorContractTarget,
} from "../src/tools.ts";

type Handler = (req: Request) => Response | Promise<Response>;

function mockFetch(routes: Record<string, Handler>): typeof fetch {
  return async (input, init) => {
    const req = new Request(input, init);
    const url = new URL(req.url);
    const key = `${req.method} ${url.pathname}`;
    const handler = routes[key];
    if (!handler) {
      return new Response(`unexpected ${key}`, { status: 599 });
    }
    return handler(req);
  };
}

function gw(routes: Record<string, Handler>): Gateway {
  return new Gateway({
    baseUrl: "https://gw.test",
    apiKey: "cg_live_test",
    fetch: mockFetch(routes),
  });
}

test("fromEnv requires CONTRACTGATE_API_KEY", () => {
  assert.throws(
    () => Gateway.fromEnv({}),
    (err: unknown) => err instanceof ConfigError,
  );
});

test("fromEnv reads key and optional base URL", () => {
  const g = Gateway.fromEnv({
    CONTRACTGATE_API_KEY: " cg_live_abc ",
    CONTRACTGATE_BASE_URL: "https://custom.example/",
  });
  assert.equal(g.apiKey, "cg_live_abc");
  assert.equal(g.baseUrl, "https://custom.example");
});

test("infer_contract posts samples and returns yaml", async () => {
  const g = gw({
    "POST /contracts/infer": async (req) => {
      assert.equal(req.headers.get("x-api-key"), "cg_live_test");
      const body = await req.json();
      assert.equal(body.name, "user_events");
      assert.equal(body.samples.length, 1);
      return Response.json({ yaml_content: "name: user_events\n", field_count: 1, sample_count: 1 });
    },
  });
  const out = (await inferContract(g, {
    name: "user_events",
    samples: [{ user_id: "u_1" }],
  })) as { yaml_content: string };
  assert.match(out.yaml_content, /user_events/);
});

test("validate_events ingest defaults to dry_run", async () => {
  const g = gw({
    "POST /v1/ingest/aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa": async (req) => {
      const url = new URL(req.url);
      assert.equal(url.searchParams.get("dry_run"), "true");
      return new Response(
        JSON.stringify({ total: 1, passed: 1, failed: 0, dry_run: true, results: [] }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    },
  });
  const out = (await validateEvents(g, {
    contract_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
    events: [{ user_id: "u_1" }],
  })) as { dry_run: boolean };
  assert.equal(out.dry_run, true);
});

test("validate_events treats 422 as data, not transport error", async () => {
  const g = gw({
    "POST /v1/ingest/aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa": async () =>
      new Response(
        JSON.stringify({
          total: 1,
          passed: 0,
          failed: 1,
          results: [
            {
              index: 0,
              passed: false,
              violations: [
                {
                  field: "timestamp",
                  kind: "type_mismatch",
                  suggestion: "emit integer epoch",
                },
              ],
            },
          ],
        }),
        { status: 422, headers: { "Content-Type": "application/json" } },
      ),
  });
  const out = (await validateEvents(g, {
    contract_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
    events: [{ timestamp: "now" }],
    dry_run: true,
  })) as { failed: number; results: { violations: { suggestion: string }[] }[] };
  assert.equal(out.failed, 1);
  assert.equal(out.results[0].violations[0].suggestion, "emit integer epoch");
});

test("validate_events yaml path uses playground per event", async () => {
  let n = 0;
  const g = gw({
    "POST /playground/validate": async (req) => {
      n += 1;
      const body = await req.json();
      assert.ok(typeof body.yaml_content === "string");
      return Response.json({ passed: true, violations: [], validation_us: 1 });
    },
  });
  const out = (await validateEvents(g, {
    yaml_content: "name: t\nontology:\n  entities: []\n",
    events: [{ a: 1 }, { a: 2 }],
  })) as { mode: string; persisted: boolean; results: unknown[] };
  assert.equal(out.mode, "playground");
  assert.equal(out.persisted, false);
  assert.equal(out.results.length, 2);
  assert.equal(n, 2);
});

test("validate_events rejects both or neither target", () => {
  assert.throws(() => xorContractTarget({}));
  assert.throws(() =>
    xorContractTarget({
      contract_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
      yaml_content: "x",
    }),
  );
  xorContractTarget({ contract_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa" });
  xorContractTarget({ yaml_content: "x" });
});

test("deploy_contract posts yaml and defaults deployed_by", async () => {
  const g = gw({
    "POST /contracts/deploy": async (req) => {
      const body = await req.json();
      assert.equal(body.name, "user_events");
      assert.equal(body.deployed_by, "mcp");
      assert.ok(body.yaml_content.includes("user_events"));
      return Response.json({
        contract_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        version: "1.0",
      });
    },
  });
  const out = (await deployContract(g, {
    name: "user_events",
    yaml_content: "name: user_events\nversion: \"1.0\"\n",
  })) as { contract_id: string };
  assert.equal(out.contract_id, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
});

test("get_quarantine forwards query params", async () => {
  const g = gw({
    "GET /quarantine": async (req) => {
      const url = new URL(req.url);
      assert.equal(url.searchParams.get("contract_id"), "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
      assert.equal(url.searchParams.get("limit"), "10");
      return Response.json([]);
    },
  });
  const out = await getQuarantine(g, {
    contract_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
    limit: 10,
  });
  assert.deepEqual(out, []);
});

test("list_contracts GET /contracts", async () => {
  const g = gw({
    "GET /contracts": async () => Response.json([{ id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa" }]),
  });
  const out = (await listContracts(g)) as { id: string }[];
  assert.equal(out[0].id, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
});

test("createServer registers without throwing", () => {
  const server = createServer({
    env: { CONTRACTGATE_API_KEY: "cg_live_test" },
  });
  assert.ok(server);
});

test("401 becomes GatewayError", async () => {
  const g = gw({
    "GET /contracts": async () => new Response("nope", { status: 401 }),
  });
  await assert.rejects(
    () => listContracts(g),
    (err: unknown) => err instanceof GatewayError && err.status === 401,
  );
});
