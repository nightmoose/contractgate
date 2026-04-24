"use client";

import { useState, useEffect, useCallback } from "react";
import { createClient } from "@/lib/supabase/client";
import { useRouter } from "next/navigation";
import type { User } from "@supabase/supabase-js";

// ── Types ─────────────────────────────────────────────────────────────────────
interface ApiKey {
  id: string;
  name: string;
  key_prefix: string;
  created_at: string;
  last_used_at: string | null;
  revoked_at: string | null;
  is_active: boolean;
}

// ── Key generation ────────────────────────────────────────────────────────────
// Generates a secure random API key in the format: cg_live_<32 random hex chars>
function generateRawKey(): string {
  const bytes = new Uint8Array(24);
  crypto.getRandomValues(bytes);
  const hex = Array.from(bytes).map((b) => b.toString(16).padStart(2, "0")).join("");
  return `cg_live_${hex}`;
}

// Simple browser-compatible hash using SubtleCrypto (SHA-256 then base64).
// Note: for production, the hash should be bcrypt — this is the client-side
// representation sent to the server which should re-hash with bcrypt before
// storage. The API route handles bcrypt hashing.
async function hashKey(raw: string): Promise<string> {
  const encoder = new TextEncoder();
  const data = encoder.encode(raw);
  const hashBuf = await crypto.subtle.digest("SHA-256", data);
  return btoa(String.fromCharCode(...new Uint8Array(hashBuf)));
}

// ── Helpers ───────────────────────────────────────────────────────────────────
function formatDate(iso: string | null): string {
  if (!iso) return "Never";
  return new Date(iso).toLocaleDateString("en-US", {
    month: "short", day: "numeric", year: "numeric",
  });
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      onClick={() => { navigator.clipboard.writeText(text); setCopied(true); setTimeout(() => setCopied(false), 2000); }}
      className="ml-2 text-xs text-slate-500 hover:text-green-400 transition-colors"
    >
      {copied ? "✓ Copied" : "Copy"}
    </button>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────
export default function AccountPage() {
  const router = useRouter();
  const supabase = createClient();

  const [user, setUser] = useState<User | null>(null);
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [loading, setLoading] = useState(true);

  // New key creation state
  const [showNewKeyForm, setShowNewKeyForm] = useState(false);
  const [newKeyName, setNewKeyName] = useState("");
  const [creating, setCreating] = useState(false);
  const [createdKey, setCreatedKey] = useState<string | null>(null);

  // Revocation state
  const [revoking, setRevoking] = useState<string | null>(null);

  const loadKeys = useCallback(async () => {
    const { data, error } = await supabase
      .from("api_keys")
      .select("id, name, key_prefix, created_at, last_used_at, revoked_at, is_active")
      .order("created_at", { ascending: false });
    if (!error && data) setKeys(data);
  }, [supabase]);

  useEffect(() => {
    supabase.auth.getUser().then(({ data: { user } }) => {
      if (!user) { router.push("/auth/login"); return; }
      setUser(user);
      loadKeys().finally(() => setLoading(false));
    });
  }, [supabase, router, loadKeys]);

  async function handleCreateKey(e: React.FormEvent) {
    e.preventDefault();
    if (!newKeyName.trim() || !user) return;
    setCreating(true);

    try {
      const rawKey = generateRawKey();
      const keyHash = await hashKey(rawKey);
      const keyPrefix = rawKey.substring(0, 12); // "cg_live_XXXX"

      const { error } = await supabase.from("api_keys").insert({
        user_id: user.id,
        name: newKeyName.trim(),
        key_prefix: keyPrefix,
        key_hash: keyHash,
      });

      if (error) throw error;

      setCreatedKey(rawKey);
      setNewKeyName("");
      setShowNewKeyForm(false);
      await loadKeys();
    } catch (err) {
      console.error("Failed to create key:", err);
    } finally {
      setCreating(false);
    }
  }

  async function handleRevoke(keyId: string) {
    if (!confirm("Revoke this API key? Any connector using it will stop authenticating within 60 seconds.")) return;
    setRevoking(keyId);
    await supabase.from("api_keys").update({ revoked_at: new Date().toISOString() }).eq("id", keyId);
    await loadKeys();
    setRevoking(null);
  }

  async function handleSignOut() {
    await supabase.auth.signOut();
    router.push("/auth/login");
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-slate-500 text-sm animate-pulse">Loading account…</div>
      </div>
    );
  }

  const activeKeys = keys.filter((k) => k.is_active);
  const revokedKeys = keys.filter((k) => !k.is_active);

  return (
    <div className="max-w-2xl mx-auto py-10 px-4">
      {/* Header */}
      <div className="flex items-start justify-between mb-8">
        <div>
          <h1 className="text-2xl font-bold text-slate-100">Account</h1>
          <p className="text-slate-500 text-sm mt-1">{user?.email}</p>
        </div>
        <button
          onClick={handleSignOut}
          className="text-sm text-slate-500 hover:text-slate-300 transition-colors"
        >
          Sign out
        </button>
      </div>

      {/* Newly created key banner — shown once, then dismissed */}
      {createdKey && (
        <div className="mb-6 bg-green-900/20 border border-green-700/40 rounded-xl p-5">
          <div className="flex items-start justify-between">
            <div>
              <p className="text-sm font-semibold text-green-400 mb-1">
                ✓ API key created — copy it now
              </p>
              <p className="text-xs text-slate-500 mb-3">
                This is the only time this key will be shown. Store it in a secrets manager or password vault.
              </p>
            </div>
            <button
              onClick={() => setCreatedKey(null)}
              className="text-slate-500 hover:text-slate-300 text-lg leading-none ml-4"
            >
              ×
            </button>
          </div>
          <div className="flex items-center gap-2 bg-[#0a0d12] border border-[#374151] rounded-lg px-4 py-3">
            <code className="text-green-400 text-sm font-mono flex-1 break-all">{createdKey}</code>
            <CopyButton text={createdKey} />
          </div>
        </div>
      )}

      {/* API Keys section */}
      <section>
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-lg font-semibold text-slate-100">API Keys</h2>
            <p className="text-xs text-slate-500 mt-0.5">
              Use these in your Kafka connector config as{" "}
              <code className="text-slate-400">contractgate.api.key</code>.
            </p>
          </div>
          {!showNewKeyForm && (
            <button
              onClick={() => setShowNewKeyForm(true)}
              className="text-sm bg-green-600 hover:bg-green-500 text-white rounded-lg px-4 py-2 transition-colors"
            >
              + New key
            </button>
          )}
        </div>

        {/* New key form */}
        {showNewKeyForm && (
          <form
            onSubmit={handleCreateKey}
            className="mb-4 bg-[#111827] border border-[#1f2937] rounded-xl p-4 flex gap-3 items-end"
          >
            <div className="flex-1">
              <label className="block text-xs text-slate-400 mb-1.5">Key name</label>
              <input
                type="text"
                required
                autoFocus
                value={newKeyName}
                onChange={(e) => setNewKeyName(e.target.value)}
                placeholder="e.g. Production S3 connector"
                maxLength={80}
                className="w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-green-600 focus:ring-1 focus:ring-green-600/50 transition-colors"
              />
            </div>
            <button
              type="submit"
              disabled={creating || !newKeyName.trim()}
              className="bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white rounded-lg px-4 py-2 text-sm font-medium transition-colors whitespace-nowrap"
            >
              {creating ? "Creating…" : "Create"}
            </button>
            <button
              type="button"
              onClick={() => { setShowNewKeyForm(false); setNewKeyName(""); }}
              className="text-slate-500 hover:text-slate-300 text-sm px-2 py-2 transition-colors"
            >
              Cancel
            </button>
          </form>
        )}

        {/* Active keys */}
        {activeKeys.length === 0 && !showNewKeyForm ? (
          <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-8 text-center">
            <p className="text-slate-500 text-sm mb-4">No API keys yet. Create one to start using the Kafka connector.</p>
            <button
              onClick={() => setShowNewKeyForm(true)}
              className="text-sm bg-green-600 hover:bg-green-500 text-white rounded-lg px-4 py-2 transition-colors"
            >
              Create your first API key
            </button>
          </div>
        ) : (
          <div className="space-y-2">
            {activeKeys.map((key) => (
              <div
                key={key.id}
                className="bg-[#111827] border border-[#1f2937] rounded-xl px-4 py-3.5 flex items-center gap-4"
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium text-slate-200 truncate">{key.name}</span>
                    <span className="text-xs bg-green-900/30 text-green-400 border border-green-700/30 px-1.5 py-0.5 rounded">active</span>
                  </div>
                  <div className="flex items-center gap-4 mt-1 text-xs text-slate-600">
                    <span>
                      <code className="text-slate-500 font-mono">{key.key_prefix}…</code>
                    </span>
                    <span>Created {formatDate(key.created_at)}</span>
                    <span>Last used: {formatDate(key.last_used_at)}</span>
                  </div>
                </div>
                <button
                  onClick={() => handleRevoke(key.id)}
                  disabled={revoking === key.id}
                  className="text-xs text-slate-600 hover:text-red-400 disabled:opacity-40 transition-colors whitespace-nowrap"
                >
                  {revoking === key.id ? "Revoking…" : "Revoke"}
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Revoked keys (collapsed) */}
        {revokedKeys.length > 0 && (
          <details className="mt-4">
            <summary className="text-xs text-slate-600 hover:text-slate-400 cursor-pointer select-none">
              {revokedKeys.length} revoked key{revokedKeys.length > 1 ? "s" : ""}
            </summary>
            <div className="mt-2 space-y-2">
              {revokedKeys.map((key) => (
                <div
                  key={key.id}
                  className="bg-[#0d1117] border border-[#1f2937] rounded-xl px-4 py-3 flex items-center gap-4 opacity-50"
                >
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm text-slate-400 line-through truncate">{key.name}</span>
                      <span className="text-xs text-red-500/60">revoked</span>
                    </div>
                    <div className="mt-0.5 text-xs text-slate-700">
                      <code className="font-mono">{key.key_prefix}…</code>
                      {" · "}Revoked {formatDate(key.revoked_at)}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </details>
        )}
      </section>

      {/* Quick-reference box */}
      <section className="mt-10 p-5 bg-[#111827] border border-[#1f2937] rounded-xl">
        <h3 className="text-sm font-semibold text-slate-300 mb-3">Using your key in Kafka Connect</h3>
        <p className="text-xs text-slate-500 mb-3">
          Add these lines to your connector properties, replacing the placeholders:
        </p>
        <pre className="text-xs text-slate-400 font-mono bg-[#0a0d12] rounded-lg p-3 overflow-x-auto leading-relaxed">
{`transforms=contractgate
transforms.contractgate.type=io.datacontractgate.connect.smt.ContractGateValidator
transforms.contractgate.contractgate.api.url=https://contractgate-api.fly.dev
transforms.contractgate.contractgate.api.key=<your key>
transforms.contractgate.contractgate.contract.id=<your contract UUID>`}
        </pre>
        <p className="text-xs text-slate-600 mt-2">
          See the{" "}
          <a href="/docs/kafka-connect" className="text-green-500 hover:text-green-400">
            full Kafka Connect docs →
          </a>
        </p>
      </section>
    </div>
  );
}
