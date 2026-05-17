# RFC-041: Correct API key hash algorithm documentation

**Status:** Accepted  
**Branch:** `dev/p02-rls-contract-versions`  
**Fixes:** P0-3 from REVIEW-2026-05-16-saas-readiness.md

## Problem

`supabase/migrations/006_accounts_and_api_keys.sql` has a `COMMENT ON COLUMN`
claiming `key_hash` is "bcrypt (cost 10)".  `dashboard/app/account/page.tsx`
has a comment saying "the API route handles bcrypt hashing."  Neither is true:

- The browser computes `SHA-256(raw_key)` then base64-encodes it via
  `SubtleCrypto`.
- `src/api_key_auth.rs` verifies by computing the same SHA-256/base64 and
  comparing strings.
- There is no bcrypt step anywhere, and no "API route" involved in hashing.

A future engineer reading the migration or the dashboard code will believe the
column contains bcrypt hashes and break key verification when they touch either
side.

## Decision

**Keep SHA-256.**  The raw key is 56 characters of cryptographically random
hex (`cg_live_<28-byte hex>`), giving ~224 bits of entropy.  SHA-256 of a
high-entropy key provides equivalent security to bcrypt for this use case — the
cost function adds nothing when the input space is already unguessable.

Changes:
1. New migration (`024`) updates the `COMMENT ON COLUMN` to document SHA-256.
2. Add a `CHECK` constraint on `key_hash` length (44 chars = base64(32 bytes))
   to catch accidental algorithm changes at the DB layer.
3. Fix the misleading comment in `dashboard/app/account/page.tsx`.
4. No changes to `src/api_key_auth.rs` — implementation is already correct.

## Files changed

- `supabase/migrations/024_api_key_hash_algorithm_docs.sql`
- `dashboard/app/account/page.tsx` (comment fix only)
