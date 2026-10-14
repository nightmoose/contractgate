import { NextRequest, NextResponse } from 'next/server';
import Stripe from 'stripe';
import { createClient } from '@/lib/supabase/server';
import { getServiceClient } from '@/lib/org/authz';

export const dynamic = 'force-dynamic';

// Self-serve account deletion. The caller must re-type their email.
// 1. Note Stripe subscriptions of orgs where the caller is the only member.
// 2. delete_user_account() (migration 040, service role only) refuses for a
//    last owner with other members, else purges sole-member orgs and clears
//    references to the user.
// 3. Cancel those subscriptions immediately, then delete the auth user.
export async function POST(req: NextRequest) {
  const supabase = await createClient();
  const { data: { user } } = await supabase.auth.getUser();
  if (!user) return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });

  const body = await req.json().catch(() => ({}));
  const confirm = typeof body?.confirm === 'string' ? body.confirm.trim().toLowerCase() : '';
  if (!user.email || confirm !== user.email.toLowerCase()) {
    return NextResponse.json({ error: 'Type your email address to confirm.' }, { status: 400 });
  }

  const svc = getServiceClient();

  const { data: mine } = await svc
    .from('org_memberships')
    .select('org_id, orgs(stripe_subscription_id)')
    .eq('user_id', user.id);
  const subIds: string[] = [];
  for (const m of mine ?? []) {
    const { count } = await svc
      .from('org_memberships')
      .select('user_id', { count: 'exact', head: true })
      .eq('org_id', m.org_id);
    const orgs = m.orgs as { stripe_subscription_id?: string | null } | { stripe_subscription_id?: string | null }[] | null;
    const sub = Array.isArray(orgs) ? orgs[0]?.stripe_subscription_id : orgs?.stripe_subscription_id;
    if (count === 1 && sub) subIds.push(sub);
  }

  const { error: rpcError } = await svc.rpc('delete_user_account', { p_user: user.id });
  if (rpcError) {
    if (rpcError.message?.includes('last_owner')) {
      return NextResponse.json(
        { error: 'You are the last owner of an organization with other members. Make someone else an owner first.' },
        { status: 409 },
      );
    }
    console.error('delete_user_account failed:', rpcError);
    return NextResponse.json({ error: 'Could not delete account data. Nothing was deleted.' }, { status: 500 });
  }

  if (subIds.length && process.env.STRIPE_SECRET_KEY) {
    const stripe = new Stripe(process.env.STRIPE_SECRET_KEY, { apiVersion: '2026-06-24.dahlia' });
    for (const id of subIds) {
      try {
        await stripe.subscriptions.cancel(id);
      } catch (err) {
        // Data is already gone; flag for manual follow-up rather than fail.
        console.error(`account delete: could not cancel subscription ${id}:`, err);
      }
    }
  }

  const { error: delError } = await svc.auth.admin.deleteUser(user.id);
  if (delError) {
    console.error('auth deleteUser failed:', delError);
    return NextResponse.json({ error: 'Account data was removed but sign-in could not be deleted. Contact support.' }, { status: 500 });
  }

  await supabase.auth.signOut().catch(() => undefined);
  return NextResponse.json({ ok: true });
}
