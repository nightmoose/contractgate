/**
 * ContractGate v1 ingest — Node.js example (native fetch, no dependencies).
 *
 * Requires Node 18+ (built-in fetch).
 *
 * Run against the public demo:
 *   export CONTRACTGATE_API_KEY=cg_live_<your_key>
 *   export CONTRACTGATE_CONTRACT_ID=<uuid>
 *   node node_example.js
 */

const BASE_URL    = process.env.CONTRACTGATE_BASE_URL    ?? "https://contractgate.io";
const API_KEY     = process.env.CONTRACTGATE_API_KEY;
const CONTRACT_ID = process.env.CONTRACTGATE_CONTRACT_ID;

if (!API_KEY || !CONTRACT_ID) {
  console.error("Set CONTRACTGATE_API_KEY and CONTRACTGATE_CONTRACT_ID");
  process.exit(1);
}

// ---------------------------------------------------------------------------
// JSON array body
// ---------------------------------------------------------------------------

async function ingestJson() {
  const events = [
    { user_id: "u_001", event_type: "login",    timestamp: 1_714_000_000 },
    { user_id: "u_002", event_type: "purchase", timestamp: 1_714_000_001, amount: 49.99 },
    { user_id: "u_003", event_type: "bad_type", timestamp: 1_714_000_002 }, // will fail
  ];

  const res = await fetch(`${BASE_URL}/v1/ingest/${CONTRACT_ID}`, {
    method: "POST",
    headers: {
      "X-Api-Key":      API_KEY,
      "Content-Type":   "application/json",
      "Idempotency-Key": `node-example-${Date.now()}`,
    },
    body: JSON.stringify(events),
  });

  const data = await res.json();
  console.log(`JSON batch: total=${data.total} passed=${data.passed} failed=${data.failed}`);
  for (const r of data.results) {
    const mark = r.passed ? "✓" : "✗";
    const qid  = r.quarantine_id ? `  quarantine_id=${r.quarantine_id}` : "";
    console.log(`  [${mark}] index=${r.index}${qid}`);
  }
  console.log(`  X-RateLimit-Remaining: ${res.headers.get("x-ratelimit-remaining")}`);
}

// ---------------------------------------------------------------------------
// NDJSON body
// ---------------------------------------------------------------------------

async function ingestNdjson() {
  const lines = [
    { user_id: "u_ndjson_1", event_type: "view",  timestamp: 1_714_000_100 },
    { user_id: "u_ndjson_2", event_type: "click", timestamp: 1_714_000_101 },
  ].map(e => JSON.stringify(e)).join("\n") + "\n";

  const res = await fetch(`${BASE_URL}/v1/ingest/${CONTRACT_ID}`, {
    method: "POST",
    headers: {
      "X-Api-Key":    API_KEY,
      "Content-Type": "application/x-ndjson",
    },
    body: lines,
  });

  const data = await res.json();
  console.log(`\nNDJSON batch: total=${data.total} passed=${data.passed}`);
}

(async () => {
  await ingestJson();
  await ingestNdjson();
})();
