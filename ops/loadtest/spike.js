// k6 load test — "Show HN" traffic spike against the DataContractGate /
// AISchemaGate gateway.
//
// Targets STAGING by default. Point it elsewhere with BASE_URL, but it refuses
// to run against production (api.datacontractgate.com) unless ALLOW_PROD=1 so a
// stray run cannot DoS the real gateway.
//
// Run:
//   BASE_URL=https://contractgate-api-staging.fly.dev k6 run ops/loadtest/spike.js
//
// Add the authenticated ingest scenario by also setting API_KEY and CONTRACT_ID:
//   BASE_URL=... API_KEY=cg_... CONTRACT_ID=<uuid> k6 run ops/loadtest/spike.js
//
// See ops/loadtest/README.md.

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Rate } from 'k6/metrics';

// ── Config (env) ───────────────────────────────────────────────────────────
const BASE_URL = (__ENV.BASE_URL || 'https://contractgate-api-staging.fly.dev').replace(/\/$/, '');
const ALLOW_PROD = __ENV.ALLOW_PROD === '1';
const API_KEY = __ENV.API_KEY || '';
const CONTRACT_ID = __ENV.CONTRACT_ID || '';
// A published contract reference to exercise GET /published/{ref}. Optional —
// the hit is skipped when unset so the script still runs on a bare staging DB.
const PUBLISHED_REF = __ENV.PUBLISHED_REF || '';

// Refuse prod unless explicitly allowed. This runs in init context, so a bad
// BASE_URL aborts before any VU starts.
if (BASE_URL.includes('api.datacontractgate.com') && !ALLOW_PROD) {
  throw new Error(
    `Refusing to load test production (${BASE_URL}). ` +
      'Point BASE_URL at staging, or set ALLOW_PROD=1 to override.',
  );
}

const AUTHED = API_KEY !== '' && CONTRACT_ID !== '';

// Error rate for the read mix, tracked separately from the total so an authed
// ingest 4xx (e.g. a deliberately invalid event) does not fail the read SLO.
const readErrors = new Rate('read_errors');

// ── Load profile ─────────────────────────────────────────────────────────
// A Show HN front-page curve: a slow warm-up, a sharp climb as the post gains
// traction, a sustained plateau on the front page, then decay.
const browseStages = [
  { duration: '1m', target: 20 },   // warm-up
  { duration: '2m', target: 100 },  // climbing — post gaining votes
  { duration: '3m', target: 300 },  // front page
  { duration: '5m', target: 300 },  // sustained peak
  { duration: '3m', target: 80 },   // falling off the front page
  { duration: '1m', target: 0 },    // drain
];

// Authed writers are a small fraction of readers on a Show HN day.
const ingestStages = [
  { duration: '1m', target: 2 },
  { duration: '2m', target: 10 },
  { duration: '8m', target: 20 },
  { duration: '3m', target: 5 },
  { duration: '1m', target: 0 },
];

export const options = {
  scenarios: {
    browse: {
      executor: 'ramping-vus',
      exec: 'browse',
      startVUs: 0,
      stages: browseStages,
      gracefulRampDown: '30s',
    },
    ...(AUTHED
      ? {
          ingest: {
            executor: 'ramping-vus',
            exec: 'ingest',
            startVUs: 0,
            stages: ingestStages,
            gracefulRampDown: '30s',
          },
        }
      : {}),
  },
  thresholds: {
    // Overall health of the public read surface.
    read_errors: ['rate<0.02'],
    // Fast endpoints should stay fast under the spike.
    'http_req_duration{endpoint:health}': ['p(95)<200'],
    'http_req_duration{endpoint:catalog}': ['p(95)<800'],
    'http_req_duration{endpoint:public_contracts}': ['p(95)<800'],
    'http_req_duration{endpoint:public_contract}': ['p(95)<800'],
    'http_req_duration{endpoint:published}': ['p(95)<800'],
    // Authed ingest is allowed a looser budget (does real DB work).
    'http_req_duration{endpoint:ingest}': ['p(95)<1500'],
  },
};

// ── Helpers ────────────────────────────────────────────────────────────────
function getRead(path, endpoint, extra) {
  const res = http.get(`${BASE_URL}${path}`, {
    tags: { endpoint },
    ...(extra || {}),
  });
  // 404 on an optional published ref is an expected "not seeded", not an error.
  // status 0 is a dropped/failed request (connection refused, timeout, reset) —
  // the dominant failure mode under a spike, so it must count as an error, not a
  // pass. `r.status < 500` alone would let status 0 through.
  const ok = check(res, {
    [`${endpoint} not 5xx`]: (r) => r.status >= 200 && r.status < 500,
  });
  readErrors.add(!ok);
  return res;
}

// ── Scenarios ────────────────────────────────────────────────────────────
// Public, unauthenticated reads — what a curious visitor browsing the catalog
// and playground hits.
export function browse() {
  getRead('/health', 'health');

  getRead('/catalog', 'catalog');

  // List, then drill into a single public contract — the "playground-ish"
  // read path a visitor follows from the catalog into a contract preview.
  const list = getRead('/public-contracts', 'public_contracts');
  let firstId;
  try {
    const body = list.json();
    const items = Array.isArray(body) ? body : body && body.items;
    if (Array.isArray(items) && items.length > 0) {
      firstId = items[0].id || items[0].contract_id;
    }
  } catch (_) {
    // Non-JSON / empty — nothing to drill into.
  }
  if (firstId) {
    getRead(`/public-contracts/${firstId}`, 'public_contract');
  }

  if (PUBLISHED_REF) {
    getRead(`/published/${PUBLISHED_REF}`, 'published');
  }

  // Think time between page views.
  sleep(Math.random() * 2 + 0.5);
}

// Authenticated bulk ingest — only registered when API_KEY and CONTRACT_ID are
// set. Posts one valid-shaped event per iteration.
export function ingest() {
  const payload = JSON.stringify([
    {
      user_id: `k6_${__VU}_${__ITER}`,
      event_type: 'view',
      timestamp: Math.floor(1714000000 + __ITER),
      amount: 1.0,
    },
  ]);
  const res = http.post(`${BASE_URL}/v1/ingest/${CONTRACT_ID}`, payload, {
    headers: {
      'Content-Type': 'application/json',
      'X-Api-Key': API_KEY,
    },
    tags: { endpoint: 'ingest' },
  });
  check(res, {
    'ingest accepted': (r) => r.status === 200 || r.status === 207,
    'ingest not 5xx': (r) => r.status >= 200 && r.status < 500,
  });
  sleep(Math.random() + 0.2);
}
