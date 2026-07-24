-- ContractGate — Migration 035: Bot signup cleanup
-- Run after 034_gated_payload_storage.sql
--
-- Removes 50 accounts identified by a manual audit (2026-07-22) as automated
-- signup-bot abuse, out of 57 total auth.users rows.
--
-- Audit method: every affected row has last_sign_in_at IS NULL (created an
-- account and never once logged in) AND a raw_user_meta_data.display_name
-- consisting of a single random-looking token with no spaces (e.g.
-- "MyOHfgDwwVJmHXxrHs") rather than a real name — the signature of a script
-- that auto-fills the signup form. All 50 accounts each own exactly one
-- solo-member org auto-created at signup (org name = the same random
-- display_name), with zero contracts, zero API keys, zero audit log entries,
-- and zero invites — confirmed by direct query before writing this
-- migration. The 7 real/team rows (the project owner's account, GitHub-OAuth
-- accounts, and nightmoose.com test accounts) all have a non-NULL
-- last_sign_in_at and are excluded automatically.
--
-- Deletion is scoped to this explicit (user_id, org_id) list, not a general
-- pattern, so the migration can't accidentally widen its blast radius if
-- re-run later against a database with new legitimate signups. Both the
-- org and the user deletes repeat the audit criteria as an independent
-- second check, and a DO block aborts the whole migration (no rows deleted)
-- if reality no longer matches what was audited.
--
-- auth.identities cascades automatically (identities_user_id_fkey ON DELETE
-- CASCADE). org_memberships and orgs have no FK to auth.users, so they're
-- deleted explicitly, in dependency order, before the auth.users delete.

BEGIN;

CREATE TEMP TABLE _bot_accounts (user_id uuid PRIMARY KEY, org_id uuid NOT NULL UNIQUE) ON COMMIT DROP;

INSERT INTO _bot_accounts (user_id, org_id) VALUES
    ('fd10928e-7d60-46c9-a8e5-d8b144fc56d2', '469d2471-1972-443c-87b6-449fa27dc6d6'),
    ('414d8d0b-7c28-465f-a55a-a4ed047bff5b', '91281c8a-e831-453a-80f0-24fa44b3d548'),
    ('c8538ea4-b667-42af-b276-4a5a8767b173', '3c35f3b7-8d85-4f59-a753-7507819a2e94'),
    ('245262e2-43a7-44d2-850c-b05f972dc3de', '2506309e-9f83-45a3-9519-fbbf8a242be4'),
    ('d08e4a14-1bca-4f1b-aad6-9d33d1f2c6db', '49a0594f-f66b-4a89-9e19-2c34be12d368'),
    ('ac77d8fa-bcac-45fa-b0bc-9696e2b09b9d', '44e27204-f566-46f3-9a54-ad7acf9494bf'),
    ('93b05854-e775-49d4-bfbb-e091a68e0760', 'cf265b31-a152-44b3-8cb1-e144e52af821'),
    ('63e294f4-c55c-4c70-9688-9ddd5c6cf91b', 'd8ebcbf7-703c-42d1-8a18-cdffbd26fd22'),
    ('d2e91b0b-eec4-4d00-8958-22d51e7e909c', '41d3ef46-bb46-4701-8415-e7e8e59af54e'),
    ('96b627c8-d73e-4e77-86f5-4071030e6e34', '93f04e86-5333-4974-a6bf-7fac86e70142'),
    ('9eaf3ee5-300e-449f-b190-04f693ef17b7', '61628e37-9ec3-4523-b317-c064ea339d23'),
    ('91f31854-53b3-48c3-9350-2be08af3286e', 'c532b208-1dcf-4e8a-84e1-da6cbaf61b1e'),
    ('44b53e32-9d58-4e03-864d-cec42d9d922a', 'c17d6e31-0b76-46a3-8ae1-db4ef9ba6965'),
    ('5546b621-358c-48d9-b79a-65c9a0579994', '95fe8941-2653-44da-b2ea-70216aeb2185'),
    ('9564f84d-f5dd-4653-8b04-796ef5de1650', 'c9ef44ad-ee1f-458c-95bb-d7d245de070a'),
    ('f83d6e62-4bea-43c7-a21a-70db96b2a57a', 'd2a6dd18-d1e1-41e7-9e16-19714c1b4445'),
    ('69712b75-13a1-47a5-823e-ebdf18a0063e', '7d647db0-7748-474b-aeb1-324d3530c852'),
    ('a52175d6-b6ef-4103-b6d0-f60147278998', 'efa97aa9-be78-407d-906e-75b0d08b5d10'),
    ('55d3063f-7b0d-4cae-a2f1-343a558af70f', '879f3dbf-803a-4e05-ba23-52ce338f291d'),
    ('f64b2d16-deee-4d8b-bdda-49bc7b482955', '24d9490e-6100-410c-bc2f-51680f12f9a4'),
    ('e095a6ef-8141-4a2e-bce2-cd62692f0f30', 'ceb2fb4c-6fbd-413c-bb4d-ab2ebb461ea9'),
    ('fae324ab-6cb9-4d53-ad4e-c9734094b94a', '85e611ba-bc4d-4f35-9a92-439e3acc7722'),
    ('8846f64a-d6de-4f85-b878-9b4654637288', '4e94aa9e-1182-4c4b-bc53-25b2b7c7430f'),
    ('4f902843-c05b-4f24-a44c-212dbb4f6206', 'ad6419f0-3df0-4ec4-bc6c-d79b17fe90e8'),
    ('2c7b8605-d2f1-43fa-8c64-a6fa949c0291', 'f6cbebf0-17a5-4f24-b0e7-a1dec29cc6d2'),
    ('618a7e50-96a0-42d6-bc87-03483c80dd9e', '82c75365-ccb3-4dfb-9477-1db736196143'),
    ('4ed83396-f764-4d14-8973-8fcfdc86e3e3', '63e1ce8d-3497-4eb5-b24a-ef41f945dbe8'),
    ('87c2e958-ac3f-49d7-bee1-4345cb35685b', 'fbecf39a-cd5c-4ce4-ad36-ef65774b6b68'),
    ('f47997f3-f5c3-4855-8a72-89d4302a5e48', '04ffb0ac-4392-4634-ad42-0b77610d1ad0'),
    ('788c398a-378a-42fd-9073-86c96bd7c009', 'b8695ab5-efe8-451f-b8c0-0c005bbcbf07'),
    ('0478f0a6-7696-4b02-9f93-7906ebdfb09f', '9b3796e5-147a-4e5e-8d0c-242b54d844f0'),
    ('4fccf3e7-cd73-47c1-b5ae-e18adbad8bb2', '9a5e088a-ba12-41b5-9680-fd4cee168cd1'),
    ('d834a208-bbde-4aa3-8dd4-44729c77cfe8', 'b16c2788-b099-4294-850e-45765db03d75'),
    ('a61c341e-9949-49cb-a302-3d970d5c0b99', '89252de7-72b6-42e8-b606-5e1a1ac642f4'),
    ('192b2cb9-6e66-4b4c-a055-1af18589658f', 'dd66f8d9-e6ed-4b3a-8417-4f717c1b26d5'),
    ('4d815601-8ec5-44fe-a784-cb3c05462d02', '750d754a-2d13-4132-b06b-d887151d7d8e'),
    ('b6929fb9-f298-4272-96a7-9b694f8aff16', '9e06ba4b-2fe9-4c81-a810-512f9fea1b6d'),
    ('a6a089f5-3d70-4a16-882b-e7411aa89845', '88e77fc8-abec-4f93-b970-e9223b7871f1'),
    ('f239bea9-68dc-4082-ab0a-b7ba9713732c', '969b1d4a-ffcb-4770-878a-d03408b10d83'),
    ('eebe42fb-9c2f-4bc3-b012-c6b5b635c748', 'feb981db-ad59-458d-9b45-d594c86a0d50'),
    ('312d60d1-04ce-48b6-a99b-374cc0b6bb2c', 'd02c995d-d6cc-4521-8d13-29c058ca4520'),
    ('db9d0891-a32b-43ca-9de8-45f728b17495', 'fc2788d8-350d-4cfd-8f03-69fa66c6024a'),
    ('7fdf14ee-8176-4e4f-bc2b-a62cc0816dad', '565d0232-1ad1-4160-b5c2-0be7f6cc2f6c'),
    ('71392033-8c29-476c-8c56-63355cec4967', 'babccf66-8d30-4f6e-a3cd-f46392004580'),
    ('359689eb-7c45-421c-84d0-c88c7fcbc3e3', '06bb63d2-de71-4cd0-bf3c-94936fbb943c'),
    ('de1bd730-d5ac-4543-9ece-0ff09c867db8', '1dc104e9-bc17-4055-ad2b-8cf1ec078b5e'),
    ('b65a68ba-5eee-425c-8e35-2ae4a9a2ea9d', 'ce7fd370-f398-4b64-aeab-58408a51b8e7'),
    ('e7bdf47b-981d-4104-9262-14ed2d170455', 'ca5037ea-67b0-418e-b77d-e711473deecb'),
    ('cd1fba8c-a733-4aa8-a19a-6f673d96d093', '0f3833c0-1873-4841-a882-7ca48794c742'),
    ('451bade0-50bc-4643-8255-803c2c675f33', '9d271725-48b9-4650-b003-51b9349af1c8');

-- Safety valve: abort the whole migration (no rows deleted anywhere) unless
-- every listed row still matches the bot signature, each listed org still has
-- exactly one member (the listed user) and no contracts / API keys / audit
-- log entries / pending invites.
DO $$
DECLARE
    listed_count integer;
    matching_users integer;
    clean_orgs integer;
BEGIN
    SELECT count(*) INTO listed_count FROM _bot_accounts;

    SELECT count(*) INTO matching_users
    FROM auth.users u
    JOIN _bot_accounts b ON b.user_id = u.id
    WHERE u.last_sign_in_at IS NULL
      AND u.created_at >= '2026-06-01'::timestamptz
      AND u.created_at <  '2026-07-24'::timestamptz
      AND u.email NOT LIKE '%@nightmoose.com'
      AND u.raw_user_meta_data ? 'display_name'
      AND u.raw_user_meta_data->>'display_name' !~ ' ';

    SELECT count(*) INTO clean_orgs
    FROM _bot_accounts b
    WHERE (SELECT count(*) FROM public.org_memberships om WHERE om.org_id = b.org_id) = 1
      AND EXISTS (
          SELECT 1 FROM public.org_memberships om
          WHERE om.org_id = b.org_id AND om.user_id = b.user_id
      )
      AND NOT EXISTS (SELECT 1 FROM public.contracts c WHERE c.org_id = b.org_id)
      AND NOT EXISTS (SELECT 1 FROM public.api_keys ak WHERE ak.org_id = b.org_id)
      AND NOT EXISTS (SELECT 1 FROM public.audit_log al WHERE al.org_id = b.org_id)
      AND NOT EXISTS (SELECT 1 FROM public.org_invites oi WHERE oi.org_id = b.org_id);

    IF listed_count <> 50 OR matching_users <> listed_count OR clean_orgs <> listed_count THEN
        RAISE EXCEPTION
            'Bot cleanup safety check failed (listed=%, matching_users=%, clean_orgs=%) — aborting, no rows deleted',
            listed_count, matching_users, clean_orgs;
    END IF;
END $$;

DELETE FROM public.org_memberships
WHERE org_id IN (SELECT org_id FROM _bot_accounts);

DELETE FROM public.orgs
WHERE id IN (SELECT org_id FROM _bot_accounts);

DELETE FROM auth.users
WHERE id IN (SELECT user_id FROM _bot_accounts)
  AND last_sign_in_at IS NULL
  AND created_at >= '2026-06-01'::timestamptz
  AND created_at <  '2026-07-24'::timestamptz
  AND email NOT LIKE '%@nightmoose.com'
  AND raw_user_meta_data ? 'display_name'
  AND raw_user_meta_data->>'display_name' !~ ' ';

COMMIT;
