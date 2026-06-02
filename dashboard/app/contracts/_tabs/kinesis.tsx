"use client";

/**
 * Kinesis Ingress tab — RFC-026.
 *
 * Lets users enable / disable AWS Kinesis ingress for a contract.
 * On enable: provisions three Kinesis streams + a scoped IAM user, shows
 *   credentials (copy-on-reveal, one-time).
 * On rotate: issues a new IAM access key and invalidates the old one.
 * On disable: revokes credentials immediately, streams deleted after drain window.
 */

import { useState } from "react";
import useSWR, { mutate } from "swr";
import clsx from "clsx";
import {
  getKinesisIngress,
  enableKinesisIngress,
  disableKinesisIngress,
  rotateKinesisCredentials,
} from "@/lib/api";
import type { KinesisIngressConfig } from "@/lib/api";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface KinesisTabProps {
  contractId: string;
}

// ---------------------------------------------------------------------------
// Copy-to-clipboard helper (same design as Kafka tab)
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
          Disable Kinesis Ingress?
        </h3>
        <p className="text-sm text-slate-400">
          IAM credentials will be revoked immediately. Kinesis streams will be
          deleted after the drain window (24 h). This action cannot be undone.
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
// Rotate confirm modal
// ---------------------------------------------------------------------------

function RotateModal({
  onConfirm,
  onCancel,
  rotating,
}: {
  onConfirm: () => void;
  onCancel: () => void;
  rotating: boolean;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6 max-w-sm w-full space-y-4 shadow-2xl">
        <h3 className="text-base font-semibold text-slate-100">
          Rotate IAM Credentials?
        </h3>
        <p className="text-sm text-slate-400">
          A new access key will be issued and your current key will be
          invalidated immediately. Update your producers before rotating.
        </p>
        <div className="flex gap-3 justify-end">
          <button
            onClick={onCancel}
            disabled={rotating}
            className="px-4 py-2 text-sm text-slate-400 hover:text-slate-200 border border-[#1f2937] rounded-lg transition-colors disabled:opacity-40"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={rotating}
            className="px-4 py-2 text-sm font-medium bg-amber-700 hover:bg-amber-600 text-white rounded-lg transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
          >
            {rotating ? "Rotating…" : "Rotate"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main tab
// ---------------------------------------------------------------------------

export function KinesisTab({ contractId }: KinesisTabProps) {
  const {
    data: config,
    isLoading,
  } = useSWR(
    contractId ? `kinesis-ingress-${contractId}` : null,
    () => getKinesisIngress(contractId).catch(() => null), // null = not enabled
    { revalidateOnFocus: false }
  );

  /** One-time secret returned immediately after enabling or rotation. */
  const [freshSecret, setFreshSecret] = useState<string | null>(null);
  const [freshConfig, setFreshConfig] = useState<KinesisIngressConfig | null>(null);

  const [enabling, setEnabling] = useState(false);
  const [showDisableModal, setShowDisableModal] = useState(false);
  const [disabling, setDisabling] = useState(false);
  const [showRotateModal, setShowRotateModal] = useState(false);
  const [rotating, setRotating] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const activeConfig = freshConfig ?? config;
  const isEnabled = activeConfig?.enabled === true;

  const handleEnable = async () => {
    setEnabling(true);
    setActionError(null);
    try {
      const result = await enableKinesisIngress(contractId);
      setFreshSecret(result.iam_secret_access_key ?? null);
      setFreshConfig(result);
      mutate(`kinesis-ingress-${contractId}`);
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
      await disableKinesisIngress(contractId);
      setFreshSecret(null);
      setFreshConfig(null);
      mutate(`kinesis-ingress-${contractId}`);
      setShowDisableModal(false);
    } catch (e: unknown) {
      setActionError(e instanceof Error ? e.message : "Disable failed");
    } finally {
      setDisabling(false);
    }
  };

  const handleRotateConfirm = async () => {
    setRotating(true);
    setActionError(null);
    try {
      const result = await rotateKinesisCredentials(contractId);
      setFreshSecret(result.iam_secret_access_key ?? null);
      setFreshConfig(result);
      mutate(`kinesis-ingress-${contractId}`);
      setShowRotateModal(false);
    } catch (e: unknown) {
      setActionError(e instanceof Error ? e.message : "Rotation failed");
    } finally {
      setRotating(false);
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
      {showRotateModal && (
        <RotateModal
          onConfirm={handleRotateConfirm}
          onCancel={() => setShowRotateModal(false)}
          rotating={rotating}
        />
      )}

      {/* Header */}
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold text-slate-100">
            Kinesis Ingress
          </h2>
          <p className="text-sm text-slate-400 mt-0.5">
            Produce events to a managed AWS Kinesis stream. ContractGate
            validates each record and routes it to your clean or quarantine
            stream automatically.
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
            aria-label={
              isEnabled ? "Disable Kinesis Ingress" : "Enable Kinesis Ingress"
            }
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
          Provisioning Kinesis streams and IAM credentials… (this may take up to
          60 s while streams become active)
        </div>
      )}

      {/* Credentials + stream panel — shown when enabled */}
      {isEnabled && activeConfig && (
        <div className="bg-[#0d1627] border border-[#1f2937] rounded-xl p-5 space-y-5">
          {freshSecret && (
            <div className="text-xs text-amber-400 bg-amber-950/40 border border-amber-700/50 rounded-lg px-3 py-2">
              ⚠ Copy your secret access key now — it will not be shown again.
            </div>
          )}

          {/* IAM Credentials */}
          <div className="grid gap-4">
            <CopyField
              label="AWS Region"
              value={activeConfig.aws_region}
            />
            <CopyField
              label="IAM Access Key ID"
              value={activeConfig.iam_access_key_id ?? ""}
            />
            {freshSecret && (
              <CopyField
                label="IAM Secret Access Key"
                value={freshSecret}
                secret
              />
            )}
          </div>

          {/* Rotate credentials */}
          <div className="flex items-center justify-between">
            <p className="text-xs text-slate-500">
              Rotate credentials to issue a new key and invalidate the current one.
            </p>
            <button
              onClick={() => setShowRotateModal(true)}
              disabled={rotating}
              className="text-xs px-3 py-1.5 border border-amber-700/60 text-amber-400 hover:bg-amber-950/40 rounded-lg transition-colors disabled:opacity-40 whitespace-nowrap"
            >
              Rotate credentials
            </button>
          </div>

          <hr className="border-[#1f2937]" />

          {/* Streams */}
          <div className="space-y-3">
            <p className="text-xs text-slate-400 font-medium uppercase tracking-wide">
              Streams
            </p>
            <div className="grid gap-2 text-xs font-mono">
              <div className="flex items-center gap-3">
                <span className="w-20 text-slate-500 shrink-0">Produce to</span>
                <code className="text-green-400 bg-[#0a0f1a] px-2 py-1 rounded border border-[#1f2937] break-all">
                  {activeConfig.stream_raw}
                </code>
              </div>
              <div className="flex items-center gap-3">
                <span className="w-20 text-slate-500 shrink-0">Valid out</span>
                <code className="text-blue-400 bg-[#0a0f1a] px-2 py-1 rounded border border-[#1f2937] break-all">
                  {activeConfig.stream_clean}
                </code>
              </div>
              <div className="flex items-center gap-3">
                <span className="w-20 text-slate-500 shrink-0">Invalid out</span>
                <code className="text-red-400 bg-[#0a0f1a] px-2 py-1 rounded border border-[#1f2937] break-all">
                  {activeConfig.stream_quarantine}
                </code>
              </div>
            </div>

            {/* ARNs — collapsed into a detail block so they don't crowd the UI */}
            {activeConfig.raw_stream_arn && (
              <details className="mt-2">
                <summary className="text-xs text-slate-500 cursor-pointer select-none hover:text-slate-400">
                  Show ARNs
                </summary>
                <div className="mt-2 grid gap-1 text-[11px] font-mono text-slate-400">
                  <div className="break-all">{activeConfig.raw_stream_arn}</div>
                  <div className="break-all">{activeConfig.clean_stream_arn}</div>
                  <div className="break-all">{activeConfig.quarantine_stream_arn}</div>
                </div>
              </details>
            )}
          </div>

          <hr className="border-[#1f2937]" />

          {/* Quick-start snippet */}
          <div className="space-y-2">
            <p className="text-xs text-slate-400 font-medium uppercase tracking-wide">
              Quick start (Python / boto3)
            </p>
            <pre className="text-[11px] text-slate-300 bg-[#0a0f1a] border border-[#1f2937] rounded-lg p-3 overflow-x-auto font-mono leading-relaxed">
{`import boto3, json

client = boto3.client(
    "kinesis",
    region_name="${activeConfig.aws_region}",
    aws_access_key_id="${activeConfig.iam_access_key_id ?? "<access-key-id>"}",
    aws_secret_access_key="<your-secret>",
)

client.put_record(
    StreamName="${activeConfig.stream_raw}",
    Data=json.dumps({"user_id": "u1", "event_type": "click", "timestamp": 1}),
    PartitionKey="partition-1",
)`}
            </pre>
          </div>
        </div>
      )}

      {/* Empty state */}
      {!isEnabled && !enabling && (
        <div className="border border-dashed border-[#1f2937] rounded-xl p-8 text-center space-y-3">
          <p className="text-sm text-slate-400">
            Kinesis ingress is not enabled for this contract.
          </p>
          <p className="text-xs text-slate-500">
            Enable it to receive a managed AWS Kinesis input stream, scoped IAM
            credentials, and automatic routing of valid / invalid events.
          </p>
          <button
            onClick={handleEnable}
            disabled={enabling}
            className="mt-2 px-4 py-2 bg-green-700 hover:bg-green-600 disabled:opacity-40 text-white text-sm font-medium rounded-lg transition-colors"
          >
            Enable Kinesis Ingress
          </button>
        </div>
      )}
    </div>
  );
}
