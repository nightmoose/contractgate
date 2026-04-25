/**
 * POST /api/github/sync
 *
 * Commits a contract version's YAML to the configured GitHub repository.
 *
 * Request body:
 *   {
 *     contractId:   string   — ContractGate contract UUID (for the commit message)
 *     contractName: string   — Human-readable name (used as the file name slug)
 *     version:      string   — Semver string, e.g. "1.2.0"
 *     yamlContent:  string   — Full YAML text to write
 *   }
 *
 * Success response:
 *   { url: string, sha: string, path: string }
 *   where `url` is the GitHub web URL of the committed file.
 *
 * Conflict response (409):
 *   { error: "conflict", message: string, remote_sha: string }
 *   Caller can retry by supplying the remote_sha (not needed here — we always
 *   fetch the current SHA before writing, so conflicts are handled automatically).
 *
 * Error response (4xx/5xx):
 *   { error: string }
 *
 * Algorithm:
 *   1. Resolve the caller's org_id from their Supabase session.
 *   2. Read the github_integrations row (token, repo, prefix, branch) via service role.
 *   3. Build the file path: <prefix><slug>/<version>.yaml
 *      where slug = contract name lowercased, spaces→dashes, non-alphanumeric stripped.
 *   4. GET the current file from GitHub (to retrieve its SHA if it already exists).
 *   5. PUT the file to GitHub with the YAML content (base64-encoded) and current SHA.
 *   6. Return the GitHub web URL.
 */

import { createClient } from "@/lib/supabase/server";
import { createClient as createServiceClient } from "@supabase/supabase-js";
import { NextResponse } from "next/server";

const GITHUB_API = "https://api.github.com";

function getServiceClient() {
  return createServiceClient(
    process.env.NEXT_PUBLIC_SUPABASE_URL!,
    process.env.SUPABASE_SERVICE_ROLE_KEY!
  );
}

async function resolveOrgId(userId: string): Promise<string | null> {
  const svc = getServiceClient();
  const { data } = await svc
    .from("org_memberships")
    .select("org_id")
    .eq("user_id", userId)
    .order("joined_at", { ascending: true })
    .limit(1)
    .single();
  return data?.org_id ?? null;
}

interface GitHubIntegration {
  repo: string;
  path_prefix: string;
  branch: string;
  github_token: string | null;
}

async function getIntegration(orgId: string): Promise<GitHubIntegration | null> {
  const svc = getServiceClient();
  const { data, error } = await svc
    .from("github_integrations")
    .select("repo, path_prefix, branch, github_token")
    .eq("org_id", orgId)
    .maybeSingle();
  if (error || !data) return null;
  return data as GitHubIntegration;
}

/**
 * Convert a contract name to a URL/filesystem safe slug.
 * e.g. "User Events v2" → "user-events-v2"
 */
function toSlug(name: string): string {
  return name
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

interface GitHubFileResponse {
  sha?: string;
  html_url?: string;
  content?: {
    sha: string;
    html_url: string;
  };
}

/**
 * Fetch the current file from GitHub to get its SHA (needed for updates).
 * Returns null if the file doesn't exist yet (new file creation).
 */
async function getExistingFileSha(
  token: string,
  repo: string,
  filePath: string,
  branch: string
): Promise<string | null> {
  const url = `${GITHUB_API}/repos/${repo}/contents/${filePath}?ref=${encodeURIComponent(branch)}`;
  const res = await fetch(url, {
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
    },
  });

  if (res.status === 404) return null; // File doesn't exist yet
  if (!res.ok) {
    const body = await res.text();
    throw new Error(`GitHub GET file failed (${res.status}): ${body}`);
  }

  const data = (await res.json()) as { sha?: string };
  return data.sha ?? null;
}

/**
 * Commit a file to GitHub using the Contents API.
 * If sha is provided the file is updated; otherwise it is created.
 *
 * Returns the committed file's sha and html_url.
 */
async function commitFile(
  token: string,
  repo: string,
  filePath: string,
  branch: string,
  content: string,
  message: string,
  existingSha: string | null
): Promise<{ sha: string; html_url: string }> {
  const url = `${GITHUB_API}/repos/${repo}/contents/${filePath}`;

  // GitHub Contents API requires base64-encoded content.
  const encoded = Buffer.from(content, "utf8").toString("base64");

  const body: Record<string, unknown> = {
    message,
    content: encoded,
    branch,
  };
  if (existingSha) {
    body.sha = existingSha;
  }

  const res = await fetch(url, {
    method: "PUT",
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });

  if (res.status === 409) {
    // SHA mismatch — the file was modified externally between our GET and PUT.
    // This is rare since we always fetch the SHA immediately before writing,
    // but it can happen under concurrent edits. Return a structured conflict error.
    const errBody = await res.text();
    return Promise.reject(
      Object.assign(new Error(`GitHub conflict: ${errBody}`), { code: "conflict" })
    );
  }

  if (!res.ok) {
    const errBody = await res.text();
    throw new Error(`GitHub PUT file failed (${res.status}): ${errBody}`);
  }

  const data = (await res.json()) as GitHubFileResponse;
  const committed = data.content ?? data;
  return {
    sha: (committed as { sha: string }).sha,
    html_url: (committed as { html_url: string }).html_url,
  };
}

// ── POST handler ──────────────────────────────────────────────────────────────

export async function POST(request: Request) {
  // 1. Auth
  const supabase = await createClient();
  const { data: { user }, error: authErr } = await supabase.auth.getUser();
  if (authErr || !user) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const orgId = await resolveOrgId(user.id);
  if (!orgId) {
    return NextResponse.json({ error: "No org found for user" }, { status: 404 });
  }

  // 2. Parse body
  let body: {
    contractId?: string;
    contractName?: string;
    version?: string;
    yamlContent?: string;
  };
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
  }

  const { contractId, contractName, version, yamlContent } = body;
  if (!contractName || !version || !yamlContent) {
    return NextResponse.json(
      { error: "contractName, version, and yamlContent are required" },
      { status: 400 }
    );
  }

  // 3. Load integration config
  const integration = await getIntegration(orgId);
  if (!integration) {
    return NextResponse.json(
      { error: "GitHub integration not configured. Set it up in Account → GitHub Integration." },
      { status: 422 }
    );
  }
  if (!integration.github_token) {
    return NextResponse.json(
      { error: "GitHub token not set. Add a Personal Access Token in Account → GitHub Integration." },
      { status: 422 }
    );
  }

  // 4. Build file path: <prefix><slug>/<version>.yaml
  const slug = toSlug(contractName);
  const filePath = `${integration.path_prefix}${slug}/${version}.yaml`;

  // 5. Fetch existing SHA (null = new file)
  let existingSha: string | null;
  try {
    existingSha = await getExistingFileSha(
      integration.github_token,
      integration.repo,
      filePath,
      integration.branch
    );
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error("[github/sync] getExistingFileSha error:", msg);
    return NextResponse.json(
      { error: `Failed to check existing file on GitHub: ${msg}` },
      { status: 502 }
    );
  }

  // 6. Commit
  const action = existingSha ? "Update" : "Add";
  const commitMessage = `${action} contract ${contractName} v${version}${contractId ? ` [${contractId.slice(0, 8)}]` : ""}`;

  let committed: { sha: string; html_url: string };
  try {
    committed = await commitFile(
      integration.github_token,
      integration.repo,
      filePath,
      integration.branch,
      yamlContent,
      commitMessage,
      existingSha
    );
  } catch (err) {
    const e = err as Error & { code?: string };
    if (e.code === "conflict") {
      return NextResponse.json(
        {
          error: "conflict",
          message:
            "The file was modified on GitHub between when we checked and when we tried to write. Please try again.",
        },
        { status: 409 }
      );
    }
    console.error("[github/sync] commitFile error:", e.message);
    return NextResponse.json(
      { error: `GitHub commit failed: ${e.message}` },
      { status: 502 }
    );
  }

  return NextResponse.json({
    url: committed.html_url,
    sha: committed.sha,
    path: filePath,
    repo: integration.repo,
    branch: integration.branch,
  });
}
