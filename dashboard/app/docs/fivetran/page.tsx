import Link from "next/link";

export default function FivetranDocsPage() {
  return (
    <div className="max-w-3xl py-10">
      <p className="text-sm text-slate-500 mb-3">
        <Link href="/docs" className="hover:text-green-400">
          Docs
        </Link>
      </p>
      <h1 className="text-3xl font-bold text-slate-100 mb-3">Fivetran connector</h1>
      <p className="text-slate-400 leading-relaxed mb-6">
        Connect Fivetran deploys the ContractGate connector into your own Fivetran
        account. ContractGate is DataContractGate and AISchemaGate sharing one
        engine. The connector reads events that passed validation and writes one
        table per contract.
      </p>

      <div className="bg-amber-900/15 border border-amber-700/30 text-amber-200 rounded-lg px-4 py-3 text-sm leading-relaxed mb-8">
        Your Fivetran API key and secret are used once for this deploy and are not
        stored. ContractGate mints a separate API key named Fivetran connector for
        the connection. Available on Pro and Enterprise.
      </div>

      <h2 className="text-xl font-semibold text-slate-100 mb-3">From the dashboard</h2>
      <ol className="list-decimal pl-5 text-slate-300 text-sm space-y-2 mb-8">
        <li>Open Contracts.</li>
        <li>Click Connect Fivetran.</li>
        <li>Paste the API key and secret from your Fivetran account, and the destination group id.</li>
        <li>Choose the contracts to sync. A table name is optional.</li>
        <li>Deploy. The connection is created paused so you can review it before the first sync.</li>
      </ol>

      <h2 className="text-xl font-semibold text-slate-100 mb-3">API</h2>
      <p className="text-sm text-slate-400 mb-3">
        <code className="text-slate-200">POST /v1/integrations/fivetran/connect</code>
      </p>
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed mb-4">
{`{
  "fivetran_api_key": "<fivetran key>",
  "fivetran_api_secret": "<fivetran secret>",
  "group_id": "<destination group id>",
  "contract_ids": ["<contract uuid>"]
}`}
      </pre>
      <p className="text-sm text-slate-400">
        The response is <code className="text-slate-200">connection_id</code>,{" "}
        <code className="text-slate-200">status</code>, and an optional{" "}
        <code className="text-slate-200">setup_url</code>. Field rules are in{" "}
        <a
          className="text-green-400 hover:underline"
          href="https://github.com/nightmoose/contractgate/blob/main/docs/fivetran-connect-reference.md"
        >
          the reference
        </a>
        .
      </p>
    </div>
  );
}
