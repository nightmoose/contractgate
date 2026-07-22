import { requireAdmin } from "@/lib/auth";
import { getOrgsOverview, type OrgOverview } from "@/lib/data";
import { logoutAction } from "../actions";

export const dynamic = "force-dynamic"; // always live; never cache god-mode data

function StatusPill({ o }: { o: OrgOverview }) {
  const s = o.live_sub_status ?? o.plan_status ?? (o.plan === "free" ? "free" : "—");
  return <span className="pill">{s}</span>;
}

export default async function UsersPage() {
  const operator = await requireAdmin();

  let orgs: OrgOverview[] = [];
  let error: string | null = null;
  try {
    orgs = await getOrgsOverview();
  } catch (e) {
    error = e instanceof Error ? e.message : String(e);
  }

  return (
    <>
      <header className="bar">
        <strong>ContractGate — Internal Admin</strong>
        <span style={{ fontSize: 12, color: "#94a3b8" }}>
          {operator}
          {" · "}
          <form action={logoutAction} style={{ display: "inline" }}>
            <button
              type="submit"
              style={{ background: "none", border: 0, color: "#4ade80", cursor: "pointer" }}
            >
              Sign out
            </button>
          </form>
        </span>
      </header>

      <div className="wrap">
        <h2 style={{ fontSize: 16 }}>Users &amp; Subscriptions</h2>
        <p style={{ color: "#64748b", fontSize: 12 }}>
          {orgs.length} orgs · read-only · act via the Stripe / Supabase links.
        </p>

        {error ? (
          <div className="err">{error}</div>
        ) : (
          <table>
            <thead>
              <tr>
                <th>Org</th>
                <th>Plan</th>
                <th>Status</th>
                <th>Members</th>
                <th>Created</th>
                <th>Act</th>
              </tr>
            </thead>
            <tbody>
              {orgs.map((o) => (
                <tr key={o.id}>
                  <td>
                    <div style={{ color: "#e2e8f0" }}>{o.name}</div>
                    <div style={{ color: "#64748b", fontFamily: "monospace", fontSize: 11 }}>
                      {o.slug ?? o.id}
                    </div>
                  </td>
                  <td style={{ textTransform: "capitalize" }}>{o.plan}</td>
                  <td>
                    <StatusPill o={o} />
                  </td>
                  <td>
                    <div>{o.member_count}</div>
                    <div style={{ color: "#64748b", fontSize: 11 }}>
                      {o.member_emails.slice(0, 3).join(", ")}
                      {o.member_emails.length > 3 ? ` +${o.member_emails.length - 3}` : ""}
                    </div>
                  </td>
                  <td style={{ color: "#64748b", fontSize: 12 }}>
                    {o.created_at ? new Date(o.created_at).toLocaleDateString() : "—"}
                  </td>
                  <td style={{ whiteSpace: "nowrap" }}>
                    {o.links.stripe_customer ? (
                      <a href={o.links.stripe_customer} target="_blank" rel="noreferrer">
                        Stripe
                      </a>
                    ) : (
                      <span style={{ color: "#475569" }}>Stripe</span>
                    )}
                    {" · "}
                    {o.links.supabase_org ? (
                      <a href={o.links.supabase_org} target="_blank" rel="noreferrer">
                        Supabase
                      </a>
                    ) : (
                      <span style={{ color: "#475569" }}>Supabase</span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}
