# AISchemaGate Launch — Tester Handover Brief

**Date:** 2026-10-12
**Branch:** `nightly-maintenance-2026-10-11-rfc094-096-aischemagate`
**Head:** `84fd819`
**Status:** All in-repo work committed. Ready for pre-launch verification.
**Design:** [RFC-094](../rfcs/094-sdk-llm-output-guard.md),
[RFC-095](../rfcs/095-llm-agent-integration-docs-and-positioning.md),
[RFC-096](../rfcs/096-aischemagate-frontend-repo.md).

> This document is temporary — delete it (or move it to
> `docs/ops/archive/`) once the launch is signed off.

---

## 1. What landed in this branch (8 commits)

| # | Commit | Scope |
|---|---|---|
| 1 | `0ffb1ba` | Three RFC drafts with signed-off decisions |
| 2 | `520019c` | Pre-existing packaging bug fix (empty `contractgate/__init__.py` stub was shadowing `src/contractgate/`) |
| 3 | `8d48296` | RFC-094 Python: `contractgate.llm` module + 30 tests |
| 4 | `a12863d` | RFC-094 TypeScript: `./llm` subpath export + 29 tests |
| 5 | `b486a35` | RFC-094 alias packages: `aischemagate` (PyPI) + `@nightmoose/aischemagate` (npm) + core SDK 0.3.0 bump |
| 6 | `34ad837` | RFC-094 docs: `docs/llm-sdk-reference.md` (Python + TS side-by-side) |
| 7 | `c9ee93e` | RFC-095: `docs/llm-output-validation.md` playbook + drift gate + README paragraph |
| 8 | `84fd819` | RFC-096: `DASHBOARD_ORIGIN` examples extended to list aischemagate origins in `.env.example`, `docker-compose.yml`, `fly.toml`, `docs/ops/runbook-production.md`, `docs/auth-reference.md` |

Net: +4,934 / -14 lines across 38 files.

## 2. Test matrix — how to verify locally

Everything below should pass out-of-the-box. If any assertion fails,
stop and report which commit / file / test — do not patch around it.

### 2.1 Rust engine + drift gate

```bash
cargo test                              # full suite — baseline
cargo test --test llm_docs_test         # RFC-095 drift gate (6 tests)
```

The drift gate (`tests/llm_docs_test.rs`) asserts the output-validation
playbook only cites routes that exist in `src/main.rs` and that the
example contract YAML parses + compiles through the real engine. Two
tests are new (RFC-095); should see **6/6 passing**.

### 2.2 Python `contractgate` SDK

```bash
cd sdks/python
./.venv/bin/python -m pytest tests/ -v
```

Expect **81/81 passing**: 51 pre-existing (client, contract parse,
validator) + 30 new in `test_llm_gate.py` (extractor shape matrix,
construction, `validate_only`, `completion` with 3-arm auto-repair,
Anthropic `message()`, `mask_prompt`, streaming rejection, async
sanity, dataclass shape).

**Note on venv state:** the venv has an editable install pointing at
`src/`. If you blow it away, recreate with `python -m venv .venv &&
./.venv/bin/pip install -e ".[dev]"`.

### 2.3 TypeScript `@nightmoose/contractgate-sdk` SDK

```bash
cd sdks/typescript
./node_modules/.bin/tsc --noEmit        # type-check
./node_modules/.bin/tsx --test tests/*.test.ts
```

Expect **90/90 passing** (61 existing + 29 new in `tests/llm.test.ts`).
Build also smoke-tested: `./node_modules/.bin/tsc` emits
`dist/llm.{js,d.ts}` and the five expected symbols are re-exported.

### 2.4 Python `aischemagate` alias package

```bash
cd sdks/aischemagate/python
PYTHONPATH="src:../../python/src" ../../python/.venv/bin/python -m pytest tests/ -v
```

Expect **6/6 passing**. Tests assert every exported symbol in
`contractgate.__all__` is reachable via `aischemagate` and resolves to
**the same Python object** (not a copy), plus `aischemagate.LLMGate is
contractgate.llm.LLMGate` and the `llm=None` guard still fires.

### 2.5 npm `@nightmoose/aischemagate` alias package

The package depends on `@nightmoose/contractgate-sdk ^0.3.0`, which is
not yet published. Local dev uses a symlink. From a fresh checkout:

```bash
cd sdks/aischemagate/typescript
mkdir -p node_modules/@nightmoose
ln -s ../../../../typescript node_modules/@nightmoose/contractgate-sdk
../../typescript/node_modules/.bin/tsc --noEmit
../../typescript/node_modules/.bin/tsx --test tests/imports.test.ts
```

Expect **8/8 passing** + clean typecheck.

### 2.6 Full repo build

```bash
cargo check                             # should be clean
cd dashboard && npm run build           # existing dashboard builds (unaffected)
```

## 3. Known gaps — not verified in this branch

These need live-infra validation before launch. Do not treat as
"already done":

1. **No test runs against the live gateway.** All SDK tests use
   `httpx.MockTransport` (Python) or `fetch` injection (TS). An
   end-to-end test with a real `cg_live_...` key against
   `app.datacontractgate.com/egress/{contract_id}` is pending.
2. **CORS smoke test from an aischemagate.com origin.** The config
   examples in commit 8 are documentation only. The actual Fly secret
   flip is operator-side (see §5).
3. **npm publish dry-run.** `aischemagate@0.1.0` has not been
   `npm publish --dry-run`'d. Run from `sdks/aischemagate/typescript/`
   after installing deps once the core SDK is published.
4. **PyPI build.** `pip wheel .` in `sdks/aischemagate/python/`
   has not been run. Hatchling should emit a wheel cleanly, but verify
   before upload to TestPyPI.
5. **Doc copy to `aischemagate.dev`.** RFC-095 §Design 2 Option B
   specs a sync script in the sibling frontend repo. That repo does
   not exist yet (RFC-096), so the sync is unverified.
6. **Dashboard prebuild hook.** RFC-089 shipped a prebuild step that
   copies `docs/llm-integration.md` into `dashboard/public/`. The new
   `docs/llm-output-validation.md` and `docs/llm-sdk-reference.md`
   should also be copied — check `dashboard/scripts/` or
   `dashboard/package.json` prebuild script and extend if needed.
7. **MCP server tool list.** The RFC-093 `egress_validate` tool is
   mentioned in the output-validation playbook §5.7; verify the tool
   name exactly matches what the shipped MCP server exposes.
8. **Example `tool_choice` syntax** for OpenAI tool calling (playbook
   §5.2) — pulled from current OpenAI SDK docs; verify against the
   OpenAI SDK version your tests target.

## 4. Pre-launch acceptance tests the tester should run

For each, document pass/fail and the exact command that produced the
output.

### 4.1 SDK happy-path against live gateway

```python
import os
from aischemagate import LLMGate
from openai import OpenAI

gate = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id="<deployed contract uuid>",
    llm=OpenAI(),
)
result = gate.completion(
    model="gpt-4o-mini",
    response_format={"type": "json_object"},
    messages=[{"role": "user", "content": "Return a user_event JSON with user_id='alice'."}],
)
assert result.ok, result.violations
```

### 4.2 SDK auto-repair against a known-bad prompt

Prompt the model to produce an out-of-range value (e.g. a negative
`amount` when the contract declares `min: 0`). Confirm `auto_repair=True`
flips the result to `ok=True` within 2 retries.

### 4.3 Alias package install

In a scratch venv / project:

```bash
pip install aischemagate            # once published; otherwise install from the local build
python -c "from aischemagate import LLMGate; print(LLMGate)"
```

```bash
npm install @nightmoose/aischemagate   # once published
node -e "import('@nightmoose/aischemagate').then(m => console.log(m.LLMGate))"
```

Confirm the symbol resolves and construction with a fake `llm`
succeeds (per the import tests).

### 4.4 Docs render correctly

- `curl https://app.datacontractgate.com/llm-output-validation.md`
  returns the raw file once the dashboard prebuild hook is extended.
- `curl https://aischemagate.dev/llm-output-validation.md` returns the
  raw file once RFC-096 is executed.
- `curl https://aischemagate.dev/docs/llm-output-validation` returns a
  rendered page (RFC-095 §Design 2 Option B).

### 4.5 CORS smoke from a staging frontend

With `DASHBOARD_ORIGIN` extended on **staging** Fly to include
`https://aischemagate.com,https://aischemagate.dev`:

```bash
curl -i -X OPTIONS "https://staging-contractgate-api.fly.dev/egress/<cid>" \
     -H "Origin: https://aischemagate.com" \
     -H "Access-Control-Request-Method: POST" \
     -H "Access-Control-Request-Headers: x-api-key,content-type"
```

Expect `HTTP/2 204` + `access-control-allow-origin:
https://aischemagate.com`. Also verify a disallowed origin
(`https://evil.example`) does **not** receive the header.

## 5. Operator-side launch sequence (not in this branch)

Spec'd in RFC-096 §Design 10. All reversible with a single env change
or DNS rollback.

1. **Publish core SDKs.** `contractgate@0.3.0` to PyPI,
   `@nightmoose/contractgate-sdk@0.3.0` to npm.
2. **Publish alias packages.** `aischemagate@0.1.0` and
   `@nightmoose/aischemagate@0.1.0` once (1) is live and resolvable.
3. **Stand up `nightmoose/aischemagate-www`** on Vercel preview.
   Repo creation + initial clone-and-strip from `dashboard/` is the
   biggest piece of pending work. See RFC-096 §Design 1 for repo
   layout.
4. **Flip staging `DASHBOARD_ORIGIN`** on Fly to include the two new
   origins. Smoke test §4.5.
5. **DNS cutover** for `aischemagate.com` and `aischemagate.dev` to
   the Vercel edge.
6. **Flip production `DASHBOARD_ORIGIN`** on Fly:
   ```bash
   fly secrets set DASHBOARD_ORIGIN=https://app.datacontractgate.com,https://aischemagate.com,https://aischemagate.dev
   ```
7. **Launch announcement.** Marketing applies RFC-095 Appendix A
   verbatim — any copy not in Appendix A must be removed before
   publish.

## 6. Rollback

- **Marketing-only rollback** (frontend broken, backend fine): revert
  Vercel to the previous deploy; marketing site goes down; dashboard
  + ingest + egress keep working.
- **CORS rollback** (prod CORS misconfigured, legitimate browser
  requests failing): `fly secrets set
  DASHBOARD_ORIGIN=https://app.datacontractgate.com` to restore the
  single-origin state. Takes ~30s to propagate.
- **DNS rollback**: revert the A/ALIAS record for aischemagate.com /
  .dev. Vercel's edge continues to serve stale until TTL expires.
- **SDK publish rollback**: npm/PyPI do not support unpublish beyond
  72h. Publish a `0.3.1` with the fix instead. The alias packages are
  trivially re-publishable since they re-export only.

## 7. Open items for the tester to probe

- **Prebuild hook extension.** Grep `dashboard/package.json` and
  `dashboard/scripts/` for the RFC-089 prebuild step that copies
  `llm-integration.md` into `dashboard/public/`. Verify the new docs
  are added. If missing, flag — RFC-095 §Design 7 called this out but
  the extension itself was not committed in this branch.
- **MCP tool name.** Confirm the exact tool name is `egress_validate`
  (snake_case) in the shipped MCP server. The playbook cites it in
  §5.7.
- **`contractgate` 0.3.0 release notes vs. CHANGELOG.md.** The
  `sdks/python/CHANGELOG.md` and new `sdks/typescript/CHANGELOG.md`
  carry the release notes; make sure the GitHub Release description
  pulls from or links to them.
- **Playbook §2 example syntax.** The 10-line Python quickstart in
  `docs/llm-output-validation.md` constructs an `OpenAI()` client
  twice (one for the gate, one for the direct `create()` call) — this
  is intentional to make the data flow legible, but the tester may
  want to confirm it reads cleanly against a fresh user.

## 8. Where to find things

| Looking for | Where |
|---|---|
| Design rationale | `docs/rfcs/09{4,5,6}-*.md` |
| Public API ref | `docs/llm-sdk-reference.md` |
| Developer playbook | `docs/llm-output-validation.md` |
| Agent playbook (ingest side) | `docs/llm-integration.md` |
| Marketing copy lock | RFC-095 Appendix A (`docs/rfcs/095-...md`) |
| CORS config semantics | `docs/auth-reference.md` + `src/main.rs:1252-1298` |
| Python SDK impl | `sdks/python/src/contractgate/llm.py` |
| TS SDK impl | `sdks/typescript/src/llm.ts` |
| Alias packages | `sdks/aischemagate/{python,typescript}/` |

## 9. Contact

- Launch owner: Alex Suarez (`alex@fullscoredata.com`).
- The branch is on `main` fork at
  `nightmoose/contractgate` with name
  `nightly-maintenance-2026-10-11-rfc094-096-aischemagate`.
- Memory of in-session decisions lives at
  `/Users/alexsuarez/.claude-work/projects/-Users-alexsuarez-contractgate/memory/`
  — not required for the tester, but useful if they inherit a Claude
  session.
