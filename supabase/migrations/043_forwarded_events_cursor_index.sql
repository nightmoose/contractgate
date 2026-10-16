-- Migration 043: keyset index for GET /v1/contracts/{id}/passed.
--
-- The read is `WHERE contract_id = $1 AND created_at <= now() - 5s
-- AND (created_at, id) > ($cursor_ts, $cursor_id) ORDER BY created_at, id`.
-- Ids are random UUIDs, so (created_at, id) is the order, not id alone.
--
-- CREATE INDEX CONCURRENTLY cannot run inside a transaction. This file has
-- no BEGIN/COMMIT. CI applies it with `psql -f` (no --single-transaction),
-- which is allowed. A runner that wraps each file in a transaction must
-- apply this statement on its own.
--
-- Do not apply to production until Alex approves. If a previous attempt
-- left an INVALID index of this name, drop that index before re-running;
-- IF NOT EXISTS will not rebuild an invalid one.

CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_forwarded_events_contract_created_id
    ON forwarded_events (contract_id, created_at, id);
