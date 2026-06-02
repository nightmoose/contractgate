# RFC-005: Python SDK

| Status        | Accepted (2026-04-26)                                                   |
|---------------|-------------------------------------------------------------------------|
| Author        | ContractGate team                                                       |
| Created       | 2026-04-26                                                              |
| Accepted      | 2026-04-26 — Alex sign-off on Q1, Q2, Q4, Q6 (recommendations); Q3, Q5 default |
| Target branch | `nightly-maintenance-2026-04-26`                                        |
| Tracking      | Adoption / DX surface                                                   |
| Depends on    | RFC-002 (versioning), RFC-004 (PII transforms) — both landed            |

## Summary

Ship a first-party **Python SDK** at `sdks/python/` (PyPI name
`contractgate`). Three capabilities, in adoption order:

1. **HTTP client for the gateway** — `validate(...)`, `ingest(...)`,
   `audit(...)`, contract CRUD reads. Wraps the existing REST API.
   Sync + async (httpx).
2. **Local validator** — pure-Python port of `src/validation.rs`. Parse
   contract YAML, compile once, validate many. No network, useful for
   unit tests in user code and pre-commit hooks. Mirrors Rust's
   compile-once-validate-many shape.
3. **Audit log query helper** — paginated reads of `/audit`,
   `/quarantine`, `/contracts/:id/quarantine/:qid/replay-history`.
   Iterator + filter ergonomics.

The SDK is the lowest-risk, highest-leverage adoption surface we have:
every Python data team that hits the gateway today writes their own
`requests.post(...)` wrapper. Owning that wrapper means we control the
DX, the retry semantics, the masking-respect contract, and the upgrade
path when we change the wire shape.

## Goals

1. **First-class sync + async**. Both ship in v0.1; both share the same
   transport layer (httpx) and the same response models.
2. **No external runtime deps beyond `httpx` and `PyYAML`.** Pure-Python
   validator, no C extensions, no Rust bindings (those are a future
   RFC — `contractgate-rs` via PyO3).
3. **Python 3.9+** for broad adoption. Type hints written in `from
   __future__ import annotations` style so 3.10+ union syntax is
   parsed but not required.
4. **Validator parity with the Rust engine.** Same violation kinds,
   same message shape, same field-path format, same per-event
   ordering. Locked via shared JSON fixtures (see Test plan §V).
5. **Respect the audit-honesty invariant.** When the SDK calls
   `/ingest`, it surfaces `contract_version` from the response on each
   per-event result — never substitutes the resolved/requested
   version. Same rule the gateway already follows.
6. **Stable error taxonomy.** One exception hierarchy
   (`ContractGateError` → `HTTPError`, `ValidationError`,
   `ContractCompileError`, `AuthError`) so callers can `except` on
   semantics rather than HTTP status codes.

## Non-goals

- **PII transform parity in the local validator.** The local validator
  is *read-only*: it reports pass/fail and violations, it does NOT run
  `mask` / `hash` / `drop` / `redact`. Reasons: (a) the
  `format_preserving` mask uses ChaCha20 seeded on the per-contract
  `pii_salt`, which the SDK can't see (it's never in API responses by
  design — RFC-004); (b) "what gets stored" is a server-side question
  and we don't want to ship two implementations that can drift. Local
  validation is for `assert event_passes_contract(...)` in user tests,
  not for replicating the gateway's storage path.
- **Auto-retry / circuit breakers.** Out of scope for v0.1. Document
  the recommended `httpx.HTTPTransport(retries=...)` pattern in the
  README and let users layer `tenacity` if they want more. RFC-003's
  replay endpoint is the right answer for failed events; client-side
  retry would create double-write risk.
- **Streaming ingest / Kafka shim.** No. The gateway has no streaming
  ingest API today, and the demo SSE stream is dashboard-only.
- **A CLI.** Out of scope for v0.1. If users want one, it lands in a
  follow-up RFC alongside contract import/export tooling.
- **Rust-backed validator (PyO3 bindings).** Deferred. Pure-Python is
  fine for the unit-test use case (the only local-validator use case
  in scope). When someone needs <1ms validation in Python prod, that's
  a follow-up RFC.
- **Browser/Pyodide compatibility.** Not a target. `httpx` and
  `PyYAML` both work there, but we will not block on it or test it.

## Current state

- No SDK exists. Users hit the REST API directly with `requests` /
  `httpx` / `aiohttp` and roll their own response parsing.
- The dashboard is the only first-party HTTP client today, and it
  lives in `dashboard/` (Next.js, TypeScript). It is not reusable
  outside the browser.
- The Rust validator (`src/validation.rs`, `src/contract.rs`) is the
  canonical reference implementation. Any local-validation port must
  treat it as the spec.
- The wire shapes are already stable: `BatchIngestResponse`,
  `IngestEventResult`, `Violation`, `ViolationKind` (all in
  `src/ingest.rs` + `src/validation.rs`). The SDK pins to those names
  exactly.

## Design

### Package layout

```
sdks/python/
├── pyproject.toml               # PEP 621 metadata, hatchling build
├── README.md                    # quickstart + parity caveats
├── LICENSE                      # MIT (matches root LICENSE)
├── src/
│   └── contractgate/
│       ├── __init__.py          # public re-exports
│       ├── _version.py          # 0.1.0
│       ├── client.py            # sync Client
│       ├── async_client.py      # async AsyncClient
│       ├── _transport.py        # shared httpx wiring (auth, base_url)
│       ├── models.py            # @dataclass response shapes
│       ├── exceptions.py        # error hierarchy
│       ├── contract.py          # YAML → Contract dataclass + compile
│       └── validator.py         # pure-Python validate()
└── tests/
    ├── test_client_sync.py
    ├── test_client_async.py
    ├── test_validator.py
    ├── test_contract_parse.py
    └── fixtures/
        ├── contracts/*.yaml
        └── parity/*.json        # event + expected violations, shared with Rust
```

### Public API surface (v0.1)

```python
from contractgate import Client, AsyncClient, Contract, ValidationResult

# 1. HTTP client
c = Client(base_url="https://gw.example.com", api_key="cg_live_...")
result = c.ingest(contract_id="...", events=[{...}, {...}])
for r in result.results:
    if not r.passed:
        for v in r.violations:
            print(v.field, v.kind, v.message)

# audit reads
for entry in c.audit(contract_id="...", limit=200):
    ...

# 2. Async equivalent
async with AsyncClient(base_url=..., api_key=...) as ac:
    result = await ac.ingest(contract_id="...", events=[...])

# 3. Local validator
contract = Contract.from_yaml(open("user_events.yaml").read())
compiled = contract.compile()
vr = compiled.validate({"user_id": "alice_01", "event_type": "click", ...})
assert vr.passed, vr.violations
```

### Transport layer (`_transport.py`)

- One private `_BaseTransport` class holds `base_url`, `api_key`,
  default headers (`x-api-key`, `User-Agent: contractgate-python/0.1`),
  and timeout config.
- `Client` wraps `httpx.Client`; `AsyncClient` wraps
  `httpx.AsyncClient`. Both forward to the same `_BaseTransport`
  request-building helpers, so the two clients can never drift on
  header / auth shape.
- Errors are translated centrally: `4xx → ContractGateError` subclass
  by status code, `5xx → ContractGateError` with raw response
  attached. Network-level exceptions wrap to `ConnectionError`.
- `x-org-id` header is supported as an explicit kwarg
  (`Client(..., org_id="...")`) for legacy/dev mode, mirroring the
  gateway's resolution order in `main.rs::org_id_from_req`.

### Local validator (`validator.py`, `contract.py`)

Direct port of `src/contract.rs` + `src/validation.rs`. Each Rust type
maps to a Python dataclass:

| Rust                       | Python                          |
|----------------------------|---------------------------------|
| `Contract`                 | `Contract` (frozen dataclass)   |
| `FieldDefinition`          | `FieldDefinition`               |
| `FieldType`                | `FieldType` (Enum)              |
| `MetricDefinition`         | `MetricDefinition`              |
| `Transform`                | `Transform` (declared, not run) |
| `CompiledContract`         | `CompiledContract`              |
| `ValidationResult`         | `ValidationResult`              |
| `Violation`                | `Violation`                     |
| `ViolationKind`            | `ViolationKind` (Enum)          |

Compile stage:
- Parse YAML (PyYAML, safe_load).
- Pre-compile every `pattern` via `re.compile`. A bad regex raises
  `ContractCompileError`.
- Build the declared-top-level set when `compliance_mode = true`
  (matches Rust's HashSet).
- `validate_transform_types`: reject contracts that declare a
  transform on a non-string field. **Same error message text as
  Rust** so users see one error string regardless of which side
  caught it.

Validate stage — same per-event order as Rust:
1. Walk ontology fields recursively (required / type / pattern /
   enum / range / length).
2. Walk metrics with `field` set + `min`/`max` bounds.
3. If `compliance_mode = true`, append `UNDECLARED_FIELD` violations
   for every top-level key not in the declared set.

`ViolationKind` values are the snake_case strings the Rust enum
serializes to (`missing_required_field`, `type_mismatch`, etc.) so a
violation produced by the gateway and a violation produced locally
deserialize into the same Python value.

### Response models (`models.py`)

`@dataclass(frozen=True)` for everything the gateway returns:
`IngestEventResult`, `BatchIngestResponse`, `AuditEntry`,
`QuarantineEvent`, `ContractResponse`, `VersionResponse`. Field names
match the JSON exactly (snake_case both sides — no rename layer).
Decoding is a thin `from_dict` constructor; no third-party
serialization library.

### Error hierarchy (`exceptions.py`)

```
ContractGateError
├── HTTPError                     # response-attached, has .status, .body
│   ├── BadRequestError           # 400
│   ├── AuthError                 # 401
│   ├── NotFoundError             # 404
│   ├── ConflictError             # 409 (NoStableVersion)
│   ├── ValidationFailedError     # 422 (atomic reject, all-failed batch)
│   └── ServerError               # 5xx
├── ConnectionError               # transport/DNS/timeout
└── ContractCompileError          # local YAML parse / compile failure
```

Note: per-event validation **failures** in a 207 Multi-Status response
do NOT raise — they're reflected in `BatchIngestResponse.results`.
Only whole-batch rejects (422) raise.

### Versioning + release

- Independent SemVer for the SDK. v0.1.0 ships the surface above;
  breaking changes during 0.x are allowed but documented in
  `sdks/python/CHANGELOG.md`.
- The SDK pins the *wire shapes* listed under "Current state" and
  treats them as part of the gateway's public API. If the gateway
  changes a wire shape in a breaking way, the SDK gets a major bump
  and we coordinate the release.
- PyPI publish: out of scope for the RFC's first landing — the v0.1
  PR ships the package source under `sdks/python/` and a `make
  python-sdk-build` target. Publishing to PyPI is a separate ops PR
  once we have the package name reserved.

## Decisions (signed off 2026-04-26)

- **Q1 → `contractgate`.** Reserve `contractgate-sdk` and `cg-sdk` as
  redirect packages once the primary name is claimed.
- **Q2 → Separate `Client` and `AsyncClient`.** Matches `httpx`'s own
  pattern; avoids runtime-flag ambiguity at call sites.
- **Q3 → `httpx>=0.25,<1.0`.** Default. Wide enough for adoption; the
  pre-1.0 caveat lives in the README. Bump to `>=1.0` once httpx 1.x
  ships and we've tested.
- **Q4 → Strict parity.** Byte-identical message text and field paths
  to the Rust validator. Locked via the shared `tests/fixtures/parity/`
  corpus consumed by both `pytest` and a new Rust integration test.
- **Q5 → MIT.** Default; matches the gateway's existing code license
  intent. Patent-pending status is independent of the SDK's source
  license. Revisit if legal flags an Apache-2.0 patent-grant
  preference.
- **Q6 → No transforms in local validator.** Read-only validator. The
  per-contract `pii_salt` is never exposed to clients (RFC-004
  invariant); the gateway is the single source of truth for the
  post-transform payload. The wire response's `transformed_event`
  field is what callers should rely on for "what got stored."

## Original options (kept for record)

- **Q1: Package name.** `contractgate` (clean, but unverified on
  PyPI), `contractgate-sdk` (defensive), or `cg-sdk` (terse). I
  recommend `contractgate` and reserve the others as redirect
  packages.
- **Q2: Sync/async surface — separate classes or unified?** I
  recommend separate classes (`Client`, `AsyncClient`) for clarity
  and to match `httpx`'s own pattern. Unified (`Client(async_=True)`)
  is more compact but obscures which methods are awaitable.
- **Q3: Pin httpx?** I recommend `httpx>=0.25,<1.0` (broad range,
  pre-1.0 caveat noted in README). Tighter pin (e.g.,
  `httpx~=0.27.0`) would force lockstep upgrades on users.
- **Q4: Validator output equivalence — strict or human?** Two
  options:
  - **Strict**: byte-for-byte identical messages and field paths to
    Rust. Locks the parity tests but means changing a Rust message
    means changing the SDK in lockstep.
  - **Human**: same `kind` and `field`, message text may differ.
    Looser parity, easier to evolve.
  I recommend **strict** for v0.1 — gives users one canonical
  violation surface across both code paths. We can relax later if it
  turns into a maintenance drag.
- **Q5: License.** MIT (matches the root `LICENSE`)? Or
  Apache-2.0 to give users a patent grant given the patent-pending
  status of the gateway?
- **Q6: Should the local validator run RFC-004 transforms?** I'm
  recommending NO under non-goals (read-only validator, no salt
  visibility). Confirm — this is the most consequential design call
  and it's worth a explicit yes.

## Test plan

### I. Contract parse (no network)
1. Parse the canonical CLAUDE.md YAML example. All fields populated.
2. `compliance_mode: true` parses; default is `false` when omitted.
3. `transform: { kind: mask, style: opaque }` parses.
4. Bad regex → `ContractCompileError`.
5. Transform on non-string field → `ContractCompileError` with the
   same wording as Rust.

### II. Local validator (pure-Python, no network)
6. Pass: valid event for the canonical contract.
7. Each violation kind triggers (one test per `ViolationKind`).
8. Compliance mode: undeclared field → `UndeclaredField`.
9. Compliance mode off: undeclared field passes through.
10. Nested object: `user.address.zip` violation reports the dotted path.
11. Array items: per-index path `tags[3]` reports correctly.
12. Metric range: `min`/`max` violation maps to
    `MetricRangeViolation`.

### III. Sync HTTP client
13. `ingest` happy path: posts to `/ingest/{id}`, parses
    `BatchIngestResponse`, surfaces per-event `transformed_event`.
14. `ingest` with `@version` suffix: passed through verbatim.
15. 401 → `AuthError`, 422 → `ValidationFailedError`,
    409 → `ConflictError`, 404 → `NotFoundError`.
16. 207 Multi-Status: NO exception, both passed and failed events
    visible in `result.results`.
17. `audit(contract_id=..., limit=...)` paginates correctly.
18. `x-api-key` and `x-org-id` headers attached to every request.

### IV. Async HTTP client
19. Same matrix as §13–§18 against `AsyncClient`.
20. `async with AsyncClient(...)` cleans up the underlying
    `httpx.AsyncClient`.

### V. Cross-language parity (the load-bearing test)
21. `tests/fixtures/parity/*.json` — each fixture is `{contract_yaml,
    event, expected_violations}`. The test harness:
    a. Parses + compiles in Python, validates the event, asserts
       `violations == expected`.
    b. Same fixture is consumed by a new Rust integration test
       (`tests/parity_python.rs`) that does the equivalent assert.
    Same set of fixtures drives both — divergence breaks one side
    immediately.
22. At least one fixture per `ViolationKind`. At least one fixture
    with three violations in a row (asserts ordering is identical).

### VI. Build / packaging
23. `pip install -e sdks/python` succeeds in a fresh venv on Python
    3.9, 3.11, 3.12.
24. `pyproject.toml` declares correct deps; `pip-audit` runs clean.

## Rollout

1. Land RFC-005 (this doc) on `nightly-maintenance-2026-04-26` once
   Q1–Q6 are signed off.
2. Scaffold `sdks/python/` with `pyproject.toml`, package skeleton,
   README, `__init__.py` re-exports.
3. Implement `contract.py` + `validator.py` (the canonical-spec
   port). Unit tests §I + §II. Use the existing Rust unit-test
   fixtures in `src/validation.rs::tests` as the seed corpus.
4. Implement `_transport.py`, `models.py`, `exceptions.py`,
   `client.py`. Unit tests §III + §V (sync side).
5. Implement `async_client.py`. Unit tests §IV.
6. Add `tests/parity_python.rs` to the Rust crate; wire `cargo test`
   to consume the shared fixture directory. CI runs both sides on
   every PR.
7. Document the audit-honesty invariant in the SDK README — surface
   `contract_version` from each per-event result, never substitute.
8. Document the RFC-004 caveat: the local validator does not run
   transforms; the wire response's `transformed_event` is the
   authoritative "what got stored" view.
9. Reserve PyPI name in a separate ops task; first publish ships
   when the package name is confirmed.
10. Update `MAINTENANCE_LOG.md` with the rollout entry.

## Decisions locked in before implementation

- Q1–Q6 — all signed off 2026-04-26. No open questions remain.

Work top-down through the rollout list on
`nightly-maintenance-2026-04-26`.
