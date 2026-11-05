import assert from "node:assert/strict";
import test from "node:test";

// The route captures EARLY_ACCESS_NOTIFY_TO at import time, so clear it before
// loading the module to exercise the "recipient not configured" path.
delete process.env.EARLY_ACCESS_NOTIFY_TO;
const { OPTIONS, POST } = await import("./route");

function postRequest(body: unknown): Request {
  return new Request("http://localhost/api/early-access", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

test("OPTIONS preflight returns 204 with CORS headers", async () => {
  const res = await OPTIONS();
  assert.equal(res.status, 204);
  assert.equal(res.headers.get("Access-Control-Allow-Origin"), "*");
  assert.equal(res.headers.get("Access-Control-Allow-Methods"), "POST, OPTIONS");
});

test("POST rejects a request missing required fields with 400", async () => {
  // Missing `stack`.
  const res = await POST(postRequest({ name: "Ada", email: "ada@example.com" }));
  assert.equal(res.status, 400);
  assert.equal(res.headers.get("Access-Control-Allow-Origin"), "*");
  const body = (await res.json()) as { error: string };
  assert.match(body.error, /required/i);
});

test("POST returns 500 (no crash) when EARLY_ACCESS_NOTIFY_TO is unset", async () => {
  const res = await POST(
    postRequest({ name: "Ada", email: "ada@example.com", stack: "dbt + Snowflake" })
  );
  assert.equal(res.status, 500);
  assert.equal(res.headers.get("Access-Control-Allow-Origin"), "*");
  const body = (await res.json()) as { error: string };
  assert.ok(typeof body.error === "string" && body.error.length > 0);
});
