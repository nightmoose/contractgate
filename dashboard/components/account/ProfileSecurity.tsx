"use client";

// Account page: change display name, change password, delete account.
// Name and password go straight to Supabase Auth (updateUser). Deletion goes
// through /api/account/delete (service role, migration 040).

import { useState } from "react";
import type { User } from "@supabase/supabase-js";
import { createClient } from "@/lib/supabase/client";

const input =
  "w-full bg-[#0a0d12] border border-[#374151] rounded-lg px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-green-600";
const btn =
  "text-xs px-3 py-1.5 rounded bg-[#1f2937] hover:bg-[#374151] text-slate-200 font-medium disabled:opacity-50";

type Msg = { ok: boolean; text: string } | null;

function Note({ msg }: { msg: Msg }) {
  if (!msg) return null;
  return <p className={`text-xs mt-2 ${msg.ok ? "text-green-400" : "text-red-400"}`}>{msg.text}</p>;
}

export default function ProfileSecurity({ user }: { user: User | null }) {
  const supabase = createClient();
  const initialName = (user?.user_metadata?.full_name as string | undefined) ?? "";
  const [name, setName] = useState(initialName);
  const [nameMsg, setNameMsg] = useState<Msg>(null);
  const [savingName, setSavingName] = useState(false);

  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [pwMsg, setPwMsg] = useState<Msg>(null);
  const [savingPw, setSavingPw] = useState(false);

  const [showDelete, setShowDelete] = useState(false);
  const [confirm, setConfirm] = useState("");
  const [delMsg, setDelMsg] = useState<Msg>(null);
  const [deleting, setDeleting] = useState(false);

  async function saveName(e: React.FormEvent) {
    e.preventDefault();
    setSavingName(true);
    setNameMsg(null);
    const { error } = await supabase.auth.updateUser({ data: { full_name: name.trim() } });
    setSavingName(false);
    setNameMsg(error ? { ok: false, text: error.message } : { ok: true, text: "Name saved." });
  }

  async function savePassword(e: React.FormEvent) {
    e.preventDefault();
    setPwMsg(null);
    if (pw.length < 8) return setPwMsg({ ok: false, text: "Password must be at least 8 characters." });
    if (pw !== pw2) return setPwMsg({ ok: false, text: "Passwords do not match." });
    setSavingPw(true);
    const { error } = await supabase.auth.updateUser({ password: pw });
    setSavingPw(false);
    if (error) return setPwMsg({ ok: false, text: error.message });
    setPw("");
    setPw2("");
    setPwMsg({ ok: true, text: "Password changed." });
  }

  async function deleteAccount(e: React.FormEvent) {
    e.preventDefault();
    setDeleting(true);
    setDelMsg(null);
    try {
      const res = await fetch("/api/account/delete", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ confirm }),
      });
      const data = await res.json().catch(() => ({}));
      if (!res.ok) {
        setDelMsg({ ok: false, text: data.error || "Could not delete account." });
        setDeleting(false);
        return;
      }
      await supabase.auth.signOut().catch(() => undefined);
      window.location.assign("/auth/login?deleted=1");
    } catch {
      setDelMsg({ ok: false, text: "Could not delete account." });
      setDeleting(false);
    }
  }

  return (
    <section className="bg-[#111827] border border-[#1f2937] rounded-2xl p-6 mb-8 space-y-6">
      <div className="text-sm font-semibold text-slate-200">Profile &amp; security</div>

      <form onSubmit={saveName}>
        <label htmlFor="acct-name" className="block text-xs text-slate-400 mb-1.5">Name</label>
        <div className="flex gap-2">
          <input id="acct-name" className={input} value={name} maxLength={100}
            onChange={(e) => setName(e.target.value)} placeholder="Your name" />
          <button type="submit" className={btn} disabled={savingName || name.trim() === initialName}>
            {savingName ? "Saving…" : "Save"}
          </button>
        </div>
        <Note msg={nameMsg} />
      </form>

      <form onSubmit={savePassword} className="space-y-2">
        <div className="block text-xs text-slate-400">Change password</div>
        <input type="password" aria-label="New password" autoComplete="new-password" className={input}
          placeholder="New password (8+ characters)" value={pw} onChange={(e) => setPw(e.target.value)} />
        <input type="password" aria-label="Confirm new password" autoComplete="new-password" className={input}
          placeholder="Confirm new password" value={pw2} onChange={(e) => setPw2(e.target.value)} />
        <button type="submit" className={btn} disabled={savingPw || !pw}>
          {savingPw ? "Saving…" : "Change password"}
        </button>
        <Note msg={pwMsg} />
      </form>

      <div className="border-t border-[#1f2937] pt-5">
        <div className="text-xs font-semibold text-red-400 mb-1">Delete account</div>
        <p className="text-xs text-slate-500 mb-3">
          Permanently deletes your sign-in. Organizations where you are the only member are deleted
          with all their contracts, keys and logs, and their subscription is cancelled. If you own an
          organization with other members, make someone else an owner first.
        </p>
        {!showDelete ? (
          <button type="button" className="text-xs px-3 py-1.5 rounded border border-red-700/50 text-red-400 hover:bg-red-900/20"
            onClick={() => setShowDelete(true)}>
            Delete my account…
          </button>
        ) : (
          <form onSubmit={deleteAccount} className="space-y-2">
            <label htmlFor="acct-del" className="block text-xs text-slate-400">
              Type <span className="font-mono text-slate-300">{user?.email}</span> to confirm
            </label>
            <input id="acct-del" className={input} value={confirm} autoComplete="off"
              onChange={(e) => setConfirm(e.target.value)} />
            <div className="flex gap-2">
              <button type="submit"
                className="text-xs px-3 py-1.5 rounded bg-red-700 hover:bg-red-600 text-white font-medium disabled:opacity-50"
                disabled={deleting || confirm.trim().toLowerCase() !== (user?.email ?? "").toLowerCase()}>
                {deleting ? "Deleting…" : "Permanently delete"}
              </button>
              <button type="button" className={btn} onClick={() => { setShowDelete(false); setConfirm(""); }}>
                Cancel
              </button>
            </div>
            <Note msg={delMsg} />
          </form>
        )}
      </div>
    </section>
  );
}
