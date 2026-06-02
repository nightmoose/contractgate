"use client";

/**
 * Kafka Ingress tab — RFC-025.
 *
 * Lets users enable / disable Confluent Cloud ingress for a contract.
 * On enable: shows bootstrap server + credentials (copy-on-reveal, one-time).
 * On disable: confirms, then revokes credentials and soft-deletes topics.
 */

import { useState, useEffect } from "react";
import useSWR, { mutate } from "swr";
import clsx from "clsx";
import {
  getKafkaIngress,
  enableKafkaIngress,
  disableKafkaIngress,
} from "@/lib/api";
import type { KafkaIngressConfig } from "@/lib/api";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface KafkaTabProps {
  contractId: string;
}

// ---------------------------------------------------------------------------
// Copy-to-clipboard helper
// ---------------------------------------------------------------------------

function CopyField({
  label,
  value,
  secret = false,
}: {
  label: string;
  value: string;
  secret?: boolean;
}) {
  const [revealed, setRevealed] = useState(!secret);
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText(value).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <div className="space-y-1">
      <p className="text-xs text-slate-400 font-medium uppercase tracking-wide">
        {label}
      </p>
      <div className="flex items-center gap-2">
        <code className="flex-1 bg-[#0a0f1a] text-green-400 text-xs px-3 py-2 rounded-lg font-mono break-all select-all border border-[#1f2937]">
          {revealed ? value : "••••••••••••••••••••••••"}
        </code>
        {secret && (
          <button
            onClick={() => setRevealed((r) => !r)}
            className="text-xs text-slate-400 hover:text-slate-200 px-2 py-1 rounded border border-[#1f2937] transition-colors whitespace-nowrap"
          >
            {revealed ? "Hide" : "Reveal"}
          </button>
        )}
        <button
          onClick={handleCopy}
          className="text-xs text-slate-400 hover:text-slate-200 px-2 py-1 rounded border border-[#1f2937] transition-colors whitespace-nowrap"
        >
          {copied ? "Copied!" : "Copy"}
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Disable confirm modal
// ---------------------------------------------------------------------------

function DisableModal({
  onConfirm,
  onCancel,
  disabling,
}: {
  onConfirm: () => void;
  onCancel: () => void;
  disabling: boolean;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6 max-w-sm w-full space-y-4 shadow-2xl">
        <h3 className="text-base font-semibold text-slate-100">
          Disable Kafka Ingress?
        </h3>
        <p className="text-sm text-slate-400">
          Credentials will be revoked immediately. Topics will be deleted after
          the drain window (24 h). This action cannot be undone.
        </p>
        <div className="flex gap-3 justify-end">
          <button
            onClick={onCancel}
            disabled={disabling}
            className="px-4 py-2 text-sm text-slate-400 hover:text-slate-200 border border-[#1f2937] rounded-lg transition-colors disabled:opacity-40"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={disabling}
            className="px-4 py-2 text-sm font-medium bg-red-700 hover:bg-red-600 text-white rounded-lg transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
          >
            {disabling ? "Disabling…" : "Disable"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main tab
// ---------------------------------------------------------------------------

export function KafkaTab({ contractId }: KafkaTabProps) {
  const {
    data: config,
    error: fetchError,
    isLoading,
  } = useSWR(
    contractId ? `kafka-ingress-${contractId}` : null,
    () => getKafkaIngress(contractId).catch(() => null), // null = not enabled
    { revalidateOnFocus: false }
  );

  /** One-time secret returned immediately after enabling — not in SWR cache. */
  const [freshSecret, setFreshSecret] = useState<string | null>(null);
  const [freshConfig, setFreshConfig] = useState<KafkaIngressConfig | null>(null);

  const [enabling, setEnabling] = useState(false);
  const [showDisableModal, setShowDisableModal] = useState(false);
  const [disabling, setDisabling] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const activeConfig = freshConfig ?? config;
  const isEnabled = activeConfig?.enabled === true;

  const handleEnable = async () => {
    setEnabling(true);
    setActionError(null);
    try {
      const result = await enableKafkaIngress(contractId);
      setFreshSecret(result.sasl_password ?? null);
      setFreshConfig(result);
      mutate(`kafka-ingress-${contractId}`);
    } catch (e: unknown) {
      setActionError(e instanceof Error ? e.message : "Enable failed");
    } finally {
      setEnabling(false);
    }
  };

  const handleDisableConfirm = async () => {
    setDisabling(true);
    setActionError(null);
    try {
      await disableKafkaIngress(contractId);
      setFreshSecret(null);
      setFreshConfig(null);
      mutate(`kafka-ingress-${contractId}`);
      setShowDisableModal(false);
    } catch (e: unknown) {
      setActionError(e instanceof Error ? e.message : "Disable failed");
    } finally {
      setDisabling(false);
    }
  };

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-40 text-slate-500 text-sm">
        Loading…
      </div>
    );
  }

  return (
    <div className="space-y-6 max-w-2xl">
      {showDisableModal && (
        <DisableModal
          onConfirm={handleDisableConfirm}
          onCancel={() => setShowDisableModal(false)}
          disabling={disabling}
        />
      )}

      {/* Header */}
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold text-slate-100">
            Kafka Ingress
          </h2>
          <p className="text-sm text-slate-400 mt-0.5">
            Produce events directly to a Confluent Cloud topic. ContractGate
            validates each message and routes it to your clean or quarantine
            topic automatically.
          </p>
        </div>

        {/* Toggle */}
        <div className="flex items-center gap-3 shrink-0">
          <span
            className={clsx(
              "text-xs font-medium",
              isEnabled ? "text-green-400" : "text-slate-500"
            )}
          >
            {isEnabled ? "Enabled" : "Disabled"}
          </span>
          <button
            onClick={isEnabled ? () => setShowDisableModal(true) : handleEnable}
            disabled={enabling || disabling}
            className={clsx(
              "relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none disabled:opacity-40 disabled:cursor-not-allowed",
              isEnabled ? "bg-green-600" : "bg-slate-700"
            )}
            aria-label={isEnabled ? "Disable Kafka Ingress" : "Enable Kafka Ingress"}
          >
            <span
              className={clsx(
                "inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform",
                isEnabled ? "translate-x-6" : "translate-x-1"
              )}
            />
          </button>
        </div>
      </div>

      {actionError && (
        <div className="text-sm text-red-400 bg-red-950/40 border border-red-800/50 rounded-lg px-4 py-3">
          {actionError}
        </div>
      )}

      {enabling && (
        <div className="text-sm text-slate-400 bg-[#0a0f1a] border border-[#1f2937] rounded-lg px-4 py-3">
          Provisioning Confluent topics and credentials…
        </div>
      )}

      {/* Credentials panel — shown when enabled */}
      {isEnabled && activeConfig && (
        <div className="bg-[#0d1627] border border-[#1f2937] rounded-xl p-5 space-y-5">
          {freshSecret && (
            <div className="text-xs text-amber-400 bg-amber-950/40 border border-amber-700/50 rounded-lg px-3 py-2">
              ⚠ Copy your password now — it will not be shown again.
            </div>
          )}

          <div className="grid gap-4">
            <CopyField
              label="Bootstrap Servers"
              value={activeConfig.bootstrap_servers}
            />
            <CopyField
              label="SASL Username (API Key)"
              value={activeConfig.sasl_username}
            />
            {freshSecret && (
              <CopyField
                label="SASL Password (API Secret)"
                value={freshSecret}
                secret
              />
            )}
          </div>

          <hr className="border-[#1f2937]" />

          <div className="space-y-3">
            <p className="text-xs text-slate-400 font-medium uppercase tracking-wide">
              Topics
            </p>
            <div className="grid gap-2 text-xs font-mono">
              <div className="flex items-center gap-3">
                <span className="w-20 text-slate-500 shrink-0">Produce to</span>
                <code className="text-green-400 bg-[#0a0f1a] px-2 py-1 rounded border border-[#1f2937] break-all">
                  {activeConfig.topic_raw}
                </code>
              </div>
              <div className="flex items-center gap-3">
                <span className="w-20 text-slate-500 shrink-0">Valid out</span>
                <code className="text-blue-400 bg-[#0a0f1a] px-2 py-1 rounded border border-[#1f2937] break-all">
                  {activeConfig.topic_clean}
                </code>
              </div>
              <div className="flex items-center gap-3">
                <span className="w-20 text-slate-500 shrink-0">Invalid out</span>
                <code className="text-red-400 bg-[#0a0f1a] px-2 py-1 rounded border border-[#1f2937] break-all">
                  {activeConfig.topic_quarantine}
                </code>
              </div>
            </div>
          </div>

          <hr className="border-[#1f2937]" />

          {/* Quick-start snippet */}
          <div className="space-y-2">
            <p className="text-xs text-slate-400 font-medium uppercase tracking-wide">
              Quick start (Python)
            </p>
            <pre className="text-[11px] text-slate-300 bg-[#0a0f1a] border border-[#1f2937] rounded-lg p-3 overflow-x-auto font-mono leading-relaxed">
{`from confluent_kafka import Producer

p = Producer({
    "bootstrap.servers": "${activeConfig.bootstrap_servers}",
    "security.protocol": "SASL_SSL",
    "sasl.mechanisms": "PLAIN",
    "sasl.username": "${activeConfig.sasl_username}",
    "sasl.password": "<your-secret>",
})

p.produce("${activeConfig.topic_raw}", value='{"user_id":"u1","event_type":"click","timestamp":1}')
p.flush()`}
            </pre>
          </div>
        </div>
      )}

      {/* Empty state */}
      {!isEnabled && !enabling && (
        <div className="border border-dashed border-[#1f2937] rounded-xl p-8 text-center space-y-3">
          <p className="text-sm text-slate-400">
            Kafka ingress is not enabled for this contract.
          </p>
          <p className="text-xs text-slate-500">
            Enable it to receive a Confluent Cloud input topic, credentials, and
            automatic routing of valid / invalid events.
          </p>
          <button
            onClick={handleEnable}
            disabled={enabling}
            className="mt-2 px-4 py-2 bg-green-700 hover:bg-green-600 disabled:opacity-40 text-white text-sm font-medium rounded-lg transition-colors"
          >
            Enable Kafka Ingress
          </button>
        </div>
      )}
    </div>
  );
}
