import type { ReactNode } from "react";
import Link from "next/link";

function H2({ id, children }: { id: string; children: ReactNode }) {
  return (
    <h2 id={id} className="text-xl font-semibold text-slate-100 mt-12 mb-3 scroll-mt-8">
      {children}
    </h2>
  );
}

export default function FivetranDocsPage() {
  return (
    <div className="max-w-3xl py-10">
      <p className="text-sm text-slate-500 mb-3">
        <Link href="/docs" className="hover:text-green-400">
          Docs
        </Link>
        <span className="mx-2">/</span>
        <span className="text-slate-300">Fivetran</span>
      </p>
      <h1 className="text-3xl font-bold text-slate-100 mb-3">Use ContractGate with Fivetran</h1>
      <p className="text-slate-400 leading-relaxed mb-6">
        For DataContractGate and AISchemaGate users who want events that passed
        their contract in their own warehouse. ContractGate is both products
        sharing one engine. The connector reads only those passing records and
        writes one table per contract.
      </p>

      <H2 id="plans">Paid plan</H2>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Connect Fivetran and the passed-event export are paid-plan features.
        Plans <code className="text-slate-200">free</code> and{" "}
        <code className="text-slate-200">aisg_free</code> receive HTTP 403.
        Paid plans are <code className="text-slate-200">growth</code>,{" "}
        <code className="text-slate-200">aisg_pro</code>, and{" "}
        <code className="text-slate-200">enterprise</code> (Pro and Enterprise
        in the dashboard).
      </p>
      <p className="text-slate-400 text-sm leading-relaxed">
        On a free plan the form says “Connect Fivetran is a paid-plan feature”
        and links to Upgrade. The API error names the same three plan ids.
      </p>

      <H2 id="fivetran-account">Fivetran API key and destination group</H2>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Create an API key in your Fivetran account. Fivetran gives you a key
        and a secret. The Connect Fivetran form has a field for each:{" "}
        <span className="text-slate-200">Fivetran API key</span> and{" "}
        <span className="text-slate-200">Fivetran API secret</span>. Paste them
        as Fivetran showed them. Do not Base64-encode them for the button.
        ContractGate uses the pair once to upload the connector and create the
        connection, then drops them. They are not stored and are redacted from
        logs.
      </p>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        <span className="text-slate-200">Destination group id</span> is the id
        of the Fivetran destination (the warehouse group) the tables should
        land in. Create a destination in Fivetran first if you do not have one.
        List groups with the same key and secret; the <code className="text-slate-200">id</code>{" "}
        is what the form wants. The <code className="text-slate-200">name</code>{" "}
        is what the manual <code className="text-slate-200">fivetran deploy --destination</code>{" "}
        flag wants.
      </p>
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed mb-4">
{`curl -sS -u "$FIVETRAN_KEY:$FIVETRAN_SECRET" \\
  https://api.fivetran.com/v1/groups`}
      </pre>
      <p className="text-slate-400 text-sm leading-relaxed">
        An unknown group returns 400: “Fivetran group &lt;id&gt; was not found”.
        No ContractGate key is minted in that case.
      </p>

      <H2 id="connect">Connect Fivetran</H2>
      <ol className="list-decimal pl-5 text-slate-300 text-sm space-y-2 mb-4">
        <li>Open Contracts.</li>
        <li>Click Connect Fivetran.</li>
        <li>
          Paste the Fivetran API key, the Fivetran API secret, and the
          Destination group id.
        </li>
        <li>
          Under Contracts to sync, check each contract to include. Up to 32.
          Each checked row has an optional table name. Leave it blank to use
          the default name below.
        </li>
        <li>
          Click Deploy connector. The button reads Connecting… while the
          request runs.
        </li>
        <li>
          The connection is created paused. The panel shows the connection id,
          the status, and Open in Fivetran. Start the first sync from Fivetran
          after you review it.
        </li>
      </ol>
      <p className="text-slate-400 text-sm leading-relaxed">
        ContractGate mints a new API key named Fivetran connector, scoped to
        your org and the selected contracts, and sends it to Fivetran as a
        secret. Your dashboard key is not reused. The raw key is not shown in
        the browser. The same call is{" "}
        <code className="text-slate-200">POST /v1/integrations/fivetran/connect</code>.
        The scheduled sync is every 6 hours once you unpause it.
      </p>

      <H2 id="schema">Destination schema</H2>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        The Connect Fivetran form does not ask for a schema. When the request
        omits <code className="text-slate-200">destination_schema</code>,
        ContractGate picks a new schema named{" "}
        <code className="text-slate-200">contractgate_</code> plus 8 hex digits,
        for example <code className="text-slate-200">contractgate_a1b2c3d4</code>.
        Fivetran creates that schema in the destination on setup. The warehouse
        user on the destination must be allowed to create schemas, or that
        setup fails.
      </p>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        API callers can set <code className="text-slate-200">destination_schema</code>{" "}
        to a schema that already exists. Use that when the destination user
        cannot create schemas. The value must start with a lowercase letter and
        then use only lowercase letters, digits, and underscores, at most 63
        characters. Anything else is HTTP 400. Fivetran still needs permission
        to create tables in that schema.
      </p>
      <p className="text-slate-400 text-sm leading-relaxed">
        Names are stored with Fivetran <code className="text-slate-200">SOURCE_NAMING</code>,
        so the schema and table names above are not rewritten.{" "}
        <code className="text-slate-200">FIVETRAN_NAMING</code> would split
        letter and digit boundaries (<code className="text-slate-200">user_events_427c3304</code>{" "}
        would become <code className="text-slate-200">user_events_427_c_3304</code>).
      </p>

      <H2 id="warehouse">What lands in the warehouse</H2>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        One table per contract. Connect Fivetran names it{" "}
        <code className="text-slate-200">&lt;slug&gt;_&lt;8 hex&gt;</code>. The
        slug is the contract name in lowercase, with other characters turned
        into underscores, and leading digits dropped. Contract User Events and
        id <code className="text-slate-200">427c3304-ead1-4ba7-b7e0-123456789abc</code>{" "}
        becomes <code className="text-slate-200">user_events_427c3304</code>. A
        contract name with no letters becomes{" "}
        <code className="text-slate-200">passed_427c3304</code>. An optional
        table name on the form replaces that default for that contract. Names
        must be unique across the contracts you selected.
      </p>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        The primary key is <code className="text-slate-200">id</code>. Columns:
      </p>
      <ul className="list-disc pl-5 text-slate-300 text-sm space-y-1 mb-4">
        <li><code className="text-slate-200">id</code> — event id (string)</li>
        <li><code className="text-slate-200">contract_version</code> — string</li>
        <li><code className="text-slate-200">created_at</code> — UTC timestamp</li>
        <li><code className="text-slate-200">payload</code> — JSON</li>
      </ul>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        <code className="text-slate-200">payload</code> is null unless event
        payload storage is on. Storage is on only for a paid plan when Event
        Payload Storage on Account is on and Payload storage for that contract
        (on the Quarantine tab) is on. A null payload is still a passed event;
        the body was not stored. Kafka and Kinesis consumers do not write this
        feed. HTTP ingest, egress, and quarantine replay do.
      </p>
      <p className="text-slate-300 text-sm leading-relaxed">
        Sync is incremental. The connector calls{" "}
        <code className="text-slate-200">GET /v1/contracts/{"{id}"}/passed</code>{" "}
        with the saved cursor, oldest first, at most 1000 events per page, and
        checkpoints after every page. Rows newer than 5 seconds are left out,
        so an insert still in flight is not skipped. Events show up about 5
        seconds late. The next sync resumes from the saved cursor.
      </p>

      <H2 id="troubleshooting">Troubleshooting</H2>
      <ul className="list-disc pl-5 text-slate-300 text-sm space-y-2">
        <li>
          <span className="text-slate-100">400, bad group.</span> “Fivetran group
          &lt;id&gt; was not found”. Check Destination group id against{" "}
          <code className="text-slate-200">GET /v1/groups</code>. No key is minted.
        </li>
        <li>
          <span className="text-slate-100">400, bad schema.</span>{" "}
          <code className="text-slate-200">destination_schema</code> must match
          the rule above. A table name must start with a letter or underscore
          and use only lowercase letters, numbers, and underscores.
        </li>
        <li>
          <span className="text-slate-100">400, invalid cursor.</span> During a
          sync, a 400 whose message contains “invalid cursor” means the saved
          bookmark cannot be read. The connector drops that contract’s cursor
          and re-syncs from the oldest stored row. Any other 400 fails the sync
          and leaves the cursor in place.
        </li>
        <li>
          <span className="text-slate-100">401.</span> Fivetran rejected the API
          key or secret. Connect returns 401 before a ContractGate key is
          minted. During a later sync, 401 means the minted ContractGate key
          was rejected.
        </li>
        <li>
          <span className="text-slate-100">403.</span> The org is on{" "}
          <code className="text-slate-200">free</code> or{" "}
          <code className="text-slate-200">aisg_free</code>. Upgrade to Pro or
          Enterprise, then connect again.
        </li>
        <li>
          <span className="text-slate-100">410.</span> The cursor is older than
          plan retention: 90 days on Pro (<code className="text-slate-200">growth</code>,{" "}
          <code className="text-slate-200">aisg_pro</code>), 365 days on
          Enterprise. The connector drops the cursor and re-syncs from the
          start of what is still stored. Resync the same way if you clear the
          connection state yourself.
        </li>
      </ul>

      <H2 id="manual">Deploy the connector yourself</H2>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        If you do not use the button, install the public connector with the
        Fivetran Connector SDK CLI. The source is{" "}
        <a
          className="text-green-400 hover:underline"
          href="https://github.com/nightmoose/nmb-contractgate-fivetran"
        >
          github.com/nightmoose/nmb-contractgate-fivetran
        </a>
        . From that directory, with Python 3.10–3.14:
      </p>
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed mb-4">
{`python -m venv .venv
source .venv/bin/activate
pip install fivetran-connector-sdk

# FIVETRAN_API_KEY is Base64 of key:secret, not the ContractGate key.
fivetran deploy \\
  --api-key "$FIVETRAN_API_KEY" \\
  --destination "$FIVETRAN_DESTINATION_NAME" \\
  --connection contractgate_passed \\
  --configuration configuration.json \\
  --python 3.12`}
      </pre>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        <code className="text-slate-200">configuration.json</code> needs your
        ContractGate API key (from Account → API Keys), comma-separated
        contract ids, and optionally <code className="text-slate-200">base_url</code>{" "}
        and <code className="text-slate-200">table_names</code>. Production base
        URL is <code className="text-slate-200">https://api.datacontractgate.com</code>.
        The connection is created paused until you start the initial sync in
        Fivetran.
      </p>
      <p className="text-slate-400 text-sm leading-relaxed">
        A manual deploy that leaves <code className="text-slate-200">table_names</code>{" "}
        empty uses the connector default{" "}
        <code className="text-slate-200">passed_&lt;id&gt;</code> (the contract
        UUID, hyphens replaced by underscores), not the slug name the button
        sends. Set <code className="text-slate-200">table_names</code> to{" "}
        <code className="text-slate-200">contract_id=name</code> pairs when you
        want the same readable names. The setup test is named Configuration check.
      </p>
    </div>
  );
}
