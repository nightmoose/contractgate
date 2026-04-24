"use client";

import { useState } from "react";
import Link from "next/link";

// ── Code block component ──────────────────────────────────────────────────────
function Code({ children, language = "bash" }: { children: string; language?: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    navigator.clipboard.writeText(children.trim());
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };
  return (
    <div className="relative group my-4">
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed">
        <code>{children.trim()}</code>
      </pre>
      <button
        onClick={copy}
        className="absolute top-3 right-3 px-2 py-1 text-xs rounded bg-[#1f2937] text-slate-400 hover:text-slate-200 border border-[#374151] opacity-0 group-hover:opacity-100 transition-opacity"
      >
        {copied ? "Copied!" : "Copy"}
      </button>
      {language && (
        <span className="absolute top-3 left-4 text-xs text-slate-600 font-mono">{language}</span>
      )}
    </div>
  );
}

// ── Section heading ────────────────────────────────────────────────────────────
function H2({ id, children }: { id: string; children: React.ReactNode }) {
  return (
    <h2 id={id} className="text-xl font-semibold text-slate-100 mt-12 mb-4 scroll-mt-8 flex items-center gap-2">
      <a href={`#${id}`} className="text-slate-600 hover:text-green-400 text-sm">¶</a>
      {children}
    </h2>
  );
}

function H3({ children }: { children: React.ReactNode }) {
  return <h3 className="text-base font-semibold text-slate-200 mt-6 mb-3">{children}</h3>;
}

// ── Config table row ───────────────────────────────────────────────────────────
function ConfigRow({
  name, default: def, required, children,
}: { name: string; default?: string; required?: boolean; children: React.ReactNode }) {
  return (
    <tr className="border-b border-[#1f2937] hover:bg-[#0d1117]/50">
      <td className="py-3 pr-4 align-top">
        <code className="text-green-400 text-xs font-mono whitespace-nowrap">{name}</code>
        {required && <span className="ml-2 text-red-400 text-xs">required</span>}
      </td>
      <td className="py-3 pr-4 align-top text-slate-500 text-xs font-mono whitespace-nowrap">
        {def ?? "—"}
      </td>
      <td className="py-3 align-top text-slate-400 text-sm leading-relaxed">{children}</td>
    </tr>
  );
}

// ── Step badge ─────────────────────────────────────────────────────────────────
function Step({ n, title, children }: { n: number; title: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-4 mb-8">
      <div className="flex-shrink-0 w-8 h-8 rounded-full bg-green-900/40 border border-green-700/50 flex items-center justify-center text-green-400 font-bold text-sm">
        {n}
      </div>
      <div className="flex-1">
        <div className="font-semibold text-slate-100 mb-2">{title}</div>
        {children}
      </div>
    </div>
  );
}

// ── TOC ────────────────────────────────────────────────────────────────────────
const TOC = [
  { id: "quickstart",   label: "Quick Start" },
  { id: "installation", label: "Installation" },
  { id: "config",       label: "Configuration" },
  { id: "dlq",          label: "DLQ Setup" },
  { id: "headers",      label: "Result Headers" },
  { id: "tag-pass",     label: "Tag & Pass Mode" },
  { id: "examples",     label: "Full Examples" },
  { id: "faq",          label: "FAQ" },
];

// ── Page ───────────────────────────────────────────────────────────────────────
export default function KafkaConnectDocsPage() {
  return (
    <div className="flex gap-8 min-h-screen">
      {/* Sticky TOC sidebar */}
      <aside className="hidden xl:block w-52 flex-shrink-0 pt-10">
        <div className="sticky top-8">
          <p className="text-xs font-semibold text-slate-500 uppercase tracking-widest mb-3">On this page</p>
          <nav className="space-y-1">
            {TOC.map(({ id, label }) => (
              <a
                key={id}
                href={`#${id}`}
                className="block text-sm text-slate-500 hover:text-green-400 py-0.5 transition-colors"
              >
                {label}
              </a>
            ))}
          </nav>

          <div className="mt-8 p-3 bg-green-900/20 border border-green-700/30 rounded-lg">
            <p className="text-xs text-green-400 font-medium mb-1">Get your API key</p>
            <p className="text-xs text-slate-500 mb-2">Sign up free to get a contract ID and API key.</p>
            <Link
              href="/account"
              className="block text-center text-xs bg-green-600 hover:bg-green-500 text-white rounded px-3 py-1.5 transition-colors"
            >
              Go to Account →
            </Link>
          </div>
        </div>
      </aside>

      {/* Main content */}
      <main className="flex-1 max-w-3xl py-10 px-2">
        {/* Header */}
        <div className="mb-10">
          <div className="flex items-center gap-2 text-sm text-slate-500 mb-4">
            <Link href="/docs" className="hover:text-slate-300">Docs</Link>
            <span>/</span>
            <span className="text-slate-300">Kafka Connect</span>
          </div>
          <h1 className="text-3xl font-bold text-slate-100 mb-3">Kafka Connect SMT</h1>
          <p className="text-slate-400 text-lg leading-relaxed">
            Validate every Kafka record against a ContractGate semantic contract in real-time —
            before it reaches your data warehouse or AI systems. Invalid records go to a dead-letter
            topic. Valid records continue unchanged.
          </p>
          <div className="flex gap-3 mt-4 flex-wrap">
            <span className="text-xs bg-[#1f2937] text-slate-400 border border-[#374151] px-2 py-1 rounded">Java 11+</span>
            <span className="text-xs bg-[#1f2937] text-slate-400 border border-[#374151] px-2 py-1 rounded">Kafka Connect 2.8+</span>
            <span className="text-xs bg-[#1f2937] text-slate-400 border border-[#374151] px-2 py-1 rounded">Apache 2.0</span>
            <span className="text-xs bg-green-900/30 text-green-400 border border-green-700/40 px-2 py-1 rounded">v0.1.0</span>
          </div>
        </div>

        {/* ── Quick Start ───────────────────────────────────────────── */}
        <H2 id="quickstart">Quick Start</H2>
        <p className="text-slate-400 mb-6">Three steps from zero to validated records.</p>

        <Step n={1} title="Get your credentials">
          <p className="text-slate-400 text-sm mb-3">
            Sign up for a ContractGate account, create a contract, and copy your{" "}
            <strong className="text-slate-300">API key</strong> and{" "}
            <strong className="text-slate-300">contract UUID</strong> from the{" "}
            <Link href="/account" className="text-green-400 hover:underline">Account page</Link>.
          </p>
        </Step>

        <Step n={2} title="Install the connector">
          <p className="text-slate-400 text-sm mb-2">Via Confluent Hub CLI:</p>
          <Code language="bash">{`confluent-hub install datacontractgate/kafka-connect-contractgate:latest`}</Code>
          <p className="text-slate-400 text-sm mb-2">Or manually — extract the ZIP into your Connect plugin path:</p>
          <Code language="bash">{`unzip kafka-connect-contractgate-0.1.0.zip \\
  -d /usr/share/confluent-hub-components/
# Restart Connect workers after installing`}</Code>
        </Step>

        <Step n={3} title="Add the SMT to your connector config">
          <p className="text-slate-400 text-sm mb-2">
            Add these lines to any existing connector&apos;s properties file:
          </p>
          <Code language="properties">{`transforms=contractgate
transforms.contractgate.type=io.datacontractgate.connect.smt.ContractGateValidator
transforms.contractgate.contractgate.api.url=https://contractgate-api.fly.dev
transforms.contractgate.contractgate.api.key=cg_live_YOUR_API_KEY
transforms.contractgate.contractgate.contract.id=YOUR_CONTRACT_UUID

# Route invalid records to a dead-letter topic
errors.deadletterqueue.topic.name=your-topic.dlq
errors.deadletterqueue.context.headers.enable=true`}</Code>
          <p className="text-slate-400 text-sm mt-2">
            That&apos;s it. Every record is now validated before reaching its destination.
          </p>
        </Step>

        {/* ── Installation ───────────────────────────────────────────── */}
        <H2 id="installation">Installation</H2>
        <H3>Confluent Hub CLI</H3>
        <Code language="bash">{`confluent-hub install datacontractgate/kafka-connect-contractgate:latest`}</Code>

        <H3>Manual (Self-Managed Kafka)</H3>
        <p className="text-slate-400 text-sm mb-2">
          Download the ZIP from the{" "}
          <a href="https://www.confluent.io/hub/datacontractgate/kafka-connect-contractgate" className="text-green-400 hover:underline" target="_blank" rel="noreferrer">
            Confluent Hub listing
          </a>{" "}
          and extract it into your plugin path:
        </p>
        <Code language="bash">{`# Typical plugin path locations:
# Confluent Platform: /usr/share/confluent-hub-components/
# Self-managed:       /opt/kafka/plugins/

unzip kafka-connect-contractgate-0.1.0.zip -d /usr/share/confluent-hub-components/

# Add to connect-distributed.properties if not already:
plugin.path=/usr/share/confluent-hub-components`}</Code>

        <H3>Confluent Cloud (Custom Connector)</H3>
        <p className="text-slate-400 text-sm">
          Upload the JAR via the Confluent Cloud UI under{" "}
          <span className="text-slate-300">Connectors → Add plugin</span>, then reference the SMT
          class in your connector config as shown above.
        </p>

        {/* ── Configuration ─────────────────────────────────────────── */}
        <H2 id="config">Configuration Reference</H2>
        <p className="text-slate-400 text-sm mb-4">
          All settings are namespaced under <code className="text-green-400">contractgate.*</code> to
          avoid conflicts with other SMTs in a chain.
        </p>

        <div className="overflow-x-auto">
          <table className="w-full text-sm border-collapse">
            <thead>
              <tr className="border-b border-[#374151]">
                <th className="text-left py-2 pr-4 text-slate-500 font-medium text-xs uppercase tracking-wide">Key</th>
                <th className="text-left py-2 pr-4 text-slate-500 font-medium text-xs uppercase tracking-wide">Default</th>
                <th className="text-left py-2 text-slate-500 font-medium text-xs uppercase tracking-wide">Description</th>
              </tr>
            </thead>
            <tbody>
              <ConfigRow name="contractgate.api.url" required>
                Base URL of the ContractGate API. No trailing slash.
                Use <code className="text-slate-300">https://contractgate-api.fly.dev</code> for the hosted service.
              </ConfigRow>
              <ConfigRow name="contractgate.contract.id" required>
                UUID of the contract to validate against. Copy from the{" "}
                <Link href="/contracts" className="text-green-400 hover:underline">Contracts page</Link>.
              </ConfigRow>
              <ConfigRow name="contractgate.api.key" default='""'>
                Your API key (<code className="text-slate-300">x-api-key</code> header). Leave blank only for
                local dev with auth disabled.
              </ConfigRow>
              <ConfigRow name="contractgate.contract.version" default='"" (latest)'>
                Pin to a specific contract version, e.g. <code className="text-slate-300">1.2.0</code>.
                Leave blank to always use the latest stable version — recommended for most pipelines.
              </ConfigRow>
              <ConfigRow name="contractgate.on.failure" default="DLQ">
                What to do when a record fails.{" "}
                <code className="text-green-400">DLQ</code> — throw DataException for dead-letter routing.{" "}
                <code className="text-green-400">TAG_AND_PASS</code> — add violation headers and pass through.
              </ConfigRow>
              <ConfigRow name="contractgate.dry.run" default="false">
                When <code className="text-slate-300">true</code>, validates without writing to the audit log.
                Useful for high-throughput pipelines where you want enforcement without DB write pressure.
              </ConfigRow>
              <ConfigRow name="contractgate.connect.timeout.ms" default="5000">
                TCP connection timeout to the ContractGate API in milliseconds.
              </ConfigRow>
              <ConfigRow name="contractgate.request.timeout.ms" default="10000">
                Total HTTP request/response timeout in milliseconds. Keep well below
                Kafka Connect&apos;s task timeout.
              </ConfigRow>
              <ConfigRow name="contractgate.add.result.headers" default="true">
                Stamp <code className="text-slate-300">contractgate.*</code> metadata headers onto every record
                (pass or fail). See the <a href="#headers" className="text-green-400 hover:underline">headers reference</a> below.
              </ConfigRow>
              <ConfigRow name="contractgate.max.violation.headers" default="5">
                Maximum number of individual violation detail headers to add. High violation counts
                can bloat headers — cap at a useful number.
              </ConfigRow>
            </tbody>
          </table>
        </div>

        {/* ── DLQ Setup ─────────────────────────────────────────────── */}
        <H2 id="dlq">Dead-Letter Queue Setup</H2>
        <p className="text-slate-400 text-sm mb-4">
          When <code className="text-green-400">contractgate.on.failure=DLQ</code> (the default), the SMT throws a{" "}
          <code className="text-slate-300">DataException</code> on validation failure. Kafka Connect&apos;s
          built-in error handling routes the original record to a dead-letter topic.
        </p>

        <H3>Full DLQ connector config</H3>
        <Code language="properties">{`# ── ContractGate SMT ──────────────────────────────────────
transforms=contractgate
transforms.contractgate.type=io.datacontractgate.connect.smt.ContractGateValidator
transforms.contractgate.contractgate.api.url=https://contractgate-api.fly.dev
transforms.contractgate.contractgate.api.key=cg_live_YOUR_API_KEY
transforms.contractgate.contractgate.contract.id=YOUR_CONTRACT_UUID
transforms.contractgate.contractgate.on.failure=DLQ

# ── Kafka Connect DLQ settings ─────────────────────────────
# The topic to route failed records to (create it first)
errors.deadletterqueue.topic.name=orders.dlq
errors.deadletterqueue.topic.replication.factor=3

# Include the full violation summary in DLQ record headers
errors.deadletterqueue.context.headers.enable=true

# Log every DLQ-routed record (set to false at high error rates)
errors.log.enable=true
errors.log.include.messages=true

# Retry transient errors before sending to DLQ
errors.retry.timeout=60000
errors.retry.delay.max.ms=5000`}</Code>

        <H3>Reading violation details from the DLQ</H3>
        <p className="text-slate-400 text-sm mb-2">
          With <code className="text-slate-300">errors.deadletterqueue.context.headers.enable=true</code>,
          each DLQ record carries a header like:
        </p>
        <Code language="text">{`__connect.errors.exception.message:
  ContractGate validation failed — topic=orders partition=3 offset=1042
  contract=3fa85f64-... version=1.2.0
  2 violation(s): user_id [missing_required_field]: required field missing;
  amount [range_violation]: value -5 below minimum 0`}</Code>

        {/* ── Headers ───────────────────────────────────────────────── */}
        <H2 id="headers">Result Headers</H2>
        <p className="text-slate-400 text-sm mb-4">
          When <code className="text-green-400">contractgate.add.result.headers=true</code> (default),
          the following headers are added to every record — both passing and failing.
        </p>

        <div className="overflow-x-auto">
          <table className="w-full text-sm border-collapse">
            <thead>
              <tr className="border-b border-[#374151]">
                <th className="text-left py-2 pr-4 text-slate-500 font-medium text-xs uppercase tracking-wide">Header</th>
                <th className="text-left py-2 text-slate-500 font-medium text-xs uppercase tracking-wide">Value</th>
              </tr>
            </thead>
            <tbody>
              {[
                ["contractgate.passed", '"true" or "false"'],
                ["contractgate.contract.version", 'Resolved version string, e.g. "1.2.0"'],
                ["contractgate.violations.count", "Number of violations (0 on pass)"],
                ["contractgate.violation.0.field", 'Dot-path of field, e.g. "customer.address.country"'],
                ["contractgate.violation.0.kind", "missing_required_field · type_mismatch · enum_violation · range_violation · pattern_mismatch · length_violation · undeclared_field"],
                ["contractgate.violation.0.message", "Human-readable explanation"],
              ].map(([h, v]) => (
                <tr key={h} className="border-b border-[#1f2937] hover:bg-[#0d1117]/50">
                  <td className="py-3 pr-4 align-top">
                    <code className="text-green-400 text-xs font-mono whitespace-nowrap">{h}</code>
                  </td>
                  <td className="py-3 align-top text-slate-400 text-sm">{v}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="text-slate-500 text-xs mt-2">
          Violation headers repeat for indices 0…N up to{" "}
          <code className="text-slate-400">contractgate.max.violation.headers</code> (default 5).
        </p>

        {/* ── Tag & Pass ────────────────────────────────────────────── */}
        <H2 id="tag-pass">Tag &amp; Pass Mode</H2>
        <p className="text-slate-400 text-sm mb-4">
          Set <code className="text-green-400">contractgate.on.failure=TAG_AND_PASS</code> to never drop
          records. Invalid records get violation headers and continue downstream — consumers can inspect{" "}
          <code className="text-slate-300">contractgate.passed</code> and decide what to do.
        </p>
        <p className="text-slate-400 text-sm mb-4">
          Use this when you want <strong className="text-slate-300">observability without enforcement</strong>{" "}
          — for example, shadowing a new contract version before promoting it to stable.
        </p>
        <Code language="properties">{`transforms.contractgate.contractgate.on.failure=TAG_AND_PASS
# Records now flow through regardless of violations.
# Downstream consumers can branch on contractgate.passed=false.`}</Code>

        {/* ── Full Examples ─────────────────────────────────────────── */}
        <H2 id="examples">Full Examples</H2>

        <H3>S3 Sink with DLQ</H3>
        <Code language="json">{`{
  "name": "s3-sink-validated",
  "config": {
    "connector.class": "io.confluent.connect.s3.S3SinkConnector",
    "tasks.max": "4",
    "topics": "orders",
    "s3.region": "us-east-1",
    "s3.bucket.name": "my-data-lake",
    "storage.class": "io.confluent.connect.s3.storage.S3Storage",
    "format.class": "io.confluent.connect.s3.format.json.JsonFormat",

    "transforms": "contractgate",
    "transforms.contractgate.type": "io.datacontractgate.connect.smt.ContractGateValidator",
    "transforms.contractgate.contractgate.api.url": "https://contractgate-api.fly.dev",
    "transforms.contractgate.contractgate.api.key": "\${file:/secrets/cg.properties:api.key}",
    "transforms.contractgate.contractgate.contract.id": "YOUR_CONTRACT_UUID",

    "errors.deadletterqueue.topic.name": "orders.dlq",
    "errors.deadletterqueue.context.headers.enable": "true",
    "errors.retry.timeout": "60000"
  }
}`}</Code>

        <H3>JDBC Sink (high-throughput, dry-run audit)</H3>
        <Code language="json">{`{
  "name": "postgres-sink-validated",
  "config": {
    "connector.class": "io.confluent.connect.jdbc.JdbcSinkConnector",
    "tasks.max": "8",
    "topics": "user_events",
    "connection.url": "jdbc:postgresql://db:5432/analytics",

    "transforms": "contractgate",
    "transforms.contractgate.type": "io.datacontractgate.connect.smt.ContractGateValidator",
    "transforms.contractgate.contractgate.api.url": "https://contractgate-api.fly.dev",
    "transforms.contractgate.contractgate.api.key": "\${file:/secrets/cg.properties:api.key}",
    "transforms.contractgate.contractgate.contract.id": "YOUR_CONTRACT_UUID",
    "transforms.contractgate.contractgate.dry.run": "true",
    "transforms.contractgate.contractgate.on.failure": "DLQ",

    "errors.deadletterqueue.topic.name": "user_events.dlq",
    "errors.deadletterqueue.context.headers.enable": "true"
  }
}`}</Code>

        {/* ── FAQ ───────────────────────────────────────────────────── */}
        <H2 id="faq">FAQ</H2>

        {[
          {
            q: "Does this work with Avro / Protobuf / Schema Registry records?",
            a: "Yes. The SMT converts any record value to JSON before sending to ContractGate — Struct (Avro/Protobuf), Map, String, and byte[] are all handled automatically. Your schema registry setup is untouched.",
          },
          {
            q: "What happens if the ContractGate API is unreachable?",
            a: "The SMT fails open — it logs a warning and passes the record through unchanged. This prevents a transient API outage from halting your pipeline. You can tighten this with Kafka Connect's task-level retry and restart policies.",
          },
          {
            q: "Does it add latency to my pipeline?",
            a: "Typically 2–8ms per record at p99. Each validation call is a single HTTP POST to the ContractGate API. For very high-throughput pipelines (>50k records/sec), set contractgate.dry.run=true to skip audit DB writes on the server side.",
          },
          {
            q: "Can I use it with Confluent Cloud managed connectors?",
            a: "Yes — upload the JAR as a custom connector plugin via the Confluent Cloud UI, then reference the SMT class in your connector config as shown in the examples above.",
          },
          {
            q: "How do I pin a contract version?",
            a: "Set contractgate.contract.version=1.2.0. Leave it blank (the default) to always resolve to the latest stable version — this lets you promote new contract versions without redeploying connectors.",
          },
          {
            q: "Can I chain this with other SMTs?",
            a: "Yes. List multiple transforms: transforms=contractgate,maskPii,routeByField. The ContractGate SMT works anywhere in the chain; put it first to validate raw records before any transforms mutate them.",
          },
        ].map(({ q, a }) => (
          <div key={q} className="mb-6 border-b border-[#1f2937] pb-6 last:border-0">
            <div className="font-medium text-slate-200 mb-2">{q}</div>
            <div className="text-slate-400 text-sm leading-relaxed">{a}</div>
          </div>
        ))}

        {/* CTA */}
        <div className="mt-12 p-6 bg-green-900/20 border border-green-700/30 rounded-xl text-center">
          <h3 className="text-lg font-semibold text-green-400 mb-2">Ready to get started?</h3>
          <p className="text-slate-400 text-sm mb-4">
            Create a free account to get your API key and contract UUID.
          </p>
          <div className="flex gap-3 justify-center">
            <Link
              href="/auth/signup"
              className="px-5 py-2 bg-green-600 hover:bg-green-500 text-white rounded-lg text-sm font-medium transition-colors"
            >
              Sign up free →
            </Link>
            <Link
              href="/playground"
              className="px-5 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg text-sm font-medium transition-colors border border-[#374151]"
            >
              Try the Playground
            </Link>
          </div>
        </div>
      </main>
    </div>
  );
}
