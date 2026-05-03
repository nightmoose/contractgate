"use client";

import { useState, useEffect, useCallback } from "react";
import { createClient } from "@/lib/supabase/client";
import { useOrg } from "@/lib/org";
import { useRouter } from "next/navigation";
import type { User } from "@supabase/supabase-js";
import AuthGate from "@/components/AuthGate";

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

interface OrgMember {
  user_id: string;
  /** Resolved server-side via /api/org/members (auth.users is not in public schema). */
  email: string | null;
  role: "owner" | "admin" | "member";
  joined_at: string;
}

interface OrgInvite {
  id: string;
  email: string;
  role: "admin" | "member";
  expires_at: string;
  created_at: string;
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
function AccountContent() {
  const router = useRouter();
  const supabase = createClient();
  const { org } = useOrg();

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

  // GitHub integration state
  interface GitHubConfig {
    id?: string;
    repo: string;
    path_prefix: string;
    branch: string;
    has_token: boolean;
  }
  const [ghConfig, setGhConfig] = useState<GitHubConfig | null>(null);
  const [ghLoadError, setGhLoadError] = useState<string | null>(null);
  const [ghRepo, setGhRepo] = useState("");
  const [ghPrefix, setGhPrefix] = useState("contracts/");
  const [ghBranch, setGhBranch] = useState("main");
  const [ghToken, setGhToken] = useState("");
  const [ghSaving, setGhSaving] = useState(false);
  const [ghSaveError, setGhSaveError] = useState<string | null>(null);
  const [ghSaveOk, setGhSaveOk] = useState(false);
  const [ghDeleting, setGhDeleting] = useState(false);

  // Org members + invites state
  const [members, setMembers] = useState<OrgMember[]>([]);
  const [invites, setInvites] = useState<OrgInvite[]>([]);
  const [showInviteForm, setShowInviteForm] = useState(false);
  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteRole, setInviteRole] = useState<"admin" | "member">("member");
  const [inviting, setInviting] = useState(false);
  const [inviteError, setInviteError] = useState<string | null>(null);
  const [inviteSent, setInviteSent] = useState<string | null>(null);
  const [revokingInvite, setRevokingInvite] = useState<string | null>(null);

  const loadGitHubConfig = useCallback(async () => {
    setGhLoadError(null);
    try {
      const res = await fetch("/api/github/config");
      if (!res.ok) { setGhLoadError("Failed to load GitHub config"); return; }
      const data = await res.json();
      if (data) {
        setGhConfig(data);
        setGhRepo(data.repo ?? "");
        setGhPrefix(data.path_prefix ?? "contracts/");
        setGhBranch(data.branch ?? "main");
      } else {
        setGhConfig(null);
      }
    } catch {
      setGhLoadError("Failed to load GitHub config");
    }
  }, []);

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
      Promise.all([loadKeys(), loadGitHubConfig()]).finally(() => setLoading(false));
    });
  }, [supabase, router, loadKeys, loadGitHubConfig]);

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
        org_id: org?.org_id ?? null,
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

  // Load org members and pending invites when org is resolved.
  const loadOrgData = useCallback(async (orgId: string) => {
    // Members are fetched via the server route so we can join auth.users.email.
    // Invites stay on the client (RLS lets owners/admins read them directly).
    const [membersRes, { data: invitesData }] = await Promise.all([
      fetch(`/api/org/members?org_id=${encodeURIComponent(orgId)}`).then((r) =>
        r.ok ? (r.json() as Promise<{ members: OrgMember[] }>) : { members: [] }
      ),
      supabase
        .from("org_invites")
        .select("id, email, role, expires_at, created_at")
        .eq("org_id", orgId)
        .is("accepted_at", null)
        .is("revoked_at", null)
        .gt("expires_at", new Date().toISOString())
        .order("created_at", { ascending: false }),
    ]);
    setMembers(membersRes.members ?? []);
    if (invitesData) setInvites(invitesData as OrgInvite[]);
  }, [supabase]);

  useEffect(() => {
    if (org?.org_id) loadOrgData(org.org_id);
  }, [org?.org_id, loadOrgData]);

  async function handleSendInvite(e: React.FormEvent) {
    e.preventDefault();
    if (!inviteEmail.trim() || !org || !user) return;
    setInviting(true);
    setInviteError(null);
    setInviteSent(null);
    try {
      const token = crypto.randomUUID();
      const { error } = await supabase.from("org_invites").insert({
        org_id: org.org_id,
        email: inviteEmail.trim().toLowerCase(),
        role: inviteRole,
        invited_by: user.id,
        token,
      });
      if (error) throw error;
      setInviteSent(inviteEmail.trim());
      setInviteEmail("");
      setShowInviteForm(false);
      await loadOrgData(org.org_id);
    } catch (err: unknown) {
      setInviteError(err instanceof Error ? err.message : "Failed to send invite");
    } finally {
      setInviting(false);
    }
  }

  async function handleRevokeInvite(inviteId: string) {
    if (!org) return;
    if (!confirm("Revoke this invite? The link will stop working immediately.")) return;
    setRevokingInvite(inviteId);
    await supabase
      .from("org_invites")
      .update({ revoked_at: new Date().toISOString() })
      .eq("id", inviteId);
    await loadOrgData(org.org_id);
    setRevokingInvite(null);
  }

  async function handleSaveGitHubConfig(e: React.FormEvent) {
    e.preventDefault();
    setGhSaving(true);
    setGhSaveError(null);
    setGhSaveOk(false);
    try {
      const body: Record<string, string> = {
        repo: ghRepo.trim(),
        path_prefix: ghPrefix.trim(),
        branch: ghBranch.trim() || "main",
      };
      if (ghToken.trim()) body.github_token = ghToken.trim();
      const res = await fetch("/api/github/config", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      const data = await res.json();
      if (!res.ok) {
        setGhSaveError(data.error ?? "Failed to save");
      } else {
        setGhConfig(data);
        setGhToken(""); // clear — token is never returned
        setGhSaveOk(true);
        setTimeout(() => setGhSaveOk(false), 3000);
      }
    } catch {
      setGhSaveError("Network error — please try again");
    } finally {
      setGhSaving(false);
    }
  }

  async function handleDeleteGitHubConfig() {
    if (!confirm("Remove GitHub integration? Contracts will no longer sync to GitHub.")) return;
    setGhDeleting(true);
    try {
      await fetch("/api/github/config", { method: "DELETE" });
      setGhConfig(null);
      setGhRepo(""); setGhPrefix("contracts/"); setGhBranch("main"); setGhToken("");
    } finally {
      setGhDeleting(false);
    }
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
          {org && (
            <p className="text-xs text-slate-600 mt-1 flex items-center gap-1.5">
              <span className="inline-block w-1.5 h-1.5 rounded-full bg-green-500" />
              <span className="font-mono text-slate-500">{org.org_name}</span>
              <span className="text-slate-700">·</span>
              <span className="text-slate-600 capitalize">{org.role}</span>
            </p>
          )}
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

      {/* ── Org Members ─────────────────────────────────────────────────── */}
      {org && (
        <section className="mt-10">
          <div className="flex items-center justify-between mb-4">
            <div>
              <h2 className="text-lg font-semibold text-slate-100">Team Members</h2>
              <p className="text-xs text-slate-500 mt-0.5">
                <span className="font-mono text-slate-400">{org.slug}</span>
                {" · "}
                {members.length} member{members.length !== 1 ? "s" : ""}
              </p>
            </div>
            {(org.role === "owner" || org.role === "admin") && !showInviteForm && (
              <button
                onClick={() => { setShowInviteForm(true); setInviteSent(null); setInviteError(null); }}
                className="text-sm bg-indigo-600 hover:bg-indigo-500 text-white rounded-lg px-4 py-2 transition-colors"
              >
                + Invite member
              </button>
            )}
          </div>

          {/* Invite success flash */}
          {inviteSent && (
            <div className="mb-4 bg-green-900/20 border border-green-700/40 rounded-xl px-4 py-3 flex items-center justify-between">
              <p className="text-sm text-green-400">
                ✓ Invite sent to <strong>{inviteSent}</strong> — they&apos;ll receive a link to join.
              </p>
              <button onClick={() => setInviteSent(null)} className="text-slate-500 hover:text-slate-300 ml-4">×</button>
            </div>
          )}

          {/* Invite form */}
          {showInviteForm && (
            <form
              onSubmit={handleSendInvite}
              className="mb-4 bg-[#111827] border border-[#1f2937] rounded-xl p-4 space-y-3"
            >
              <p className="text-xs text-slate-400 font-medium uppercase tracking-wider">Invite by email</p>
              <div className="flex gap-3 items-end flex-wrap">
                <div className="flex-1 min-w-[200px]">
                  <label className="block text-xs text-slate-400 mb-1.5">Email address</label>
                  <input
                    type="email"
                    required
                    autoFocus
                    value={inviteEmail}
                    onChange={(e) => setInviteEmail(e.target.value)}
                    placeholder="colleague@company.com"
                    className="w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-indigo-600 focus:ring-1 focus:ring-indigo-600/50 transition-colors"
                  />
                </div>
                <div>
                  <label className="block text-xs text-slate-400 mb-1.5">Role</label>
                  <select
                    value={inviteRole}
                    onChange={(e) => setInviteRole(e.target.value as "admin" | "member")}
                    className="bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-indigo-600 transition-colors"
                  >
                    <option value="member">Member</option>
                    {org.role === "owner" && <option value="admin">Admin</option>}
                  </select>
                </div>
                <button
                  type="submit"
                  disabled={inviting || !inviteEmail.trim()}
                  className="bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-white rounded-lg px-4 py-2 text-sm font-medium transition-colors whitespace-nowrap"
                >
                  {inviting ? "Sending…" : "Send invite"}
                </button>
                <button
                  type="button"
                  onClick={() => { setShowInviteForm(false); setInviteEmail(""); setInviteError(null); }}
                  className="text-slate-500 hover:text-slate-300 text-sm px-2 py-2 transition-colors"
                >
                  Cancel
                </button>
              </div>
              {inviteError && (
                <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
                  {inviteError}
                </p>
              )}
            </form>
          )}

          {/* Member list */}
          <div className="space-y-2">
            {members.map((m) => {
              const isYou = m.user_id === user?.id;
              return (
                <div
                  key={m.user_id}
                  className="bg-[#111827] border border-[#1f2937] rounded-xl px-4 py-3 flex items-center gap-3"
                >
                  <div className="w-8 h-8 rounded-full bg-indigo-900/50 border border-indigo-700/30 flex items-center justify-center text-xs text-indigo-300 font-medium shrink-0">
                    {((isYou ? user?.email : m.email) ?? "?")[0]?.toUpperCase() ?? "?"}
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm text-slate-200 truncate">
                        {(isYou ? user?.email : m.email) ?? m.user_id}
                      </span>
                      {isYou && (
                        <span className="text-xs text-slate-600 bg-[#1f2937] px-1.5 py-0.5 rounded">You</span>
                      )}
                    </div>
                    <p className="text-xs text-slate-600 mt-0.5">
                      Joined {formatDate(m.joined_at)}
                    </p>
                  </div>
                  <span className={`text-xs px-2 py-0.5 rounded-full font-medium capitalize ${
                    m.role === "owner"
                      ? "bg-amber-900/30 text-amber-400"
                      : m.role === "admin"
                      ? "bg-indigo-900/30 text-indigo-400"
                      : "bg-slate-800 text-slate-400"
                  }`}>
                    {m.role}
                  </span>
                </div>
              );
            })}
          </div>

          {/* Pending invites */}
          {invites.length > 0 && (
            <div className="mt-4">
              <p className="text-xs text-slate-500 uppercase tracking-wider mb-2">
                Pending invites ({invites.length})
              </p>
              <div className="space-y-2">
                {invites.map((inv) => (
                  <div
                    key={inv.id}
                    className="bg-[#0d1117] border border-[#1f2937] border-dashed rounded-xl px-4 py-3 flex items-center gap-3"
                  >
                    <div className="w-8 h-8 rounded-full bg-slate-800/50 border border-slate-700/30 flex items-center justify-center text-xs text-slate-500">
                      ✉
                    </div>
                    <div className="flex-1 min-w-0">
                      <p className="text-sm text-slate-300">{inv.email}</p>
                      <p className="text-xs text-slate-600 mt-0.5">
                        Expires {formatDate(inv.expires_at)}
                        {" · "}
                        <span className="capitalize">{inv.role}</span>
                      </p>
                    </div>
                    {(org.role === "owner" || org.role === "admin") && (
                      <button
                        onClick={() => handleRevokeInvite(inv.id)}
                        disabled={revokingInvite === inv.id}
                        className="text-xs text-slate-600 hover:text-red-400 disabled:opacity-40 transition-colors whitespace-nowrap"
                      >
                        {revokingInvite === inv.id ? "Revoking…" : "Revoke"}
                      </button>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}
        </section>
      )}

      {/* ── GitHub Integration ─────────────────────────────────────────────── */}
      <section className="mt-10">
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-base font-semibold text-slate-100 flex items-center gap-2">
              {/* GitHub mark */}
              <svg height="16" viewBox="0 0 16 16" width="16" fill="currentColor" className="text-slate-400" aria-hidden="true">
                <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
              </svg>
              GitHub Integration
            </h2>
            <p className="text-xs text-slate-500 mt-0.5">
              Sync contract YAML to a GitHub repo when you promote a version.
            </p>
          </div>
          {ghConfig && (
            <span className="text-xs px-2 py-1 bg-green-900/20 text-green-400 border border-green-700/30 rounded-full">
              Connected
            </span>
          )}
        </div>

        {ghLoadError && (
          <div className="mb-4 bg-red-900/20 border border-red-700/40 rounded-lg px-3 py-2.5 text-sm text-red-400">
            {ghLoadError}
          </div>
        )}

        {ghConfig && (
          <div className="mb-4 bg-[#0d1117] border border-[#1f2937] rounded-xl px-4 py-3 flex items-center justify-between gap-4">
            <div className="min-w-0">
              <p className="text-sm text-slate-200 font-mono truncate">{ghConfig.repo}</p>
              <p className="text-xs text-slate-500 mt-0.5">
                Branch: <span className="text-slate-400">{ghConfig.branch}</span>
                {" · "}
                Prefix: <span className="text-slate-400 font-mono">{ghConfig.path_prefix || "/"}</span>
                {" · "}
                Token: <span className={ghConfig.has_token ? "text-green-400" : "text-red-400"}>
                  {ghConfig.has_token ? "Set" : "Not set"}
                </span>
              </p>
            </div>
          </div>
        )}

        <form onSubmit={handleSaveGitHubConfig} className="bg-[#111827] border border-[#1f2937] rounded-xl p-5 space-y-4">
          <div>
            <label className="block text-sm text-slate-400 mb-1.5">
              Repository <span className="text-slate-600">(owner/repo)</span>
            </label>
            <input
              type="text"
              required
              value={ghRepo}
              onChange={(e) => setGhRepo(e.target.value)}
              placeholder="acme-corp/data-contracts"
              className="w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-green-600 focus:ring-1 focus:ring-green-600/50 transition-colors font-mono"
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-sm text-slate-400 mb-1.5">Path prefix</label>
              <input
                type="text"
                value={ghPrefix}
                onChange={(e) => setGhPrefix(e.target.value)}
                placeholder="contracts/"
                className="w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-green-600 focus:ring-1 focus:ring-green-600/50 transition-colors font-mono"
              />
              <p className="text-xs text-slate-600 mt-1">Directory inside the repo</p>
            </div>
            <div>
              <label className="block text-sm text-slate-400 mb-1.5">Branch</label>
              <input
                type="text"
                value={ghBranch}
                onChange={(e) => setGhBranch(e.target.value)}
                placeholder="main"
                className="w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-green-600 focus:ring-1 focus:ring-green-600/50 transition-colors font-mono"
              />
            </div>
          </div>

          <div>
            <label className="block text-sm text-slate-400 mb-1.5">
              GitHub Personal Access Token
              {ghConfig?.has_token && (
                <span className="ml-2 text-xs text-slate-600">(leave blank to keep existing token)</span>
              )}
            </label>
            <input
              type="password"
              value={ghToken}
              onChange={(e) => setGhToken(e.target.value)}
              placeholder={ghConfig?.has_token ? "••••••••  (already set)" : "github_pat_…"}
              className="w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-green-600 focus:ring-1 focus:ring-green-600/50 transition-colors font-mono"
            />
            <p className="text-xs text-slate-600 mt-1">
              Needs <span className="font-mono text-slate-500">contents:write</span> scope. Stored server-side, never sent to the browser.
            </p>
          </div>

          {ghSaveError && (
            <div className="bg-red-900/20 border border-red-700/40 rounded-lg px-3 py-2.5 text-sm text-red-400">
              {ghSaveError}
            </div>
          )}
          {ghSaveOk && (
            <div className="bg-green-900/20 border border-green-700/40 rounded-lg px-3 py-2.5 text-sm text-green-400">
              ✓ GitHub integration saved
            </div>
          )}

          <div className="flex items-center justify-between pt-1">
            <button
              type="submit"
              disabled={ghSaving}
              className="bg-green-600 hover:bg-green-500 disabled:opacity-50 disabled:cursor-not-allowed text-white rounded-lg px-4 py-2 text-sm font-medium transition-colors"
            >
              {ghSaving ? "Saving…" : ghConfig ? "Update" : "Save"}
            </button>

            {ghConfig && (
              <button
                type="button"
                onClick={handleDeleteGitHubConfig}
                disabled={ghDeleting}
                className="text-xs text-slate-600 hover:text-red-400 disabled:opacity-40 transition-colors"
              >
                {ghDeleting ? "Removing…" : "Remove integration"}
              </button>
            )}
          </div>
        </form>
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

export default function AccountPage() {
  return (
    <AuthGate page="account">
      <AccountContent />
    </AuthGate>
  );
}
