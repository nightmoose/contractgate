/**
 * API endpoint dogfood (HTTP). Uses Playwright's request fixture.
 *
 * Public: health / ready / openapi
 * Auth: CG_API_KEY for contracts, usage, ingest, quarantine
 */
import { test, expect } from "@playwright/test";
import * as fs from "fs";
import * as path from "path";

const API =
  process.env.CG_API_URL?.replace(/\/$/, "") || "https://contractgate-api.fly.dev";
const KEY = process.env.CG_API_KEY?.trim() || "";

test.describe("Public API", () => {
  test("GET /health", async ({ request }) => {
    const res = await request.get(`${API}/health`);
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.status || body.service).toBeTruthy();
  });

  test("GET /ready", async ({ request }) => {
    const res = await request.get(`${API}/ready`);
    // ready may be 200 or 503 depending on deps — not 404
    expect([200, 503]).toContain(res.status());
  });

  test("GET /openapi.json", async ({ request }) => {
    const res = await request.get(`${API}/openapi.json`);
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.openapi || body.paths).toBeTruthy();
  });

  test("GET /catalog is public", async ({ request }) => {
    const res = await request.get(`${API}/catalog`);
    expect(res.status()).toBeLessThan(500);
  });

  test("protected routes reject missing key", async ({ request }) => {
    const res = await request.get(`${API}/usage`);
    expect([401, 403]).toContain(res.status());
  });
});

test.describe("Authenticated API", () => {
  test.skip(!KEY, "Set CG_API_KEY for authenticated API dogfood");

  test("GET /usage", async ({ request }) => {
    const res = await request.get(`${API}/usage`, {
      headers: { "X-Api-Key": KEY },
    });
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body).toBeTruthy();
  });

  test("GET /contracts", async ({ request }) => {
    const res = await request.get(`${API}/contracts`, {
      headers: { "X-Api-Key": KEY },
    });
    expect(res.status()).toBe(200);
  });

  test("GET /quarantine", async ({ request }) => {
    const res = await request.get(`${API}/quarantine`, {
      headers: { "X-Api-Key": KEY },
    });
    expect(res.status()).toBeLessThan(500);
  });

  test("dogfood USGS: deploy + ingest pass/fail", async ({ request }) => {
    const yamlPath = path.resolve(__dirname, "../contracts/usgs_earthquake.yaml");
    const passPath = path.resolve(__dirname, "../fixtures/usgs_earthquake/pass.ndjson");
    const failPath = path.resolve(__dirname, "../fixtures/usgs_earthquake/fail.ndjson");
    test.skip(!fs.existsSync(yamlPath), "missing contract — run author_contract");
    test.skip(!fs.existsSync(passPath), "missing fixtures — run run_local");

    const yaml = fs.readFileSync(yamlPath, "utf8");
    const headers = {
      "X-Api-Key": KEY,
      "Content-Type": "application/json",
      Accept: "application/json",
    };

    // Create (or reuse) + promote to stable. Deploy may 400 if quarantine pending.
    let contractId: string | undefined;
    const name = `usgs_dogfood_${Date.now()}`;
    const create = await request.post(`${API}/contracts`, {
      headers,
      data: { name, yaml_content: yaml },
    });
    expect(create.status(), await create.text()).toBeLessThan(300);
    const created = await create.json();
    contractId = created.id || created.contract_id;
    expect(contractId, "contract id").toBeTruthy();

    // Create version if not implied by POST body, then promote
    const versions = await request.get(`${API}/contracts/${contractId}/versions`, {
      headers: { "X-Api-Key": KEY },
    });
    let versionLabel = "1.0.0";
    if (versions.ok()) {
      const vbody = await versions.json();
      const items = Array.isArray(vbody) ? vbody : vbody.versions || [];
      if (items.length === 0) {
        const ver = await request.post(`${API}/contracts/${contractId}/versions`, {
          headers,
          data: { yaml_content: yaml, version: "1.0.0" },
        });
        expect(ver.status(), await ver.text()).toBeLessThan(300);
      } else {
        versionLabel = items[0].version || versionLabel;
      }
    }
    const promote = await request.post(
      `${API}/contracts/${contractId}/versions/${versionLabel}/promote`,
      { headers, data: {} }
    );
    // 200 or already stable
    expect(promote.status(), await promote.text()).toBeLessThan(500);

    const passBody = fs.readFileSync(passPath);
    const passRes = await request.post(`${API}/v1/ingest/${contractId}`, {
      headers: {
        "X-Api-Key": KEY,
        "Content-Type": "application/x-ndjson",
        Accept: "application/json",
      },
      data: passBody,
    });
    expect(passRes.status(), await passRes.text()).toBeLessThan(300);
    const passJson = await passRes.json();
    expect(passJson.failed ?? 0).toBe(0);
    expect(passJson.passed ?? passJson.total).toBeGreaterThan(0);

    if (fs.existsSync(failPath)) {
      const failBody = fs.readFileSync(failPath);
      const failRes = await request.post(`${API}/v1/ingest/${contractId}`, {
        headers: {
          "X-Api-Key": KEY,
          "Content-Type": "application/x-ndjson",
          Accept: "application/json",
        },
        data: failBody,
      });
      // All-fail batches often return 422 — still a valid product response
      expect(failRes.status()).toBeLessThan(500);
      const failJson = await failRes.json();
      expect(failJson.failed ?? 0).toBeGreaterThan(0);
    }

    const report = await request.get(`${API}/contracts/${contractId}/report`, {
      headers: { "X-Api-Key": KEY },
    });
    expect(report.status()).toBeLessThan(500);
  });
});
