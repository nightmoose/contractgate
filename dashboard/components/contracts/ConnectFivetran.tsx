"use client";

import { useState } from "react";
import Link from "next/link";
import { connectFivetran, type FivetranConnectResponse } from "@/lib/api";
import { paidPlansPhrase } from "@/lib/org";

export interface FivetranContractOption {
  id: string;
  name: string;
}

const MAX_CONTRACTS = 32;
const MAX_TABLE_NAME = 63;

function validTableName(name: string): boolean {
  return /^[a-z_][a-z0-9_]{0,62}$/.test(name);
}

function validGroupId(value: string): boolean {
  return value.length > 0 && value.length <= 128 && /^[A-Za-z0-9_-]+$/.test(value);
}

function validCredential(value: string, allowColon: boolean): boolean {
  if (value.length < 8 || value.length > 256) return false;
  if (!allowColon && value.includes(":")) return false;
  return [...value].every((ch) => {
    const code = ch.charCodeAt(0);
    return code >= 33 && code <= 126;
  });
}

export default function ConnectFivetran({
  paid,
  contracts,
  onClose,
}: {
  /** `null` while GET /v1/me has not settled. `false` is a hosted free org. */
  paid: boolean | null;
  contracts: FivetranContractOption[];
  onClose: () => void;
}) {
  const [apiKey, setApiKey] = useState("");
  const [apiSecret, setApiSecret] = useState("");
  const [groupId, setGroupId] = useState("");
  const [selected, setSelected] = useState<string[]>([]);
  const [tables, setTables] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<FivetranConnectResponse | null>(null);

  const toggle = (id: string) => {
    setSelected((current) => {
      if (current.includes(id)) return current.filter((item) => item !== id);
      if (current.length >= MAX_CONTRACTS) return current;
      return [...current, id];
    });
  };

  const submit = async () => {
    setError(null);
    if (paid !== true) {
      setError(`Connect Fivetran is a paid-plan feature. Upgrade to ${paidPlansPhrase()}, then try again.`);
      return;
    }
    if (selected.length === 0) {
      setError("Select at least one contract.");
      return;
    }
    if (!validCredential(apiKey, false) || !validCredential(apiSecret, true)) {
      setError("Fivetran API key and secret must be 8–256 printable characters. The key cannot contain a colon.");
      return;
    }
    if (!validGroupId(groupId.trim())) {
      setError("Destination group id must be 1–128 letters, digits, underscores, or hyphens.");
      return;
    }
    const tablePairs: string[] = [];
    for (const id of selected) {
      const name = (tables[id] ?? "").trim();
      if (!name) continue;
      if (!validTableName(name)) {
        setError("Table names start with a letter or underscore and use only lowercase letters, numbers, and underscores.");
        return;
      }
      tablePairs.push(`${id}=${name}`);
    }

    const key = apiKey;
    const secret = apiSecret;
    setApiKey("");
    setApiSecret("");
    setBusy(true);
    try {
      const response = await connectFivetran({
        fivetran_api_key: key,
        fivetran_api_secret: secret,
        group_id: groupId.trim(),
        contract_ids: selected,
        ...(tablePairs.length > 0 ? { table_names: tablePairs.join(",") } : {}),
      });
      setResult(response);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Connect Fivetran failed.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="mb-6 bg-[#111827] border border-[#1f2937] rounded-xl p-5">
      <div className="flex items-start justify-between gap-4 mb-3">
        <div>
          <h2 className="text-lg font-semibold text-slate-100">Connect Fivetran</h2>
          <p className="text-sm text-slate-400 mt-1 max-w-2xl">
            Deploys the ContractGate connector into your Fivetran account. The API key and secret
            are used once for this request and are not stored.
          </p>
        </div>
        <button
          type="button"
          onClick={onClose}
          className="px-3 py-1.5 text-sm text-slate-400 hover:text-slate-200"
        >
          Close
        </button>
      </div>

      <p className="text-sm text-amber-200 bg-amber-900/20 border border-amber-700/40 rounded-lg px-3 py-2 mb-4">
        Paste a Fivetran API key and secret from your own account. ContractGate sends them to
        Fivetran for this deploy only, then drops them. They are not written to the database or logs.
      </p>

      {paid === false && (
        <p className="text-sm text-slate-300 mb-4">
          Connect Fivetran is a paid-plan feature.{" "}
          <Link href="/pricing" className="text-green-400 hover:underline">
            Upgrade
          </Link>{" "}
          to {paidPlansPhrase()}, then try again.
        </p>
      )}

      {result ? (
        <div className="text-sm text-slate-200 space-y-2">
          <p>Connection created. It starts paused so you can review it in Fivetran before the first sync.</p>
          <p>
            Connection id: <span className="font-mono text-slate-100">{result.connection_id}</span>
          </p>
          <p>Status: {result.status}</p>
          {result.setup_url && (
            <a
              href={result.setup_url}
              target="_blank"
              rel="noreferrer"
              className="inline-block text-green-400 hover:underline"
            >
              Open in Fivetran
            </a>
          )}
        </div>
      ) : (
        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            if (!busy) void submit();
          }}
        >
          <div className="grid gap-4 md:grid-cols-2">
            <label className="block text-sm text-slate-300">
              Fivetran API key
              <input
                type="password"
                autoComplete="off"
                value={apiKey}
                disabled={paid !== true || busy}
                onChange={(event) => setApiKey(event.target.value)}
                className="mt-1 w-full bg-[#0b1220] border border-[#1f2937] rounded-lg px-3 py-2 text-slate-100"
              />
            </label>
            <label className="block text-sm text-slate-300">
              Fivetran API secret
              <input
                type="password"
                autoComplete="off"
                value={apiSecret}
                disabled={paid !== true || busy}
                onChange={(event) => setApiSecret(event.target.value)}
                className="mt-1 w-full bg-[#0b1220] border border-[#1f2937] rounded-lg px-3 py-2 text-slate-100"
              />
            </label>
          </div>
          <label className="block text-sm text-slate-300">
            Destination group id
            <input
              type="text"
              autoComplete="off"
              value={groupId}
              disabled={paid !== true || busy}
              onChange={(event) => setGroupId(event.target.value)}
              placeholder="destination group id"
              className="mt-1 w-full bg-[#0b1220] border border-[#1f2937] rounded-lg px-3 py-2 text-slate-100"
            />
          </label>

          <fieldset disabled={paid !== true || busy} className="space-y-2">
            <legend className="text-sm text-slate-300 mb-2">
              Contracts to sync ({selected.length}/{MAX_CONTRACTS})
            </legend>
            {contracts.length === 0 ? (
              <p className="text-sm text-slate-500">Create a contract first, then connect it.</p>
            ) : (
              <ul className="max-h-56 overflow-auto space-y-2">
                {contracts.map((contract) => {
                  const checked = selected.includes(contract.id);
                  return (
                    <li key={contract.id} className="flex flex-wrap items-center gap-3">
                      <label className="flex items-center gap-2 text-sm text-slate-200 min-w-48">
                        <input
                          type="checkbox"
                          checked={checked}
                          onChange={() => toggle(contract.id)}
                        />
                        {contract.name}
                      </label>
                      {checked && (
                        <input
                          type="text"
                          value={tables[contract.id] ?? ""}
                          maxLength={MAX_TABLE_NAME}
                          placeholder="optional table name"
                          onChange={(event) =>
                            setTables((current) => ({
                              ...current,
                              [contract.id]: event.target.value,
                            }))
                          }
                          className="flex-1 min-w-40 bg-[#0b1220] border border-[#1f2937] rounded-lg px-3 py-1.5 text-sm text-slate-100"
                        />
                      )}
                    </li>
                  );
                })}
              </ul>
            )}
          </fieldset>

          {error && <p className="text-sm text-red-300">{error}</p>}

          <button
            type="submit"
            disabled={paid !== true || busy || contracts.length === 0}
            className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
          >
            {busy ? "Connecting…" : "Deploy connector"}
          </button>
        </form>
      )}
    </section>
  );
}
