# RFC-001 Org Isolation — Verification Runbook

Run these checks after applying migration 007 to a staging or production Supabase project.
Each check has a SQL probe (run in the Supabase SQL editor as `postgres` / service-role)
and a UI smoke test.

---

## Pre-flight

```sql
-- Confirm migration landed
select id, name, slug from public.orgs order by created_at;
select org_id, user_id, role from public.org_memberships order by joined_at;

-- Every contract must have an org_id now
select count(*) from public.contracts where org_id is null;  -- expect 0

-- Every api_key must have an org_id now
select count(*) from public.api_keys where org_id is null;   -- expect 0
```

---

## Check 1 — Account A cannot see Account B's contracts

### Setup
1. Sign up as **User A** (e.g. `a@test.com`). Create a contract called `ContractA`.
2. Sign up as **User B** (e.g. `b@test.com`). Create a contract called `ContractB`.

### SQL probe (service-role — ground truth)
```sql
-- Each contract is owned by a different org
select c.name, c.org_id, o.slug
from   public.contracts c
join   public.orgs      o on o.id = c.org_id
order  by c.created_at;
-- Expected: ContractA → org of User A, ContractB → org of User B
```

### RLS probe (run as User A's anon key — simulates what the browser sees)
```sql
-- SET ROLE to the anon role + set the JWT claim to User A
set local role authenticated;
set local request.jwt.claims = '{"sub": "<USER_A_UUID>"}';

select name from public.contracts;
-- Expected: only ContractA appears. ContractB must NOT appear.
```

### UI smoke test
- Log in as User A → Contracts page → only `ContractA` visible.
- Log out, log in as User B → Contracts page → only `ContractB` visible.

---

## Check 2 — Audit log is scoped per org

### SQL probe
```sql
-- POST an event to ContractA's ingest endpoint as User A (any API key)
-- Then confirm User B cannot see it via RLS:
set local role authenticated;
set local request.jwt.claims = '{"sub": "<USER_B_UUID>"}';

select count(*) from public.audit_log
where  contract_id = '<CONTRACT_A_UUID>';
-- Expected: 0 (User B's org_id != ContractA's org_id → RLS filters it out)
```

### UI smoke test
- Log in as User A → Audit Log → entries are visible.
- Log in as User B → Audit Log → User A's entries are NOT visible.

---

## Check 3 — API keys are scoped per org

### SQL probe
```sql
-- User B cannot see User A's keys
set local role authenticated;
set local request.jwt.claims = '{"sub": "<USER_B_UUID>"}';

select count(*) from public.api_keys
where  user_id = '<USER_A_UUID>';
-- Expected: 0
```

### RLS INSERT check
```sql
-- Attempting to create a key with User A's org_id while authenticated as User B
-- should fail (WITH CHECK on the insert policy)
set local role authenticated;
set local request.jwt.claims = '{"sub": "<USER_B_UUID>"}';

insert into public.api_keys (user_id, org_id, name, key_prefix, key_hash)
values ('<USER_B_UUID>', '<ORG_A_UUID>', 'evil-key', 'cg_live_XXX', 'fakehash');
-- Expected: RLS violation error
```

---

## Check 4 — Rust backend scopes to org from API key

### Setup
- Create an API key for User A via the dashboard → note the `cg_live_...` value.
- Create an API key for User B → note separately.

### Probe
```bash
# User A's key — should return only User A's contracts
curl -s -H "x-api-key: <USER_A_KEY>" http://localhost:3001/contracts \
  | jq '.[].name'
# Expected: ["ContractA"]

# User B's key — should return only User B's contracts
curl -s -H "x-api-key: <USER_B_KEY>" http://localhost:3001/contracts \
  | jq '.[].name'
# Expected: ["ContractB"]
```

---

## Check 5 — Auto-provisioning creates isolated orgs on signup

### SQL probe
```sql
-- One org per user, each unique slug
select u.email, o.slug, m.role
from   auth.users         u
join   public.org_memberships m on m.user_id = u.id
join   public.orgs            o on o.id      = m.org_id
order  by u.created_at;
-- Expected: each user has exactly one row, no two users share an org_id
```

---

## Check 6 — Invite flow creates membership in correct org

### Steps
1. Log in as User A → Account → Invite `c@test.com` as Member.
2. Confirm invite row exists:
```sql
select email, role, expires_at, revoked_at
from   public.org_invites
where  org_id = '<ORG_A_UUID>'
  and  accepted_at is null;
-- Expected: one row for c@test.com
```
3. (When `/auth/accept-invite` is built) User C accepts invite → membership row
   appears in User A's org, NOT in a new org:
```sql
select org_id, role from public.org_memberships where user_id = '<USER_C_UUID>';
-- Expected: org_id = ORG_A_UUID, role = 'member'
```

---

## Known gaps (defer to post-MVP)

| Gap | Impact | Mitigation |
|-----|--------|------------|
| `/auth/accept-invite` page not built | Invite tokens generated but can't be redeemed | Low — no external users yet |
| Member emails not shown in member list | Members appear as UUID in dashboard | Low — only personal orgs in use |
| `NEXT_PUBLIC_API_KEY` is a shared env-var key | All dashboard calls share one org scope | Fine until multi-org is needed |
| Rust API doesn't verify Supabase JWTs | Dashboard contract/audit calls use static key | Accepted — API keys carry org context |
