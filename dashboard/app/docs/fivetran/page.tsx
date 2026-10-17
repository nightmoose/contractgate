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

      <H2 id="plans-and-self-hosting">Plans and self-hosting</H2>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Connect Fivetran is available on paid hosted plans and on any self-hosted
        instance with <code className="text-slate-200">CONTRACTGATE_SELF_HOSTED=true</code>.
      </p>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Hosted free plans get 403. See{" "}
        <Link href="#self-hosting" className="text-green-400 hover:underline">
          Self-hosting
        </Link>
        .
      </p>
      <p className="text-slate-400 text-sm leading-relaxed">
        Paid hosted plans are <code className="text-slate-200">growth</code>,{" "}
        <code className="text-slate-200">aisg_pro</code>, and{" "}
        <code className="text-slate-200">enterprise</code> (Pro and Enterprise
        in the dashboard). Hosted <code className="text-slate-200">free</code>{" "}
        and <code className="text-slate-200">aisg_free</code> receive that 403
        when <code className="text-slate-200">CONTRACTGATE_SELF_HOSTED</code> is
        unset. The API error names those three plan ids. The Contracts form
        reads <code className="text-slate-200">paid_features</code> from{" "}
        <code className="text-slate-200">GET /v1/me</code>. A hosted free org
        sees an upgrade prompt. If that field is missing, the form follows
        the stored plan.
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
        An unknown group returns 400: “Invalid request: Fivetran group &lt;id&gt; was not found”.
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

      <H2 id="request">Connect request</H2>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        <code className="text-slate-200">POST /v1/integrations/fivetran/connect</code>{" "}
        takes a JSON object. Send the ContractGate API key in{" "}
        <code className="text-slate-200">x-api-key</code>. The body max is 64 KiB.
      </p>
      <ul className="list-disc pl-5 text-slate-300 text-sm space-y-2 mb-4">
        <li>
          <code className="text-slate-200">fivetran_api_key</code> — required
          string, 8–256 ASCII graphic characters, no colon. Paste the key
          Fivetran showed. Do not Base64-encode it.
        </li>
        <li>
          <code className="text-slate-200">fivetran_api_secret</code> — required
          string, same length and character rules. A colon is allowed.
        </li>
        <li>
          <code className="text-slate-200">group_id</code> — required string,
          1–128 letters, digits, underscores, or hyphens. This is the
          destination group <code className="text-slate-200">id</code>, not the
          group name.
        </li>
        <li>
          <code className="text-slate-200">contract_ids</code> — required array
          of 1–32 UUID strings. Duplicates are rejected. Each id must belong to
          the caller&apos;s org.
        </li>
        <li>
          <code className="text-slate-200">table_names</code> — optional string,
          at most 2048 characters. Omit it, send null, or send{" "}
          <code className="text-slate-200">&quot;&quot;</code> to use the default
          name for every contract. Otherwise it is comma-separated{" "}
          <code className="text-slate-200">contract_id=name</code> pairs. Each
          id must be in <code className="text-slate-200">contract_ids</code> and
          may appear once. A name starts with a lowercase letter or underscore
          and then uses only lowercase letters, digits, and underscores, at most
          63 characters. Two contracts cannot share a name. A contract with no
          pair still gets the default name.
        </li>
        <li>
          <code className="text-slate-200">destination_schema</code> — optional
          string. Omit it or send null to get{" "}
          <code className="text-slate-200">contractgate_</code> plus 8 hex
          digits. A value starts with a lowercase letter and then uses only
          lowercase letters, digits, and underscores, at most 63 characters.
        </li>
      </ul>
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed mb-4">
{`curl -sS -X POST "$API/v1/integrations/fivetran/connect" \\
  -H "x-api-key: $CONTRACTGATE_API_KEY" \\
  -H "content-type: application/json" \\
  -d '{
    "fivetran_api_key": "your_fivetran_key",
    "fivetran_api_secret": "your_fivetran_secret",
    "group_id": "projected_sickle",
    "contract_ids": ["427c3304-ead1-4ba7-b7e0-123456789abc"],
    "table_names": "427c3304-ead1-4ba7-b7e0-123456789abc=user_events_427c3304"
  }'`}
      </pre>

      <H2 id="response">Connect response</H2>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        A created connection returns HTTP 200.{" "}
        <code className="text-slate-200">setup_url</code> is null when the
        connection id is not safe to put in a URL.{" "}
        <code className="text-slate-200">status</code> is{" "}
        <code className="text-slate-200">paused</code> until you start the sync.
        It is <code className="text-slate-200">setup_failed</code> when a setup
        test reports FAILED or JOB_FAILED.
      </p>
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed mb-4">
{`{
  "connection_id": "conn_id",
  "setup_url": "https://fivetran.com/dashboard/connections/conn_id/status",
  "status": "paused"
}`}
      </pre>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Errors use <code className="text-slate-200">{`{"error","status"}`}</code>.
      </p>
      <ul className="list-disc pl-5 text-slate-300 text-sm space-y-2 mb-4">
        <li>
          <span className="text-slate-100">400.</span> The body is not a JSON
          object, a required field is missing, a field fails the rule above, or
          the group does not exist. The <code className="text-slate-200">error</code>{" "}
          text starts with <code className="text-slate-200">Invalid request: </code>.
          An unknown group is{" "}
          <code className="text-slate-200">
            Invalid request: Fivetran group &lt;id&gt; was not found
          </code>
          . No ContractGate key is minted.
        </li>
        <li>
          <span className="text-slate-100">401.</span>{" "}
          <code className="text-slate-200">
            Unauthorized: missing or invalid API key
          </code>{" "}
          when the ContractGate key is missing and auth is on.{" "}
          <code className="text-slate-200">Fivetran rejected the API key</code>{" "}
          when Fivetran rejects the key or secret. That check runs before a
          ContractGate key is minted.
        </li>
        <li>
          <span className="text-slate-100">403.</span> The org plan is{" "}
          <code className="text-slate-200">free</code> or{" "}
          <code className="text-slate-200">aisg_free</code> and{" "}
          <code className="text-slate-200">CONTRACTGATE_SELF_HOSTED</code> is
          not set. The error is{" "}
          <code className="text-slate-200">
            Forbidden: Connect Fivetran is a paid-plan feature; upgrade to
            growth, aisg_pro, or enterprise
          </code>
          .
        </li>
        <li>
          <span className="text-slate-100">404.</span> A contract id is not in
          the caller&apos;s org. No key is minted.
        </li>
        <li>
          <span className="text-slate-100">413.</span> The body is over 64 KiB
          (65536 bytes). The error is{" "}
          <code className="text-slate-200">
            Payload too large: Connect Fivetran body exceeds 65536 bytes
          </code>
          . No key is minted.
        </li>
        <li>
          <span className="text-slate-100">500.</span>{" "}
          <code className="text-slate-200">CANONICAL_API_ORIGIN</code> is unset,
          blank, or not an https origin with no path, query, or credentials.
          The unset message is{" "}
          <code className="text-slate-200">
            CANONICAL_API_ORIGIN is unset. Set it to this deployment&apos;s
            public https API origin before connecting Fivetran.
          </code>{" "}
          No key is minted. There is no production fallback.
        </li>
      </ul>
      <p className="text-slate-400 text-sm leading-relaxed">
        A later Fivetran failure is HTTP 502,{" "}
        <code className="text-slate-200">Fivetran request failed (&lt;status&gt;)</code>.
        The key minted for that attempt is revoked, and an uploaded package that
        never became a connection is deleted.
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
        characters. Anything else is HTTP 400:{" "}
        <code className="text-slate-200">
          Invalid request: destination_schema must start with a lowercase letter
          and contain only lowercase letters, digits, and underscores, at most
          63 characters
        </code>
        . Fivetran still needs permission
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
        One table per contract. You can write the default name before the
        first sync. It is <code className="text-slate-200">&lt;slug&gt;_</code>{" "}
        plus the first 8 hex characters of the contract id, which are the eight
        characters before the first hyphen, in lowercase. Contract User Events
        and id{" "}
        <code className="text-slate-200">427c3304-ead1-4ba7-b7e0-123456789abc</code>{" "}
        becomes <code className="text-slate-200">user_events_427c3304</code>.
      </p>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        The slug keeps ASCII letters and digits only, lowercased. Each run of
        other characters becomes one underscore. Leading digits and
        underscores are dropped, so 123 Orders with that same id is{" "}
        <code className="text-slate-200">orders_427c3304</code>.{" "}
        <code className="text-slate-200">Café</code> becomes{" "}
        <code className="text-slate-200">caf_&lt;8hex&gt;</code>. The slug stops
        at 54 characters so the full name stays within 63. A name with no
        ASCII letters left, such as !!!, becomes{" "}
        <code className="text-slate-200">passed_</code> plus those same 8 hex
        characters: <code className="text-slate-200">passed_427c3304</code>.
        An optional table name on the form, or a{" "}
        <code className="text-slate-200">table_names</code> pair, replaces that
        default for that contract only. Names must be unique across the
        contracts you selected.
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
          <span className="text-slate-100">400, bad group.</span> “Invalid request:
          Fivetran group &lt;id&gt; was not found”. Check Destination group id against{" "}
          <code className="text-slate-200">GET /v1/groups</code>. No key is minted.
        </li>
        <li>
          <span className="text-slate-100">400, bad schema.</span> “Invalid request:
          destination_schema must start with a lowercase letter and contain only
          lowercase letters, digits, and underscores, at most 63 characters”.
          A bad table name is “Invalid request: table name must start with a
          letter or underscore and use only lowercase letters, numbers, and
          underscores”.
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
          <span className="text-slate-100">403.</span> On a hosted free plan,
          open{" "}
          <Link href="/pricing" className="text-green-400 hover:underline">
            Upgrade
          </Link>{" "}
          and move to Pro or Enterprise, then connect again. On a self-hosted
          instance, set{" "}
          <code className="text-slate-200">CONTRACTGATE_SELF_HOSTED</code> to{" "}
          <code className="text-slate-200">1</code>,{" "}
          <code className="text-slate-200">true</code>, or{" "}
          <code className="text-slate-200">TRUE</code>.
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

      <H2 id="cleanup">Cleanup and another connect</H2>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Each successful connect inserts one <code className="text-slate-200">api_keys</code>{" "}
        row named <code className="text-slate-200">Fivetran connector</code>,
        limited to the selected contracts, and creates one new paused Fivetran
        connection. The raw key is not returned. Connect does not look for an
        existing connection and does not update one. A second click mints a
        second key and a second connection.
      </p>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Run connect once per set of contracts. To change the contracts, the
        table names, or the schema, delete the Fivetran connection and revoke
        the key, then connect again. Leaving the old connection in place keeps
        it syncing on the old key.
      </p>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Revoke the key on Account → API Keys. The button calls{" "}
        <code className="text-slate-200">DELETE /api/keys</code> with{" "}
        <code className="text-slate-200">{`{"id":"<api key uuid>"}`}</code> and
        sets <code className="text-slate-200">revoked_at</code>. On the compose
        database the same update is:
      </p>
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed mb-4">
{`UPDATE api_keys
SET revoked_at = now()
WHERE org_id = '<org uuid>'
  AND name = 'Fivetran connector'
  AND revoked_at IS NULL
  AND deleted_at IS NULL;`}
      </pre>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        The gateway cache keeps a revoked key working for up to 60 seconds.
        Delete the connection in Fivetran from the{" "}
        <code className="text-slate-200">setup_url</code>, or with the same key
        and secret:
      </p>
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed mb-4">
{`curl -sS -u "$FIVETRAN_KEY:$FIVETRAN_SECRET" -X DELETE \\
  "https://api.fivetran.com/v1/connections/$CONNECTION_ID"`}
      </pre>
      <p className="text-slate-400 text-sm leading-relaxed">
        <code className="text-slate-200">$CONNECTION_ID</code> is{" "}
        <code className="text-slate-200">connection_id</code> from the 200
        response. ContractGate does not store it. A connect that fails before
        the key is minted (400, 401, 403, 404, 413, 500) leaves nothing to revoke.
        A connect that fails after the key is minted revokes that key itself.
      </p>

      <H2 id="self-hosting">Self-hosting</H2>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Self-hosted orgs default to <code className="text-slate-200">free</code>.
        Set <code className="text-slate-200">CONTRACTGATE_SELF_HOSTED=true</code>{" "}
        on the gateway. Passed-event export, Connect Fivetran, and payload
        storage then treat that org as paid. The connector key is a normal API
        key for the org, so it works against{" "}
        <code className="text-slate-200">GET /v1/contracts/{"{id}"}/passed</code>.
        Hosted <code className="text-slate-200">free</code> and{" "}
        <code className="text-slate-200">aisg_free</code> still receive 403.
        Leave the variable unset on hosted production and staging.
      </p>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Keep auth on for any instance Fivetran can reach. Fivetran&apos;s cloud
        must call the API over public HTTPS. An auth-off gateway on that URL
        would expose your data to anyone who can reach it. Set{" "}
        <code className="text-slate-200">CANONICAL_API_ORIGIN</code> to that
        public URL. Connect returns 500 and mints no key when it is unset.
      </p>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        <code className="text-slate-200">CONTRACTGATE_DEV_NO_AUTH</code> is only
        for local or private use. Fivetran cloud cannot call that process. Use
        the manual connector path from{" "}
        <a
          className="text-green-400 hover:underline"
          href="https://github.com/nightmoose/nmb-contractgate-fivetran"
        >
          github.com/nightmoose/nmb-contractgate-fivetran
        </a>{" "}
        when the gateway stays private. The commands are in the next section.
      </p>
      <p className="text-slate-400 text-sm leading-relaxed">
        docker-compose publishes the gateway on localhost and runs with auth
        off. That is the local stack. To let Fivetran reach it, put a tunnel or
        a reverse proxy with TLS in front, turn auth on, set{" "}
        <code className="text-slate-200">CONTRACTGATE_SELF_HOSTED=true</code>,
        and set <code className="text-slate-200">CANONICAL_API_ORIGIN</code> to
        the proxy&apos;s https URL.
      </p>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        A fresh <code className="text-slate-200">docker compose up</code> applies{" "}
        <code className="text-slate-200">supabase/migrations</code> once, then{" "}
        <code className="text-slate-200">ops/postgres/seed/099_demo_org.sql</code>.
        That seed inserts the demo org{" "}
        <code className="text-slate-200">cccccccc-cccc-cccc-cccc-cccccccccccc</code>{" "}
        on plan <code className="text-slate-200">free</code>. It does not insert
        a user or an API key. The <code className="text-slate-200">contractgate</code>{" "}
        CLI has no command that creates an org, a user, or a key.{" "}
        <code className="text-slate-200">POST /api/keys</code> needs a dashboard
        session, which this Postgres does not provide. Create the three rows in
        the compose database (<code className="text-slate-200">cg</code> /{" "}
        <code className="text-slate-200">cg</code>, database{" "}
        <code className="text-slate-200">contractgate</code>).
      </p>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Inserting <code className="text-slate-200">auth.users</code> fires{" "}
        <code className="text-slate-200">handle_new_user()</code>. That function
        inserts <code className="text-slate-200">user_profiles</code>, an{" "}
        <code className="text-slate-200">orgs</code> row, and an owner{" "}
        <code className="text-slate-200">org_memberships</code> row. The plan is{" "}
        <code className="text-slate-200">free</code> unless{" "}
        <code className="text-slate-200">raw_user_meta_data</code> has{" "}
        <code className="text-slate-200">{`product = aischemagate`}</code>, which
        starts the org on <code className="text-slate-200">aisg_free</code>.
        This is a new org. It is not the demo org.
      </p>
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed mb-4">
{`docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U cg -d contractgate <<'SQL'
INSERT INTO auth.users (id, email, raw_user_meta_data)
VALUES (
  '11111111-1111-1111-1111-111111111111',
  'owner@example.com',
  '{"display_name":"Warehouse"}'::jsonb
);
SQL`}
      </pre>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        Then mint a key in the same shape as{" "}
        <code className="text-slate-200">src/api_key_auth.rs</code>: raw{" "}
        <code className="text-slate-200">cg_live_</code> plus 48 hex characters,
        <code className="text-slate-200">key_prefix</code> the first 12
        characters, <code className="text-slate-200">key_hash</code> standard
        base64 of SHA-256 of the raw key (44 characters).{" "}
        <code className="text-slate-200">pgcrypto</code> is already installed.
        The statement prints the raw key once. Store it. It is not saved.
      </p>
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed mb-4">
{`docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U cg -d contractgate <<'SQL'
WITH minted AS (
  SELECT 'cg_live_' || encode(gen_random_bytes(24), 'hex') AS raw_key
),
inserted AS (
  INSERT INTO api_keys (user_id, org_id, name, key_prefix, key_hash)
  SELECT
    '11111111-1111-1111-1111-111111111111',
    m.org_id,
    'warehouse',
    left(minted.raw_key, 12),
    replace(encode(digest(minted.raw_key, 'sha256'), 'base64'), E'\\n', '')
  FROM minted
  JOIN org_memberships m
    ON m.user_id = '11111111-1111-1111-1111-111111111111'
   AND m.deleted_at IS NULL
  RETURNING id, key_prefix
)
SELECT inserted.id, inserted.key_prefix, minted.raw_key
FROM inserted CROSS JOIN minted;
SQL`}
      </pre>
      <p className="text-slate-300 text-sm leading-relaxed mb-3">
        On the gateway service, remove{" "}
        <code className="text-slate-200">CONTRACTGATE_DEV_NO_AUTH</code> (only{" "}
        <code className="text-slate-200">1</code>,{" "}
        <code className="text-slate-200">true</code>, and{" "}
        <code className="text-slate-200">TRUE</code> turn auth off). Set{" "}
        <code className="text-slate-200">CONTRACTGATE_SELF_HOSTED</code> to{" "}
        <code className="text-slate-200">1</code>,{" "}
        <code className="text-slate-200">true</code>, or{" "}
        <code className="text-slate-200">TRUE</code>. Any other value leaves the
        process hosted. Set{" "}
        <code className="text-slate-200">CANONICAL_API_ORIGIN</code> to the
        public https origin, then recreate the gateway container.
      </p>
      <p className="text-slate-400 text-sm leading-relaxed">
        <code className="text-slate-200">GET /v1/me</code> with that key still
        reports <code className="text-slate-200">plan</code>{" "}
        <code className="text-slate-200">free</code>, the free monthly limit,
        and the free rate limit. <code className="text-slate-200">paid_features</code>{" "}
        is true, which is the signal that passed-event export, Connect Fivetran,
        and payload storage are on. Use this key as{" "}
        <code className="text-slate-200">x-api-key</code> for connect, or as{" "}
        <code className="text-slate-200">api_key</code> in a manual{" "}
        <code className="text-slate-200">configuration.json</code>.
      </p>

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
        <code className="text-slate-200">configuration.json</code> is the object
        the connector reads. <code className="text-slate-200">api_key</code> and{" "}
        <code className="text-slate-200">contract_ids</code> are required.{" "}
        <code className="text-slate-200">base_url</code> and{" "}
        <code className="text-slate-200">table_names</code> are optional.
        Production base URL is{" "}
        <code className="text-slate-200">https://api.datacontractgate.com</code>.
        The connection is created paused until you start the initial sync in
        Fivetran.
      </p>
      <ul className="list-disc pl-5 text-slate-300 text-sm space-y-2 mb-4">
        <li>
          <code className="text-slate-200">api_key</code> — required string. A
          ContractGate key from Account → API Keys, or the compose key above.
          The connector sends it as <code className="text-slate-200">x-api-key</code>.
        </li>
        <li>
          <code className="text-slate-200">contract_ids</code> — required
          string of comma-separated contract UUIDs. Not a JSON array.
        </li>
        <li>
          <code className="text-slate-200">base_url</code> — optional https
          origin with no path, query, or credentials. Blank uses production.
          <code className="text-slate-200">http://</code> is accepted only for{" "}
          <code className="text-slate-200">localhost</code> and{" "}
          <code className="text-slate-200">127.0.0.1</code>.
        </li>
        <li>
          <code className="text-slate-200">table_names</code> — optional string
          of comma-separated <code className="text-slate-200">contract_id=name</code>{" "}
          pairs. Each id must be in <code className="text-slate-200">contract_ids</code>.
        </li>
      </ul>
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed mb-4">
{`{
  "api_key": "cg_live_0123456789abcdef0123456789abcdef0123456789abcdef",
  "contract_ids": "427c3304-ead1-4ba7-b7e0-123456789abc",
  "base_url": "https://api.datacontractgate.com",
  "table_names": "427c3304-ead1-4ba7-b7e0-123456789abc=user_events_427c3304"
}`}
      </pre>
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
