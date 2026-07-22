import "server-only";
import { supabaseAdmin } from "./supabaseAdmin";
import { subscriptionStatus } from "./stripe";

export interface OrgOverview {
  id: string;
  name: string;
  slug: string | null;
  plan: string;
  plan_status: string | null;
  stripe_customer_id: string | null;
  stripe_subscription_id: string | null;
  created_at: string | null;
  member_emails: string[];
  member_count: number;
  /** Live status from the Stripe API (source of truth), if a sub exists. */
  live_sub_status: string | null;
  links: {
    stripe_customer: string | null;
    stripe_subscription: string | null;
    supabase_org: string | null;
  };
}

function supabaseOrgLink(orgId: string): string | null {
  const ref = process.env.SUPABASE_PROJECT_REF;
  if (!ref) return null;
  // Deep-link to the SQL editor prefilled with this org's row.
  const sql = encodeURIComponent(`select * from public.orgs where id = '${orgId}';`);
  return `https://supabase.com/dashboard/project/${ref}/sql/new?content=${sql}`;
}

/**
 * God-mode overview of every org: plan + subscription status joined across
 * Supabase (service role) and Stripe, with member emails and deep links. Read
 * only. N Stripe calls (one per org with a subscription) run in parallel.
 */
export async function getOrgsOverview(): Promise<OrgOverview[]> {
  const db = supabaseAdmin();

  const [{ data: orgs, error: orgErr }, { data: memberships, error: memErr }] =
    await Promise.all([
      db
        .from("orgs")
        .select(
          "id, name, slug, plan, plan_status, stripe_customer_id, stripe_subscription_id, created_at"
        )
        .order("created_at", { ascending: false }),
      db.from("org_memberships").select("org_id, user_id"),
    ]);

  if (orgErr) throw new Error(`orgs query failed: ${orgErr.message}`);
  if (memErr) throw new Error(`memberships query failed: ${memErr.message}`);

  // Resolve user emails (auth.users is not in the public schema — use the
  // auth admin API and build an id → email map once).
  const emailById = new Map<string, string>();
  const { data: usersPage } = await db.auth.admin.listUsers({ perPage: 1000 });
  for (const u of usersPage?.users ?? []) {
    if (u.email) emailById.set(u.id, u.email);
  }

  const membersByOrg = new Map<string, string[]>();
  for (const m of memberships ?? []) {
    const email = emailById.get(m.user_id) ?? m.user_id;
    const list = membersByOrg.get(m.org_id) ?? [];
    list.push(email);
    membersByOrg.set(m.org_id, list);
  }

  return Promise.all(
    (orgs ?? []).map(async (o): Promise<OrgOverview> => {
      const members = membersByOrg.get(o.id) ?? [];
      const live = await subscriptionStatus(o.stripe_subscription_id);
      return {
        id: o.id,
        name: o.name,
        slug: o.slug,
        plan: o.plan,
        plan_status: o.plan_status,
        stripe_customer_id: o.stripe_customer_id,
        stripe_subscription_id: o.stripe_subscription_id,
        created_at: o.created_at,
        member_emails: members,
        member_count: members.length,
        live_sub_status: live,
        links: {
          stripe_customer: o.stripe_customer_id
            ? `https://dashboard.stripe.com/customers/${o.stripe_customer_id}`
            : null,
          stripe_subscription: o.stripe_subscription_id
            ? `https://dashboard.stripe.com/subscriptions/${o.stripe_subscription_id}`
            : null,
          supabase_org: supabaseOrgLink(o.id),
        },
      };
    })
  );
}
