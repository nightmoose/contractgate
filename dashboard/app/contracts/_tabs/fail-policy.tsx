"use client";

import { useEffect, useState } from "react";
import clsx from "clsx";
import {
  getDropCounts,
  getFailPolicy,
  resetAccountGuard,
  resetContractGuard,
  setAccountGuard,
  setFailPolicy,
  type DropCountRow,
  type FailPolicy,
  type QuarantineGuardConfig,
} from "@/lib/api";

function blank(value: number | null | undefined): string {
  return value == null ? "" : String(value);
}

function parseLimit(raw: string): number | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  if (!Number.isInteger(n) || n < 1) {
    throw new Error("Limits must be whole numbers of at least 1, or left empty.");
  }
  return n;
}

function guardFromInputs(
  enforce: boolean,
  perMinute: string,
  perHour: string,
  cooldown: string,
  sample: boolean
): QuarantineGuardConfig {
  return {
    enforce,
    max_per_minute: parseLimit(perMinute),
    max_per_hour: parseLimit(perHour),
    cooldown_minutes: parseLimit(cooldown),
    sample_drops: sample,
  };
}

export function FailPolicyPanel({ contractId }: { contractId: string }) {
  const [policy, setPolicy] = useState<FailPolicy | null>(null);
  const [counts, setCounts] = useState<DropCountRow[]>([]);
  const [onFail, setOnFail] = useState<"quarantine" | "drop">("quarantine");
  const [locked, setLocked] = useState(false);
  const [enforce, setEnforce] = useState(false);
  const [perMinute, setPerMinute] = useState("");
  const [perHour, setPerHour] = useState("");
  const [cooldown, setCooldown] = useState("");
  const [sample, setSample] = useState(false);
  const [accountEnforce, setAccountEnforce] = useState(false);
  const [accountMinute, setAccountMinute] = useState("");
  const [accountHour, setAccountHour] = useState("");
  const [accountCooldown, setAccountCooldown] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  async function load() {
    const [next, rows] = await Promise.all([
      getFailPolicy(contractId),
      getDropCounts(contractId, 24),
    ]);
    setPolicy(next);
    setCounts(rows);
    setOnFail(next.on_fail);
    setLocked(next.on_fail_locked);
    setEnforce(next.quarantine_guard.enforce);
    setPerMinute(blank(next.quarantine_guard.max_per_minute));
    setPerHour(blank(next.quarantine_guard.max_per_hour));
    setCooldown(blank(next.quarantine_guard.cooldown_minutes));
    setSample(next.quarantine_guard.sample_drops);
    const account = next.account_guard;
    setAccountEnforce(account?.enforce ?? false);
    setAccountMinute(blank(account?.max_per_minute));
    setAccountHour(blank(account?.max_per_hour));
    setAccountCooldown(blank(account?.cooldown_minutes));
  }

  useEffect(() => {
    let cancelled = false;
    setError(null);
    load()
      .catch((err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
    // contractId is the only input. load closes over the latest setters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [contractId]);

  async function saveContract() {
    setBusy(true);
    setError(null);
    setSaved(null);
    try {
      const next = await setFailPolicy(contractId, {
        on_fail: onFail,
        on_fail_locked: locked,
        quarantine_guard: guardFromInputs(enforce, perMinute, perHour, cooldown, sample),
      });
      setPolicy(next);
      setSaved("Contract setting saved.");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function saveAccount() {
    setBusy(true);
    setError(null);
    setSaved(null);
    try {
      const guard = guardFromInputs(accountEnforce, accountMinute, accountHour, accountCooldown, false);
      const configured = guard.max_per_minute != null || guard.max_per_hour != null || guard.enforce;
      await setAccountGuard(configured ? guard : null);
      await load();
      setSaved("Account cap saved. It overrides this contract when a threshold is set.");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function resume() {
    setBusy(true);
    setError(null);
    try {
      if (policy?.auto_trip?.scope === "account") {
        await resetAccountGuard();
      } else {
        await resetContractGuard(contractId);
      }
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  const trip = policy?.auto_trip;
  const total = counts.reduce((sum, row) => sum + row.dropped, 0);

  return (
    <div className="space-y-3">
      {trip?.active && (
        <div className="rounded-xl border border-amber-700/50 bg-amber-950/40 px-4 py-3 text-sm text-amber-100">
          <p>
            Safety cap is enforcing
            {trip.scope === "account" ? " for the account" : " for this contract"}.
            Failing records are counted and not stored
            {trip.until ? ` until ${new Date(trip.until).toLocaleString()}` : " until an owner resets it"}.
          </p>
          <button
            type="button"
            onClick={resume}
            disabled={busy}
            className="mt-2 text-xs text-amber-200 underline disabled:opacity-50"
          >
            Resume quarantine now
          </button>
        </div>
      )}
      {trip && !trip.active && trip.would_trip && (
        <div className="rounded-xl border border-slate-600 bg-[#111827] px-4 py-3 text-sm text-slate-300">
          Safety cap would switch failing records to drop
          {trip.enforcing ? "." : " (watch only)."}
          {" "}
          {trip.enforcing
            ? "The next failing batch will count them instead of storing them."
            : "Failing records are still quarantined until enforcement is turned on."}
        </div>
      )}

      <div className="rounded-xl border border-[#1f2937] bg-[#111827] px-4 py-4 space-y-3">
        <p className="text-xs uppercase tracking-wider text-slate-500">When a record fails validation</p>
        <label className="flex items-start gap-2 text-sm text-slate-200">
          <input
            type="radio"
            name="on-fail"
            checked={onFail === "quarantine"}
            onChange={() => setOnFail("quarantine")}
          />
          <span>Quarantine it — keep it for review and replay (default)</span>
        </label>
        <label className="flex items-start gap-2 text-sm text-slate-200">
          <input type="radio" name="on-fail" checked={onFail === "drop"} onChange={() => setOnFail("drop")} />
          <span>Drop it — count it, keep nothing</span>
        </label>
        <label className="flex items-center gap-2 text-xs text-slate-400">
          <input type="checkbox" checked={locked} onChange={(e) => setLocked(e.target.checked)} />
          Lock this choice so requests cannot override it
        </label>

        <div className="border-t border-[#1f2937] pt-3 space-y-2">
          <label className="flex items-center gap-2 text-sm text-slate-200">
            <input type="checkbox" checked={enforce} onChange={(e) => setEnforce(e.target.checked)} />
            Enforce the safety cap
          </label>
          <p className="text-xs text-slate-500">
            Off means watch only: a crossing is recorded and failing records stay quarantined.
            Leave a box empty when that window has no limit. No default numbers are set.
          </p>
          <div className="flex flex-wrap gap-3 text-xs text-slate-400">
            <label>
              Per minute{" "}
              <input
                value={perMinute}
                onChange={(e) => setPerMinute(e.target.value)}
                inputMode="numeric"
                placeholder="not set"
                className="ml-1 w-24 bg-[#0b1220] border border-[#1f2937] rounded px-2 py-1 text-slate-200"
              />
            </label>
            <label>
              Per hour{" "}
              <input
                value={perHour}
                onChange={(e) => setPerHour(e.target.value)}
                inputMode="numeric"
                placeholder="not set"
                className="ml-1 w-24 bg-[#0b1220] border border-[#1f2937] rounded px-2 py-1 text-slate-200"
              />
            </label>
            <label>
              Cooldown minutes{" "}
              <input
                value={cooldown}
                onChange={(e) => setCooldown(e.target.value)}
                inputMode="numeric"
                placeholder="manual reset only"
                className="ml-1 w-28 bg-[#0b1220] border border-[#1f2937] rounded px-2 py-1 text-slate-200"
              />
            </label>
          </div>
          <label className="flex items-center gap-2 text-xs text-slate-400">
            <input type="checkbox" checked={sample} onChange={(e) => setSample(e.target.checked)} />
            Keep up to 5 metadata samples per minute (failure reasons only, never record contents)
          </label>
          <button
            type="button"
            onClick={saveContract}
            disabled={busy}
            className="text-xs bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-white rounded px-3 py-1.5"
          >
            Save
          </button>
        </div>

        <div className="border-t border-[#1f2937] pt-3 space-y-2">
          <p className="text-xs uppercase tracking-wider text-slate-500">Account cap</p>
          <p className="text-xs text-slate-500">
            When the account has a threshold, it overrides this contract. It resets after the cooldown or when an owner resets it.
          </p>
          <label className="flex items-center gap-2 text-sm text-slate-200">
            <input
              type="checkbox"
              checked={accountEnforce}
              onChange={(e) => setAccountEnforce(e.target.checked)}
            />
            Enforce the account cap
          </label>
          <div className="flex flex-wrap gap-3 text-xs text-slate-400">
            <label>
              Per minute{" "}
              <input
                value={accountMinute}
                onChange={(e) => setAccountMinute(e.target.value)}
                placeholder="not set"
                className="ml-1 w-24 bg-[#0b1220] border border-[#1f2937] rounded px-2 py-1 text-slate-200"
              />
            </label>
            <label>
              Per hour{" "}
              <input
                value={accountHour}
                onChange={(e) => setAccountHour(e.target.value)}
                placeholder="not set"
                className="ml-1 w-24 bg-[#0b1220] border border-[#1f2937] rounded px-2 py-1 text-slate-200"
              />
            </label>
            <label>
              Cooldown minutes{" "}
              <input
                value={accountCooldown}
                onChange={(e) => setAccountCooldown(e.target.value)}
                placeholder="manual reset only"
                className="ml-1 w-28 bg-[#0b1220] border border-[#1f2937] rounded px-2 py-1 text-slate-200"
              />
            </label>
          </div>
          <button
            type="button"
            onClick={saveAccount}
            disabled={busy}
            className="text-xs border border-[#374151] text-slate-300 rounded px-3 py-1.5 disabled:opacity-50"
          >
            Save account cap
          </button>
        </div>
        {saved && <p className="text-xs text-green-400">{saved}</p>}
        {error && <p className="text-xs text-red-400">{error}</p>}
      </div>

      <div className="rounded-xl border border-[#1f2937] overflow-hidden">
        <div className="px-4 py-2 text-xs text-slate-400 bg-[#111827] flex justify-between">
          <span>Dropped records (last 24h)</span>
          <span className={clsx(total > 0 ? "text-slate-200" : "text-slate-600")}>{total.toLocaleString()}</span>
        </div>
        {counts.length === 0 ? (
          <p className="px-4 py-3 text-xs text-slate-600">No dropped records in this window.</p>
        ) : (
          <table className="w-full text-xs">
            <thead>
              <tr className="text-left text-slate-500">
                <th className="px-4 py-2 font-medium">Reason</th>
                <th className="px-4 py-2 font-medium">Source</th>
                <th className="px-4 py-2 font-medium">Count</th>
              </tr>
            </thead>
            <tbody>
              {counts.map((row) => (
                <tr key={`${row.reason_key}:${row.source}`} className="border-t border-[#1f2937] text-slate-300">
                  <td className="px-4 py-2 font-mono">{row.reason_key}</td>
                  <td className="px-4 py-2">{row.source}</td>
                  <td className="px-4 py-2">{row.dropped.toLocaleString()}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
