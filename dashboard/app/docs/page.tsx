import Link from "next/link";

const DOCS = [
  {
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
        {DOCS.map(({ href, icon, title, badge, badgeColor, description, pills, cta }) => (
          <Link
            key={href}
            href={href}
            className="group block bg-[#111827] border border-[#1f2937] hover:border-green-800/60 rounded-xl p-6 transition-colors"
          >
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
          </Link>
        ))}
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
