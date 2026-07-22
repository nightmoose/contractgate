import { loginAction } from "../actions";

export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<{ error?: string; denied?: string }>;
}) {
  const sp = await searchParams;
  const msg = sp.denied
    ? "That account is not authorized for the admin console."
    : sp.error;

  return (
    <div className="wrap" style={{ maxWidth: 380, marginTop: 80 }}>
      <div className="card">
        <h1 style={{ marginTop: 0, fontSize: 18 }}>ContractGate — Internal Admin</h1>
        <p style={{ color: "#94a3b8", fontSize: 13 }}>
          Superadmin access only. Sign in with an allowlisted account.
        </p>
        {msg && <div className="err" style={{ marginBottom: 12 }}>{msg}</div>}
        <form action={loginAction} style={{ display: "grid", gap: 12 }}>
          <input name="email" type="email" placeholder="you@yourdomain.com" required />
          <input name="password" type="password" placeholder="Password" required />
          <button className="primary" type="submit">
            Sign in
          </button>
        </form>
      </div>
    </div>
  );
}
