"use client";

/**
 * Collaborate tab — RFC-033 Provider-Consumer Collaboration.
 *
 * Three panels:
 *   1. Collaborators — invite/manage orgs with role picker (owner/editor/reviewer/viewer)
 *   2. Comments      — threaded notes, optionally anchored to a field, resolvable
 *   3. Proposals     — change proposals create/review/decide/apply flow
 *
 * Role enforcement: the API enforces permissions; this UI shows/hides write
 * controls based on the `userRole` prop. Viewers see everything but cannot write.
 */

import { useState, useEffect } from "react";
import clsx from "clsx";
import {
  listCollaborators,
  grantCollaborator,
  patchCollaborator,
  revokeCollaborator,
  listComments,
  addComment,
  resolveComment,
  listProposals,
  createProposal,
  decideProposal,
  applyProposal,
} from "@/lib/api";
import type {
  CollaboratorRow,
  CollaboratorRole,
  CommentRow,
  ProposalRow,
} from "@/lib/api";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type AnyRole = CollaboratorRole | "owner";

interface CollaborateTabProps {
  /** The contract's display name (used for all API calls). */
  contractName: string;
  /** Caller's inferred role. Defaults to "owner" (owner sees all controls). */
  userRole?: AnyRole;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function fmtDate(iso: string) {
  return new Date(iso).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function roleBadge(role: AnyRole) {
  const map: Record<AnyRole, string> = {
    owner: "bg-green-900/40 text-green-400 border-green-800/40",
    editor: "bg-indigo-900/40 text-indigo-300 border-indigo-800/40",
    reviewer: "bg-amber-900/30 text-amber-300 border-amber-700/40",
    viewer: "bg-slate-800 text-slate-400 border-slate-700",
  };
  return (
    <span
      className={clsx(
        "text-[10px] uppercase tracking-wider border rounded px-2 py-0.5 font-medium",
        map[role] ?? "bg-slate-800 text-slate-400"
      )}
    >
      {role}
    </span>
  );
}

function proposalStatusBadge(status: ProposalRow["status"]) {
  const map = {
    open: "bg-blue-900/30 text-blue-300 border-blue-800/40",
    approved: "bg-green-900/40 text-green-400 border-green-800/40",
    rejected: "bg-red-900/30 text-red-400 border-red-800/40",
    applied: "bg-teal-900/30 text-teal-300 border-teal-800/40",
  };
  return (
    <span
      className={clsx(
        "text-[10px] uppercase tracking-wider border rounded px-2 py-0.5 font-medium",
        map[status]
      )}
    >
      {status}
    </span>
  );
}

// ---------------------------------------------------------------------------
// Collaborators panel
// ---------------------------------------------------------------------------

function CollaboratorsPanel({
  contractName,
  userRole,
}: {
  contractName: string;
  userRole: AnyRole;
}) {
  const [collaborators, setCollaborators] = useState<CollaboratorRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  // Invite form
  const [inviteOrgId, setInviteOrgId] = useState("");
  const [inviteRole, setInviteRole] = useState<CollaboratorRole>("viewer");
  const [inviting, setInviting] = useState(false);
  const [inviteErr, setInviteErr] = useState<string | null>(null);

  const isOwner = userRole === "owner";

  const load = () => {
    setLoading(true);
    listCollaborators(contractName)
      .then(setCollaborators)
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  };

  useEffect(() => { load(); }, [contractName]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleInvite = async () => {
    if (!inviteOrgId.trim()) { setInviteErr("Org ID is required."); return; }
    setInviting(true); setInviteErr(null);
    try {
      await grantCollaborator(contractName, { org_id: inviteOrgId.trim(), role: inviteRole });
      setInviteOrgId("");
      load();
    } catch (e) {
      setInviteErr(e instanceof Error ? e.message : String(e));
    } finally { setInviting(false); }
  };

  const handleChangeRole = async (orgId: string, newRole: CollaboratorRole) => {
    try {
      await patchCollaborator(contractName, orgId, newRole);
      load();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };

  const handleRevoke = async (orgId: string) => {
    if (!confirm("Revoke this collaborator's access?")) return;
    try {
      await revokeCollaborator(contractName, orgId);
      load();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-semibold text-slate-300">Collaborators</h3>
        <span className="text-xs text-slate-600">{collaborators.length} grant{collaborators.length !== 1 ? "s" : ""}</span>
      </div>

      {loading ? (
        <p className="text-xs text-slate-500">Loading…</p>
      ) : err ? (
        <p className="text-xs text-red-400">{err}</p>
      ) : collaborators.length === 0 ? (
        <p className="text-xs text-slate-600 italic">No collaborators yet.</p>
      ) : (
        <div className="space-y-2 mb-4">
          {collaborators.map((c) => (
            <div
              key={c.org_id}
              className="flex items-center justify-between bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-2"
            >
              <div className="min-w-0 flex-1 mr-3">
                <p className="text-xs font-mono text-slate-300 truncate">{c.org_id}</p>
                <p className="text-[10px] text-slate-600 mt-0.5">
                  granted {fmtDate(c.granted_at)}
                </p>
              </div>
              <div className="flex items-center gap-2 shrink-0">
                {isOwner ? (
                  <select
                    value={c.role}
                    onChange={(e) => handleChangeRole(c.org_id, e.target.value as CollaboratorRole)}
                    className="text-xs bg-[#1f2937] border border-[#374151] text-slate-300 rounded px-2 py-1 outline-none"
                  >
                    <option value="editor">editor</option>
                    <option value="reviewer">reviewer</option>
                    <option value="viewer">viewer</option>
                  </select>
                ) : (
                  roleBadge(c.role)
                )}
                {isOwner && (
                  <button
                    onClick={() => handleRevoke(c.org_id)}
                    className="text-xs text-red-400 hover:text-red-300 transition-colors px-2 py-1 rounded hover:bg-red-900/20"
                    title="Revoke access"
                  >
                    ✕
                  </button>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Invite form — owner only */}
      {isOwner && (
        <div className="border border-dashed border-[#2d3748] rounded-lg p-4 space-y-3">
          <p className="text-xs font-medium text-slate-400 uppercase tracking-wider">
            Invite Collaborator
          </p>
          <p className="text-xs text-slate-600">
            Grant a scoped role to another org — they can only see this contract, nothing else in your org.
          </p>
          <div className="flex gap-2 flex-wrap">
            <input
              type="text"
              value={inviteOrgId}
              onChange={(e) => setInviteOrgId(e.target.value)}
              placeholder="Org UUID"
              className="flex-1 min-w-[200px] bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-1.5 text-xs text-slate-200 font-mono placeholder-slate-600 outline-none focus:border-indigo-600 transition-colors"
            />
            <select
              value={inviteRole}
              onChange={(e) => setInviteRole(e.target.value as CollaboratorRole)}
              className="bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-1.5 text-xs text-slate-300 outline-none"
            >
              <option value="editor">editor</option>
              <option value="reviewer">reviewer</option>
              <option value="viewer">viewer</option>
            </select>
            <button
              onClick={handleInvite}
              disabled={inviting}
              className="px-3 py-1.5 bg-indigo-700 hover:bg-indigo-600 disabled:opacity-40 text-white text-xs font-medium rounded-lg transition-colors"
            >
              {inviting ? "Inviting…" : "Grant access"}
            </button>
          </div>
          {inviteErr && (
            <p className="text-xs text-red-400">✕ {inviteErr}</p>
          )}

          {/* Role description table */}
          <div className="grid grid-cols-1 gap-1 mt-1">
            {(
              [
                ["editor", "Propose changes, comment, read. Cannot publish or manage collaborators."],
                ["reviewer", "Approve/reject proposals, comment, read. Cannot edit or publish."],
                ["viewer", "Read the contract definition and comments only. Cannot write anything."],
              ] as const
            ).map(([role, desc]) => (
              <div key={role} className="flex items-start gap-2 text-[10px] text-slate-600">
                <span className="shrink-0 w-14">{roleBadge(role)}</span>
                <span className="leading-relaxed">{desc}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Comments panel
// ---------------------------------------------------------------------------

function CommentsPanel({
  contractName,
  userRole,
}: {
  contractName: string;
  userRole: AnyRole;
}) {
  const [comments, setComments] = useState<CommentRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  // New comment form
  const [author, setAuthor] = useState("");
  const [body, setBody] = useState("");
  const [field, setField] = useState("");
  const [posting, setPosting] = useState(false);
  const [postErr, setPostErr] = useState<string | null>(null);

  const canComment = userRole !== "viewer"; // viewer = read-only

  const load = () => {
    setLoading(true);
    listComments(contractName)
      .then(setComments)
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  };

  useEffect(() => { load(); }, [contractName]); // eslint-disable-line react-hooks/exhaustive-deps

  const handlePost = async () => {
    if (!body.trim()) { setPostErr("Comment body is required."); return; }
    if (!author.trim()) { setPostErr("Author name/email is required."); return; }
    setPosting(true); setPostErr(null);
    try {
      await addComment(contractName, {
        author: author.trim(),
        body: body.trim(),
        ...(field.trim() ? { field: field.trim() } : {}),
      });
      setBody("");
      load();
    } catch (e) {
      setPostErr(e instanceof Error ? e.message : String(e));
    } finally { setPosting(false); }
  };

  const handleResolve = async (commentId: string) => {
    try {
      await resolveComment(contractName, commentId);
      load();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };

  const open = comments.filter((c) => !c.resolved);
  const resolved = comments.filter((c) => c.resolved);

  return (
    <div>
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-semibold text-slate-300">Comments</h3>
        <span className="text-xs text-slate-600">
          {open.length} open{resolved.length > 0 ? `, ${resolved.length} resolved` : ""}
        </span>
      </div>

      {loading ? (
        <p className="text-xs text-slate-500">Loading…</p>
      ) : err ? (
        <p className="text-xs text-red-400">{err}</p>
      ) : (
        <div className="space-y-2 mb-4">
          {open.length === 0 && (
            <p className="text-xs text-slate-600 italic">No open comments.</p>
          )}
          {open.map((c) => (
            <div
              key={c.id}
              className="bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-2.5"
            >
              <div className="flex items-start justify-between gap-2">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap mb-1">
                    <span className="text-xs font-medium text-slate-300">{c.author}</span>
                    {c.field && (
                      <span className="text-[10px] bg-sky-900/30 text-sky-400 border border-sky-800/40 rounded px-1.5 py-0.5 font-mono">
                        field: {c.field}
                      </span>
                    )}
                    <span className="text-[10px] text-slate-600">{fmtDate(c.created_at)}</span>
                  </div>
                  <p className="text-xs text-slate-400 leading-relaxed">{c.body}</p>
                </div>
                {canComment && (
                  <button
                    onClick={() => handleResolve(c.id)}
                    className="shrink-0 text-[10px] text-slate-600 hover:text-green-400 transition-colors px-2 py-1 rounded hover:bg-green-900/10 whitespace-nowrap"
                    title="Mark resolved"
                  >
                    ✓ resolve
                  </button>
                )}
              </div>
            </div>
          ))}
          {resolved.length > 0 && (
            <details className="mt-2">
              <summary className="text-[10px] text-slate-600 cursor-pointer hover:text-slate-400 select-none">
                {resolved.length} resolved comment{resolved.length !== 1 ? "s" : ""}
              </summary>
              <div className="mt-2 space-y-2 opacity-60">
                {resolved.map((c) => (
                  <div key={c.id} className="bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-2">
                    <div className="flex items-center gap-2 mb-0.5">
                      <span className="text-[10px] text-slate-500">{c.author}</span>
                      {c.field && (
                        <span className="text-[10px] text-sky-700 font-mono">#{c.field}</span>
                      )}
                      <span className="text-[10px] text-slate-700">{fmtDate(c.created_at)}</span>
                      <span className="text-[10px] text-green-700">✓ resolved</span>
                    </div>
                    <p className="text-[10px] text-slate-600 leading-relaxed">{c.body}</p>
                  </div>
                ))}
              </div>
            </details>
          )}
        </div>
      )}

      {/* New comment form */}
      {canComment ? (
        <div className="border border-dashed border-[#2d3748] rounded-lg p-4 space-y-2">
          <p className="text-xs font-medium text-slate-400 uppercase tracking-wider">
            Add Comment
          </p>
          <div className="flex gap-2 flex-wrap">
            <input
              type="text"
              value={author}
              onChange={(e) => setAuthor(e.target.value)}
              placeholder="Your name or email"
              className="flex-1 min-w-[160px] bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-1.5 text-xs text-slate-200 placeholder-slate-600 outline-none focus:border-indigo-600 transition-colors"
            />
            <input
              type="text"
              value={field}
              onChange={(e) => setField(e.target.value)}
              placeholder="Field (optional)"
              className="w-36 bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-1.5 text-xs text-slate-200 font-mono placeholder-slate-600 outline-none focus:border-sky-600 transition-colors"
            />
          </div>
          <textarea
            value={body}
            onChange={(e) => setBody(e.target.value)}
            placeholder="What's on your mind about this contract?"
            rows={3}
            className="w-full bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-2 text-xs text-slate-200 placeholder-slate-600 outline-none focus:border-indigo-600 transition-colors resize-y"
          />
          {postErr && <p className="text-xs text-red-400">✕ {postErr}</p>}
          <button
            onClick={handlePost}
            disabled={posting}
            className="px-3 py-1.5 bg-indigo-700 hover:bg-indigo-600 disabled:opacity-40 text-white text-xs font-medium rounded-lg transition-colors"
          >
            {posting ? "Posting…" : "Post comment"}
          </button>
        </div>
      ) : (
        <p className="text-xs text-slate-600 italic">Viewers can read but not post comments.</p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Proposals panel
// ---------------------------------------------------------------------------

function ProposalsPanel({
  contractName,
  contractCurrentYaml,
  userRole,
  onApplied,
}: {
  contractName: string;
  contractCurrentYaml: string;
  userRole: AnyRole;
  /** Called when an approved proposal is applied — lets the parent refresh the YAML. */
  onApplied?: (appliedYaml: string) => void;
}) {
  const [proposals, setProposals] = useState<ProposalRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  // Create proposal form
  const [creating, setCreating] = useState(false);
  const [proposalYaml, setProposalYaml] = useState(contractCurrentYaml);
  const [submitting, setSubmitting] = useState(false);
  const [submitErr, setSubmitErr] = useState<string | null>(null);

  // Expand a proposal to see diff
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const canPropose = userRole === "owner" || userRole === "editor";
  const canDecide = userRole === "owner" || userRole === "reviewer";
  const canApply = userRole === "owner";

  const load = () => {
    setLoading(true);
    listProposals(contractName)
      .then(setProposals)
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  };

  useEffect(() => { load(); }, [contractName]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleSubmit = async () => {
    if (!proposalYaml.trim()) { setSubmitErr("Proposal YAML is required."); return; }
    setSubmitting(true); setSubmitErr(null);
    try {
      await createProposal(contractName, proposalYaml.trim());
      setCreating(false);
      load();
    } catch (e) {
      setSubmitErr(e instanceof Error ? e.message : String(e));
    } finally { setSubmitting(false); }
  };

  const handleDecide = async (proposalId: string, decision: "approved" | "rejected") => {
    try {
      await decideProposal(contractName, proposalId, decision);
      load();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };

  const handleApply = async (proposalId: string, proposedYaml: string) => {
    try {
      const updated = await applyProposal(contractName, proposalId);
      onApplied?.(updated.proposed_yaml ?? proposedYaml);
      load();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-semibold text-slate-300">Change Proposals</h3>
        <div className="flex items-center gap-2">
          <span className="text-xs text-slate-600">
            {proposals.filter((p) => p.status === "open").length} open
          </span>
          {canPropose && (
            <button
              onClick={() => { setCreating(true); setProposalYaml(contractCurrentYaml); }}
              className="text-xs px-2.5 py-1 bg-indigo-900/40 hover:bg-indigo-900/60 text-indigo-300 border border-indigo-800/40 rounded-lg transition-colors"
            >
              + New proposal
            </button>
          )}
        </div>
      </div>

      {/* Create proposal form */}
      {creating && (
        <div className="mb-4 border border-dashed border-indigo-800/40 rounded-lg p-4 space-y-3">
          <p className="text-xs font-medium text-indigo-300 uppercase tracking-wider">
            New Change Proposal
          </p>
          <p className="text-xs text-slate-500 leading-relaxed">
            Paste the full proposed YAML. This creates an open proposal that the
            owner or a reviewer can approve or reject. Your edit never lands directly
            on a stable version.
          </p>
          <textarea
            value={proposalYaml}
            onChange={(e) => setProposalYaml(e.target.value)}
            rows={12}
            className="w-full bg-[#0d1117] text-green-300 font-mono text-xs p-3 rounded-lg border border-[#1f2937] outline-none focus:border-indigo-600 resize-y transition-colors"
            spellCheck={false}
          />
          {submitErr && <p className="text-xs text-red-400">✕ {submitErr}</p>}
          <div className="flex gap-2">
            <button
              onClick={handleSubmit}
              disabled={submitting}
              className="px-3 py-1.5 bg-indigo-700 hover:bg-indigo-600 disabled:opacity-40 text-white text-xs font-medium rounded-lg transition-colors"
            >
              {submitting ? "Submitting…" : "Submit proposal"}
            </button>
            <button
              onClick={() => setCreating(false)}
              className="px-3 py-1.5 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-xs font-medium rounded-lg transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {loading ? (
        <p className="text-xs text-slate-500">Loading…</p>
      ) : err ? (
        <p className="text-xs text-red-400">{err}</p>
      ) : proposals.length === 0 ? (
        <p className="text-xs text-slate-600 italic">No proposals yet.</p>
      ) : (
        <div className="space-y-2">
          {proposals.map((p) => (
            <div
              key={p.id}
              className="bg-[#0d1117] border border-[#1f2937] rounded-lg overflow-hidden"
            >
              <div className="flex items-start justify-between gap-2 px-3 py-2.5">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap mb-0.5">
                    {proposalStatusBadge(p.status)}
                    <span className="text-[10px] text-slate-600">
                      {fmtDate(p.created_at)} · by {p.proposed_by.slice(0, 8)}…
                    </span>
                    {p.decided_by && (
                      <span className="text-[10px] text-slate-600">
                        decided by {p.decided_by.slice(0, 8)}…
                      </span>
                    )}
                  </div>
                  <button
                    onClick={() => setExpandedId(expandedId === p.id ? null : p.id)}
                    className="text-xs text-slate-500 hover:text-slate-300 transition-colors mt-1"
                  >
                    {expandedId === p.id ? "▲ Hide YAML" : "▼ View proposed YAML"}
                  </button>
                </div>

                {/* Action buttons */}
                <div className="flex items-center gap-1.5 shrink-0 flex-wrap">
                  {p.status === "open" && canDecide && (
                    <>
                      <button
                        onClick={() => handleDecide(p.id, "approved")}
                        className="text-xs px-2.5 py-1 bg-green-900/30 hover:bg-green-900/50 text-green-400 border border-green-800/40 rounded-lg transition-colors"
                      >
                        ✓ Approve
                      </button>
                      <button
                        onClick={() => handleDecide(p.id, "rejected")}
                        className="text-xs px-2.5 py-1 bg-red-900/20 hover:bg-red-900/40 text-red-400 border border-red-800/30 rounded-lg transition-colors"
                      >
                        ✕ Reject
                      </button>
                    </>
                  )}
                  {p.status === "approved" && canApply && (
                    <button
                      onClick={() => handleApply(p.id, p.proposed_yaml)}
                      className="text-xs px-2.5 py-1 bg-teal-900/30 hover:bg-teal-900/50 text-teal-300 border border-teal-800/40 rounded-lg transition-colors"
                      title="Mark applied and use proposed YAML as the new version"
                    >
                      ⬆ Apply
                    </button>
                  )}
                </div>
              </div>

              {expandedId === p.id && (
                <div className="border-t border-[#1f2937] px-3 py-2">
                  <pre className="text-[10px] text-green-300 font-mono overflow-auto max-h-48 leading-relaxed whitespace-pre-wrap">
                    {p.proposed_yaml}
                  </pre>
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {!canPropose && proposals.length === 0 && (
        <p className="text-xs text-slate-600 italic mt-2">
          Only editors and the owner can create proposals.
        </p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// CollaborateTab — orchestrating shell
// ---------------------------------------------------------------------------

type ColPanel = "collaborators" | "comments" | "proposals";

export function CollaborateTab({
  contractName,
  userRole = "owner",
  contractCurrentYaml = "",
  onProposalApplied,
}: CollaborateTabProps & {
  contractCurrentYaml?: string;
  onProposalApplied?: (yaml: string) => void;
}) {
  const [panel, setPanel] = useState<ColPanel>("collaborators");

  return (
    <div className="space-y-4">
      {/* RFC note */}
      <div className="bg-indigo-950/20 border border-indigo-800/30 rounded-lg px-4 py-3">
        <p className="text-xs text-indigo-300/80 leading-relaxed">
          <span className="font-medium text-indigo-200">RFC-033:</span>{" "}
          Collaborators from other orgs can only see this contract — not your audit logs,
          quarantine events, or PII salt. Roles are enforced by the API.
        </p>
      </div>

      {/* Sub-nav */}
      <div className="flex gap-1 bg-[#0d1117] border border-[#1f2937] rounded-lg p-1 w-fit">
        {(["collaborators", "comments", "proposals"] as ColPanel[]).map((p) => (
          <button
            key={p}
            onClick={() => setPanel(p)}
            className={clsx(
              "px-3 py-1.5 text-xs font-medium rounded-md transition-colors capitalize",
              panel === p
                ? "bg-[#1f2937] text-slate-100"
                : "text-slate-500 hover:text-slate-300"
            )}
          >
            {p === "collaborators" ? "👥 " : p === "comments" ? "💬 " : "📝 "}
            {p}
          </button>
        ))}
      </div>

      {/* Panel body */}
      {panel === "collaborators" && (
        <CollaboratorsPanel contractName={contractName} userRole={userRole} />
      )}
      {panel === "comments" && (
        <CommentsPanel contractName={contractName} userRole={userRole} />
      )}
      {panel === "proposals" && (
        <ProposalsPanel
          contractName={contractName}
          contractCurrentYaml={contractCurrentYaml}
          userRole={userRole}
          onApplied={onProposalApplied}
        />
      )}
    </div>
  );
}
