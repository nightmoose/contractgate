import Link from "next/link";

const DOCS = [
  {
    // RFC-089: raw markdown served from public/ — must be a plain <a>, not a
    // Next <Link>, so the browser fetches the file instead of client-routing.
    external: true,
    href: "/llm-integration.md",
    icon: "🤖",
    title: "Integration playbook for AI agents",
    badge: "paste this",
    badgeColor: "text-cyan-400 bg-cyan-900/30 border-cyan-700/40",
    description:
      "One URL to paste into Claude, Cursor, or Codex. The agent finds your event shape, infers a contract from real samples, deploys it, wires your producer to /v1/ingest, and verifies with a dry run — no docs reading required.",
    pills: ["copy-paste", "curl + TS + Python", "dry-run verified", "no signup to read"],
    cta: "Open the raw playbook →",
  },
  {
    external: true,
    href: "/mcp-reference.md",
    icon: "🔌",
    title: "MCP server",
    badge: "RFC-090",
    badgeColor: "text-cyan-400 bg-cyan-900/30 border-cyan-700/40",
    description:
      "Official Model Context Protocol server. Add npx -y @contractgate/mcp-server to Cursor, Claude Desktop, Windsurf, or Copilot and the agent gets typed tools for infer, dry-run, deploy, and quarantine — no curl.",
    pills: ["stdio", "npx", "API-key auth", "dry-run default"],
    cta: "Open the MCP reference →",
  },
  {
    external: false,
    href: "/docs/python-sdk",
    icon: "🐍",
    title: "Python SDK",
    badge: "v0.1.0",
    badgeColor: "text-green-400 bg-green-900/30 border-green-700/40",
    description:
      "First-party Python client for the ContractGate gateway. Validates events against semantic contracts via a simple sync or async HTTP client, and ships a pure-Python local validator for unit tests and pre-commit hooks — no network required.",
    pills: ["Python 3.9+", "sync + async", "local validator", "MIT"],
    cta: "Read the Python SDK docs →",
  },
  {
    external: false,
    href: "/docs/kafka-connect",
    icon: "🔗",
    title: "Kafka Connect SMT",
    badge: "v0.1.0",
    badgeColor: "text-green-400 bg-green-900/30 border-green-700/40",
    description:
      "A Kafka Connect Single Message Transform that validates every record against a ContractGate semantic contract in real-time — before it reaches your data warehouse or AI systems. Invalid records go to a dead-letter topic; valid records continue unchanged.",
    pills: ["Java 11+", "Kafka Connect 2.8+", "DLQ support", "Apache 2.0"],
    cta: "Read the Kafka Connect docs →",
  },
];

export default function DocsIndexPage() {
  return (
    <div className="max-w-3xl py-10">
      {/* Header */}
      <div className="mb-12">
        <h1 className="text-3xl font-bold text-slate-100 mb-3">Docs</h1>
        <p className="text-slate-400 text-lg leading-relaxed">
          Everything you need to integrate ContractGate into your stack.
          Pick an integration below to get started.
        </p>
      </div>

      {/* Cards */}
      <div className="space-y-5">
        {DOCS.map(({ external, href, icon, title, badge, badgeColor, description, pills, cta }) => {
          const cardClass =
            "group block bg-[#111827] border border-[#1f2937] hover:border-green-800/60 rounded-xl p-6 transition-colors";
          const body = (
            <div className="flex items-start gap-4">
              <span className="text-3xl mt-0.5">{icon}</span>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-3 mb-2 flex-wrap">
                  <span className="text-lg font-semibold text-slate-100 group-hover:text-green-400 transition-colors">
                    {title}
                  </span>
                  <span className={`text-xs px-2 py-0.5 rounded-full border font-medium ${badgeColor}`}>
                    {badge}
                  </span>
                </div>
                <p className="text-slate-400 text-sm leading-relaxed mb-4">
                  {description}
                </p>
                <div className="flex flex-wrap gap-2 mb-4">
                  {pills.map((p) => (
                    <span
                      key={p}
                      className="text-xs bg-[#1f2937] text-slate-400 border border-[#374151] px-2 py-0.5 rounded"
                    >
                      {p}
                    </span>
                  ))}
                </div>
                <span className="text-sm text-green-400 font-medium group-hover:underline">
                  {cta}
                </span>
              </div>
            </div>
          );

          return external ? (
            <a key={href} href={href} className={cardClass}>
              {body}
            </a>
          ) : (
            <Link key={href} href={href} className={cardClass}>
              {body}
            </Link>
          );
        })}
      </div>

      {/* Footer note */}
      <p className="mt-12 text-sm text-slate-600">
        More integrations coming soon. See the{" "}
        <Link href="/playground" className="text-slate-500 hover:text-green-400 transition-colors">
          Playground
        </Link>{" "}
        to test contracts interactively.
      </p>
    </div>
  );
}
