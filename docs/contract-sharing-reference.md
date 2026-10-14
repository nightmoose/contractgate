# Contract Sharing & Publication Reference

**RFC:** 032  
**Status:** Accepted

---

## Overview

RFC-032 adds a **publish → import** flow that lets a provider share a contract
version by reference and a consumer import it directly — no manual
reconstruction, no spec drift.

A published contract is **immutable**.  The publication ref is stable; revocation
is a soft-delete that consumers can detect via `import-status`.

---

## Endpoints

### Publish a contract version

```
POST /contracts/{contract_id}/versions/{version}/publish
```

Marks a specific contract version as published.  Returns a stable
**publication ref** and, for `link` visibility, an unguessable
**link token**.

**Request body**

```json
{ "visibility": "link" }
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `visibility` | `"public"` \| `"link"` \| `"org"` | `"link"` | Who can fetch this publication |

**Visibility semantics**

| Value | Who can fetch |
|-------|---------------|
| `public` | Anyone with the publication ref |
| `link` | Anyone with the ref **and** the link token |
| `org` | Org-granted access (RFC-033, not yet enforced) |

**Response (201 Created)**

```json
{
  "publication_ref": "a3f1b2c4d5e6f708",
  "visibility": "link",
  "link_token": "9f8e7d6c5b4a3210fedcba9876543210",
  "contract_name": "user_events",
  "contract_version": "2.1.0",
  "published_at": "2026-05-15T00:00:00Z"
}
```

> `link_token` is only present when `visibility = "link"`.  It is shown
> **once** — store it securely before closing the response.

---

### Revoke a publication

```
DELETE /contracts/publications/{publication_ref}
```

Soft-deletes the publication.  Consumers who already imported the contract
keep their copy; `import-status` will surface `source_revoked: true`.

**Response (200 OK)**

```json
{
  "publication_ref": "a3f1b2c4d5e6f708",
  "revoked_at": "2026-05-15T12:00:00Z"
}
```

---

### Fetch a published contract

```
GET /published/{publication_ref}
GET /published/{publication_ref}?token={link_token}
```

No authentication header required (the publication ref / token are the
access control).

**Query parameters**

| Parameter | Required | Description |
|-----------|----------|-------------|
| `token` | When `visibility = "link"` | The link token returned at publish time |

**Response (200 OK)**

```json
{
  "publication_ref": "a3f1b2c4d5e6f708",
  "contract_name": "user_events",
  "contract_version": "2.1.0",
  "visibility": "link",
  "published_at": "2026-05-15T00:00:00Z",
  "yaml_content": "version: \"1.0\"\nname: user_events\n..."
}
```

Revoked publications return **404**.

---

### Import a published contract

```
POST /contracts/import-published
```

Imports a published contract into the caller's org.  Creates a new contract
identity + a draft version, with provenance recorded on the `contracts` row.

**Request body**

```json
{
  "publication_ref": "a3f1b2c4d5e6f708",
  "link_token": "9f8e7d6c5b4a3210fedcba9876543210",
  "import_mode": "snapshot"
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `publication_ref` | `string` | — | Ref returned by the publish endpoint |
| `link_token` | `string` | — | Required when visibility = `link` |
| `import_mode` | `"snapshot"` \| `"subscribe"` | `"snapshot"` | How to track future versions |

**Import modes**

| Mode | Behavior |
|------|----------|
| `snapshot` | One-time copy.  Provenance recorded, never auto-updates. |
| `subscribe` | Copy + live link.  `import-status` surfaces update-available signals. |

**Response (201 Created)**

```json
{
  "contract_id": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
  "version": "2.1.0",
  "import_mode": "snapshot",
  "imported_from_ref": "a3f1b2c4d5e6f708"
}
```

The new contract's `import_source` will be `"publication"` and the
provenance columns (`imported_from_ref`, `import_mode`, `imported_at`)
on `contracts` will be populated.

---

### Check import status

```
GET /contracts/{contract_id}/import-status
```

For `subscribe`-mode imports: reports whether the source publication has a
newer version available.  **Never auto-applies** — the consumer always
pulls explicitly.

**Response (200 OK)**

```json
{
  "import_mode": "subscribe",
  "publication_ref": "a3f1b2c4d5e6f708",
  "source_revoked": false,
  "update_available": true,
  "latest_published_version": "2.2.0",
  "imported_version": "2.1.0"
}
```

| Field | Description |
|-------|-------------|
| `import_mode` | `"snapshot"` \| `"subscribe"` \| `null` (native contract) |
| `publication_ref` | The ref this contract was imported from, or `null` |
| `source_revoked` | `true` if the source publication has been revoked |
| `update_available` | `true` when `latest_published_version ≠ imported_version` |
| `latest_published_version` | Current version string on the source publication |
| `imported_version` | Version string in the consumer's draft |

When `import_mode` is `null`, the contract was not imported from a publication
and all other fields are `null` / `false`.

---

## Database schema

### `contract_publications`

| Column | Type | Description |
|--------|------|-------------|
| `ref` | `text` PK | 24-hex opaque stable id |
| `contract_id` | `uuid` FK | Source contract |
| `version_id` | `uuid` FK | Pinned source version |
| `contract_name` | `text` | Denormalised for fast fetch |
| `contract_version` | `text` | Denormalised version string |
| `yaml_content` | `text` | Locked YAML of the published version |
| `visibility` | `text` | `public` \| `link` \| `org` |
| `link_token` | `text` | Non-null when `visibility = link` |
| `org_id` | `uuid` | Publishing org |
| `published_by` | `text` | Human-readable label |
| `published_at` | `timestamptz` | |
| `revoked_at` | `timestamptz` | NULL = active |

### Provenance columns on `contracts`

| Column | Type | Description |
|--------|------|-------------|
| `imported_from_ref` | `text` | `contract_publications.ref` |
| `import_mode` | `text` | `snapshot` \| `subscribe` |
| `imported_at` | `timestamptz` | When import ran |

---

## Dashboard

The **Publish** button appears in the contract modal footer alongside the
GitHub Sync button (for stable and draft versions).  Clicking it opens a
visibility selector and displays the publication ref + link token (shown
once) on success.

Contracts imported in `subscribe` mode show an **↑ Update available** badge
in the contracts list when `import-status` reports `update_available: true`.

---

## TypeScript SDK

```typescript
import {
  publishVersion,
  revokePublication,
  fetchPublished,
  importPublished,
  getImportStatus,
} from "@/lib/api";

// Publish
const pub = await publishVersion(contractId, "2.1.0", { visibility: "link" });
// pub.publication_ref, pub.link_token

// Import (snapshot)
const imported = await importPublished({
  publication_ref: pub.publication_ref,
  link_token: pub.link_token ?? undefined,
  import_mode: "snapshot",
});

// Check for updates
const status = await getImportStatus(imported.contract_id);
if (status.update_available) {
  console.log(`v${status.latest_published_version} available`);
}

// Revoke
await revokePublication(pub.publication_ref);
```

---

## Out of scope (this RFC)

- **Joint editing / comments** between provider and consumer — RFC-033.
- **Auto-pull of subscribed updates** — never silent; always consumer-initiated.
- **Browsable public contract catalog** — `public` visibility seeds it; catalog UI is a later RFC.
- **Cross-instance federation** — the `ref` design does not preclude it.


## Import, link tokens, unpublish (2026-10-14)

- **Importing a published stable version** creates a **stable** version with the
  same number (not a draft). The 201 body includes `state: "stable"` and a
  `message` saying it validates events right away (S-I3). Non-stable published
  versions still import as draft.
- **Link / org publications** need the link token. Preview uses `?token=`;
  import uses `"link_token"`. Missing or wrong tokens return **403** with
  `link token missing` / `link token invalid` — never the API-key 401 (S-P2).
- **Unpublish** (soft delete, sets `revoked_at`): `DELETE /contracts/publications/{ref}`,
  owner org only. Other org → 403; unknown/already revoked → 404. **A used
  publication can't be unpublished → 409** ("this publication has been
  imported; publish a new version instead"). "Used" = at least one import
  (`contracts.imported_from_ref = ref`, soft-deleted contracts included).
  Previews are not recorded today, so they don't count yet.
- **Revoked refs are 404 everywhere**: preview (`GET /published/{ref}`) and
  import (`POST /contracts/import-published`) both return
  `404 publication '<ref>' not found` (import used to return 400; R3).
- `POST /contracts/{id}/versions/{version}/publish` and the collaborator,
  comment and proposal routes are in `/openapi.json` (O2).
- `GET /contracts/publications` lists the caller org's live publications with
  `import_count`. Dashboard: Publish dialog → "Published versions of this
  contract" shows **Unpublish** (with confirm) only when `import_count = 0`;
  otherwise it shows why and says to promote a new version and publish it.
- **Catalog** lists `publication_ref` (not `published_by`) (S-C1).
- **Org ID** is on Account (header + Organization ID card) for collaboration
  invites (S-A1).
