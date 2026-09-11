/**
 * RFC-091 — dashboard help catalog.
 *
 * Components take ids, not copy. Hover uses `hover ?? what`.
 * The inspect card uses title / what / does / when / href.
 */

export interface HelpEntry {
  title: string;
  what: string;
  does: string;
  when?: string;
  href?: string;
  hover?: string;
}

export const HELP_CATALOG: Record<string, HelpEntry> = {
  // ── Nav ──────────────────────────────────────────────────────────────────
  "nav.stream-demo": {
    title: "Stream Demo",
    what: "A live race between the real validator and a no-op copy path — no Kafka, no database writes.",
    does: "Shows per-event pass/fail, latency, and overhead so you can see the engine work before wiring production traffic.",
  },
  "nav.kafka-connect": {
    title: "Kafka Connect",
    what: "Docs for the Kafka Connect Single Message Transform that validates records in-flight.",
    does: "Walks through installing the SMT, pointing it at a contract, and routing failures to a dead-letter topic.",
    href: "/docs/kafka-connect",
  },
  "nav.python-sdk": {
    title: "Python SDK",
    what: "First-party Python client for the gateway, plus a local validator for tests.",
    does: "Opens the SDK docs — sync/async HTTP ingest and offline contract checks with no network.",
    href: "/docs/python-sdk",
  },
  "nav.pricing": {
    title: "Pricing",
    what: "Plan comparison for Self-Hosted Free, Cloud Free, Growth, and Enterprise.",
    does: "Shows which dashboard features are gated and where to upgrade.",
    href: "/pricing",
  },
  "nav.dashboard": {
    title: "Dashboard",
    what: "Live ingestion health for this org — totals, pass rate, latency, and recent events.",
    does: "Refreshes every few seconds so you can see whether traffic is clean before opening Audit or Quarantine.",
  },
  "nav.contracts": {
    title: "Contracts",
    what: "The semantic schemas ContractGate enforces on inbound (and, on Growth, outbound) events.",
    does: "Create, version, promote, and inspect contracts. Failures against a stable version land in Quarantine.",
    when: "Start here if you have no contracts yet.",
  },
  "nav.catalog": {
    title: "Catalog",
    what: "Public and community contracts you can fork or import, plus the egress validator.",
    does: "Browse open-data contracts, import a publication ref, or test an outbound payload against a contract you already own.",
  },
  "nav.scorecard": {
    title: "Scorecard",
    what: "Per-provider data-quality report: pass/quarantine rates, field health, and drift.",
    does: "Turns “your data is bad” into a shareable, numbered report keyed by the contract’s deploy-time source name.",
  },
  "nav.audit": {
    title: "Audit Log",
    what: "Every validation attempt — pass and fail — recorded at ingestion time.",
    does: "Lets you filter, inspect the stored (already-transformed) payload, and export the current view as CSV.",
  },
  "nav.scaffold": {
    title: "Scaffold",
    what: "Brownfield generator: JSON, NDJSON, Avro, or Protobuf in; draft contract YAML out.",
    does: "Infers types and flags PII candidates as TODOs — it never auto-applies a mask.",
  },
  "nav.workbench": {
    title: "Workbench",
    what: "Browser-local API explorer that infers a contract from live responses.",
    does: "Seed from a URL, spec, or curl; probe endpoints; refine fields; deploy. Credentials never leave the browser.",
  },
  "nav.playground": {
    title: "Playground",
    what: "A dry-run validator: paste YAML and a sample event, no ingest, no storage.",
    does: "Shows which rules pass or fail before you promote a version or send production traffic.",
  },
  "nav.account": {
    title: "Account",
    what: "Org settings: API keys, team, billing, GitHub sync, and payload-storage toggle.",
    does: "Issue keys for /v1/ingest, invite members, and manage the plan.",
  },
  "help.whats-this": {
    title: "What’s this?",
    what: "Inspect mode for this dashboard. Toggle it, then click a highlighted control.",
    does: "Opens a short card — what the control is, what it does, and a docs link when one exists. Esc exits. Mode does not survive a reload.",
    hover: "Toggle inspect mode — click a highlighted control for a description.",
  },

  // ── Pages ────────────────────────────────────────────────────────────────
  "page.dashboard": {
    title: "Live Monitor",
    what: "Org-wide ingestion health, refreshed every 5 seconds.",
    does: "Summarizes event volume, pass rate, violations, and latency, and links through to Audit and Contracts.",
  },
  "page.contracts": {
    title: "Contracts",
    what: "Versioned semantic contracts for this org.",
    does: "Create from YAML, a sample, CSV, or the visual builder; promote a draft to stable so ingest can route to it.",
  },
  "page.catalog": {
    title: "Contract Catalog",
    what: "Discovery surface for open-data and community-published contracts, plus egress checks.",
    does: "Fork or import a contract into your org, or validate an outbound payload before it leaves your API.",
  },
  "page.scorecard": {
    title: "Provider Scorecard",
    what: "Objective quality evidence for a named upstream source.",
    does: "Loads pass/quarantine totals, per-field health, and active drift signals for the source string you type.",
  },
  "page.audit": {
    title: "Audit Log",
    what: "Append-only record of every ingest decision.",
    does: "Filter by contract and pass/fail, open the stored payload, export CSV. Values here have already been through PII transforms.",
  },
  "page.scaffold": {
    title: "Contract Scaffolder",
    what: "One-shot generator from existing payloads or schemas.",
    does: "Produces draft YAML with profiler stats and PII TODO comments you review before saving.",
  },
  "page.workbench": {
    title: "API Workbench",
    what: "In-browser contract authoring against a live API.",
    does: "Discovers endpoints, infers fields from responses, and can deploy the result as a stable contract on Growth.",
  },
  "page.playground": {
    title: "Playground",
    what: "Side-by-side YAML and sample event, validated in-place.",
    does: "Calls the same engine as ingest with dry-run semantics — nothing is stored or forwarded.",
  },
  "page.account": {
    title: "Account",
    what: "This user’s org, keys, and billing.",
    does: "Create API keys (shown once), manage members, and open the Stripe portal on paid plans.",
  },
  "page.stream-demo": {
    title: "Stream Demo",
    what: "Public live demo of the validation engine versus a straight-copy baseline.",
    does: "Starts a timed scenario and streams pass/fail and latency so you can judge overhead without an account.",
  },
  "page.docs": {
    title: "Docs",
    what: "Integration starting points — agent playbook, MCP server, Python SDK, Kafka Connect.",
    does: "Opens the human and agent-facing guides. The playbook is the paste-into-Claude path.",
  },
  "page.pricing": {
    title: "Pricing",
    what: "Feature matrix across Self-Hosted Free, Cloud Free, Growth, and Enterprise.",
    does: "Shows what each plan includes. Growth unlocks quarantine replay, visual builder, scorecard, and workbench save/deploy.",
  },

  // ── Dashboard ────────────────────────────────────────────────────────────
  "dashboard.total-events": {
    title: "Total Events",
    what: "Count of ingest attempts this org has recorded.",
    does: "Includes both passes and failures. Empty means nothing has been POSTed to /v1/ingest yet.",
  },
  "dashboard.pass-rate": {
    title: "Pass Rate",
    what: "Share of ingest attempts that satisfied the contract.",
    does: "Drops when events fail validation and are quarantined or rejected. High 90s is healthy for a stable contract.",
  },
  "dashboard.violations": {
    title: "Violations",
    what: "Events that failed at least one contract rule.",
    does: "Each one has a row in Audit (FAIL) and, when quarantine is on, a replayable row in Quarantine.",
  },
  "dashboard.avg-latency": {
    title: "Avg Latency",
    what: "Mean server-side validation time per event, in microseconds.",
    does: "The engine budget is well under 15 ms p99 end-to-end. This card is the validator itself, not network time.",
  },
  "dashboard.public-contracts": {
    title: "Public Contracts",
    what: "A short list of open-data and community-published contracts.",
    does: "Click through to Catalog to fork or import one instead of starting from a blank YAML.",
  },

  // ── Contracts ────────────────────────────────────────────────────────────
  "contracts.list": {
    title: "My Contracts",
    what: "Contracts this org authored or deployed.",
    does: "Open one to edit YAML, manage versions, or inspect quarantine for that contract.",
  },
  "contracts.consumed": {
    title: "Consumed",
    what: "Contracts you imported from a provider publication ref.",
    does: "Snapshot imports stay frozen; subscribe-mode imports badge when the provider publishes a newer version.",
  },
  "contracts.visual-builder": {
    title: "Visual Builder",
    what: "Form-based contract editor — fields, types, constraints, without writing YAML by hand.",
    does: "Builds the same YAML the engine validates. Growth plan.",
  },
  "contracts.generate": {
    title: "Generate from Sample",
    what: "Infer a draft contract from one or more JSON sample events.",
    does: "Sends the samples to the Rust inference engine and returns editable YAML. Growth plan.",
  },
  "contracts.csv": {
    title: "From CSV",
    what: "Infer a draft contract from a CSV file of sample rows.",
    does: "Same inference engine as JSON samples, for tabular dumps. Growth plan.",
  },
  "contracts.quarantine": {
    title: "Quarantine",
    what: "Events that failed validation and were held instead of dropped.",
    does: "Filter, inspect the payload, and replay against a current version. Growth plan.",
    href: "https://github.com/nightmoose/contractgate/blob/main/docs/quarantine-replay-reference.md",
  },
  "contracts.new": {
    title: "New Contract",
    what: "Start a contract from a catalog fork, CSV, API, or a blank YAML.",
    does: "Opens the source-first wizard. You still need to promote a version to stable before ingest will accept unpinned traffic.",
  },
  "contracts.import-odcs": {
    title: "Import ODCS",
    what: "Open Data Contract Standard YAML, mapped onto ContractGate’s format.",
    does: "Creates a draft you can edit and promote. Useful when a producer already publishes ODCS.",
  },
  "contracts.import-ref": {
    title: "Import from Ref",
    what: "Pull a contract another org published, by its publication ref.",
    does: "Snapshot copies it once; subscribe (Growth) follows new published versions.",
  },

  // ── RFC-020 jargon ───────────────────────────────────────────────────────
  "term.ontology": {
    title: "Ontology",
    what: "The named entities and field rules your contract enforces — every inbound event is validated against these definitions.",
    does: "Types, required flags, enums, patterns, and ranges all live here. Undeclared fields are allowed unless compliance mode is on.",
  },
  "term.glossary": {
    title: "Glossary",
    what: "Human-readable descriptions of fields, including any compliance constraints attached to each one.",
    does: "Does not change validation by itself — it documents why a field exists and what “good” means to the humans.",
  },
  "term.metrics": {
    title: "Metrics",
    what: "Named aggregate formulas (e.g. sum, count) computed over events that pass this contract.",
    does: "Declares the business calculations you expect to be possible once the data is clean.",
  },
  "term.stable": {
    title: "Stable",
    what: "A frozen, immutable version eligible to receive inbound traffic. YAML cannot be edited after promotion.",
    does: "Unpinned ingest routes here. Promote a draft when you are ready for production traffic.",
  },
  "term.draft": {
    title: "Draft",
    what: "A work-in-progress version. YAML is freely editable. Promotes to Stable when ready.",
    does: "Use the playground or a pinned ingest to test a draft without moving live traffic.",
  },
  "term.deprecated": {
    title: "Deprecated",
    what: "A retired version. No new unpinned traffic routes to it.",
    does: "Clients that explicitly pin this version get their batch quarantined rather than silently accepted.",
  },
  "term.quarantine": {
    title: "Quarantine",
    what: "Events that failed contract validation are held here for inspection and optional replay. Nothing is silently dropped.",
    does: "Open a row to see the payload and violations; replay it against a fixed version when ready.",
    href: "https://github.com/nightmoose/contractgate/blob/main/docs/quarantine-replay-reference.md",
  },
  "term.replay": {
    title: "Replay",
    what: "Re-validate a quarantined event against a current contract version.",
    does: "If it passes, it is written to the audit log and forwarded downstream. The original quarantine row is kept.",
    href: "https://github.com/nightmoose/contractgate/blob/main/docs/quarantine-replay-reference.md",
  },
  "term.retention": {
    title: "Retention",
    what: "How long quarantined events are kept before being purged.",
    does: "Once purged, replay is no longer possible — the payload is gone.",
  },
  "term.mask": {
    title: "mask",
    what: "Replaces the field value with a fixed placeholder (e.g. ****). The original value is never stored.",
    does: "Runs after validation, before the event is written to the audit log or forwarded.",
    href: "https://github.com/nightmoose/contractgate/blob/main/docs/pii-masking-reference.md",
  },
  "term.hash": {
    title: "hash",
    what: "Replaces the field value with a deterministic HMAC-SHA256 digest using the contract’s per-contract salt.",
    does: "Same input always hashes the same, so joins still work without storing the raw PII.",
    href: "https://github.com/nightmoose/contractgate/blob/main/docs/pii-masking-reference.md",
  },
  "term.drop": {
    title: "drop",
    what: "Removes the field from the stored event entirely — as if it was never sent.",
    does: "Use when a field must not land in audit or downstream at all.",
    href: "https://github.com/nightmoose/contractgate/blob/main/docs/pii-masking-reference.md",
  },
  "term.redact": {
    title: "redact",
    what: "Replaces the field value with the literal string [REDACTED].",
    does: "Keeps the key in the payload so schema shape is stable, without keeping the secret.",
    href: "https://github.com/nightmoose/contractgate/blob/main/docs/pii-masking-reference.md",
  },
  "term.format_preserving": {
    title: "format_preserving",
    what: "Masks the value while preserving its structure (e.g. a credit card stays 16 digits, just with most digits replaced).",
    does: "Useful when downstream systems validate format but must not see the real value.",
    href: "https://github.com/nightmoose/contractgate/blob/main/docs/pii-masking-reference.md",
  },
  "term.salt": {
    title: "Salt",
    what: "A 32-byte secret tied to this contract used when hashing PII fields.",
    does: "Changing it invalidates all prior hashes — treat it like a key rotation.",
    href: "https://github.com/nightmoose/contractgate/blob/main/docs/pii-masking-reference.md",
  },
  "term.compliance-mode": {
    title: "Compliance mode",
    what: "When enabled, any inbound field not declared in the contract ontology is rejected.",
    does: "Nothing undeclared can enter the audit log. Turn it on when the contract is the allow-list.",
  },
  "term.leakage": {
    title: "Leakage mode",
    what: "Controls how undeclared fields in outbound payloads are handled.",
    does: "off = pass through; strip = remove silently; fail = treat as a validation error.",
  },
  "term.pass": {
    title: "PASS",
    what: "This event satisfied every rule in the contract and was written to the audit log.",
    does: "Clean events are eligible to be forwarded downstream.",
  },
  "term.fail": {
    title: "FAIL",
    what: "This event violated at least one contract rule and was quarantined or rejected.",
    does: "Open the row for field-level violations, then fix the producer or the contract and replay if needed.",
  },
  "term.fallback": {
    title: "fallback",
    what: "On unpinned traffic, if the latest stable version rejects an event, the gateway tries other stable versions in order until one accepts.",
    does: "Use when you are mid-migration and two stables must coexist. Strict is the default.",
  },
  "term.strict": {
    title: "strict",
    what: "Unpinned traffic validates against only the single latest stable version. No retry on failure. This is the default.",
    does: "Pick this unless you have a deliberate multi-stable rollout.",
  },

  // ── Catalog ──────────────────────────────────────────────────────────────
  "catalog.opendata": {
    title: "Open Data Contracts",
    what: "Curated contracts for public data sources, maintained by ContractGate.",
    does: "Fork one into your org to customise it — the original stays intact.",
  },
  "catalog.published": {
    title: "Community Published",
    what: "Contracts other organisations have published by ref.",
    does: "Browse the public list or paste a ref (and token, if link-gated) to import.",
  },
  "catalog.egress": {
    title: "Egress Validator",
    what: "The same engine as ingest, pointed at an outbound payload.",
    does: "Checks that what you are about to send still matches the contract — including leakage rules. Growth plan.",
  },
  "catalog.subscribe": {
    title: "Subscribe import",
    what: "An import that follows the provider’s published versions.",
    does: "Your copy badges when a newer version is out so you can pull the update. Growth plan.",
  },

  // ── Scorecard / Scaffold / Workbench / Playground / Audit / Account ─────
  "scorecard.source": {
    title: "Provider Source",
    what: "The source string set on a contract at deploy time (a vendor name, feed id, …).",
    does: "Scorecard rolls up quality for every contract tagged with that source.",
  },
  "scorecard.drift": {
    title: "Drift Signals",
    what: "Fields whose null or violation rate moved vs a recent baseline.",
    does: "A rising delta is an early warning that a producer changed shape or quality.",
  },
  "scaffold.pii-candidate": {
    title: "PII candidate",
    what: "A field the scaffolder thinks might be personal data, based on name and shape.",
    does: "Emits a TODO in the YAML. You choose mask / hash / drop / redact — nothing is applied automatically.",
  },
  "workbench.seed": {
    title: "Seed",
    what: "How Workbench discovers endpoints: URL, OpenAPI spec, curl, Postman, Bruno, or manual paths.",
    does: "All probing happens in your browser. ContractGate never sees the credentials or response bodies.",
  },
  "workbench.infer": {
    title: "Infer",
    what: "Field types, required flags, and PII hints derived from observed JSON responses.",
    does: "Confidence scores tell you what to review before deploying.",
  },
  "workbench.deploy": {
    title: "Deploy",
    what: "Promote the inferred YAML to a stable contract via POST /contracts/deploy.",
    does: "Growth+. Free tier can try one endpoint but cannot save or deploy.",
  },
  "playground.validate": {
    title: "Validate",
    what: "Dry-run the YAML in the editor against the sample JSON.",
    does: "Shows per-field pass/fail and any PII transform preview. Nothing is ingested.",
  },
  "audit.stored-payload": {
    title: "Stored payload",
    what: "The bytes written to audit_log after PII transforms ran — not the original request body.",
    does: "Masked, hashed, dropped, or redacted fields are already scrubbed here.",
  },
  "account.api-keys": {
    title: "API keys",
    what: "Bearer credentials for /v1/ingest and the CLI. The raw key is shown once at creation.",
    does: "Send it as X-Api-Key. Revoke from this page if it leaks. Per-key contract allow-lists still apply.",
  },
  "account.team": {
    title: "Team",
    what: "Org members and pending invites. Roles are owner, admin, and member.",
    does: "Owners and admins invite, change roles, and revoke. Members see a read-only roster.",
  },
  "account.billing": {
    title: "Billing",
    what: "The org’s current plan and Stripe status.",
    does: "Free orgs upgrade via Pricing; paid orgs open the Stripe customer portal.",
  },
};

export function getHelp(id: string): HelpEntry | undefined {
  return HELP_CATALOG[id];
}
