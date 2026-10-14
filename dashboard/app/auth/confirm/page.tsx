import type { Metadata } from "next";
import Link from "next/link";
import BrandWordmark from "@/components/BrandWordmark";
import { confirmEmailLink } from "./actions";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  robots: { index: false, follow: false },
  title: "Confirm link",
};

function first(value: string | string[] | undefined): string {
  return Array.isArray(value) ? (value[0] ?? "") : (value ?? "");
}

function buttonLabel(type: string): string {
  if (type === "recovery") return "Continue to set a new password";
  if (type === "signup") return "Confirm email";
  if (type === "email_change") return "Confirm email change";
  if (type === "invite") return "Accept invite";
  return "Continue";
}

export default async function ConfirmEmailPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const sp = await searchParams;
  const tokenHash = first(sp.token_hash);
  const type = first(sp.type);
  const next = first(sp.next);
  const product = first(sp.product);
  const ready = tokenHash.length > 0 && type.length > 0;

  return (
    <div className="min-h-screen bg-[#0a0d12] flex items-center justify-center px-4">
      <div className="w-full max-w-sm">
        <div className="text-center mb-8">
          <Link href="/" className="inline-block">
            <BrandWordmark />
          </Link>
        </div>

        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-8">
          <h1 className="text-xl font-semibold text-slate-100 mb-2">
            {type === "recovery" ? "Reset your password" : "Confirm this link"}
          </h1>
          {ready ? (
            <>
              <p className="text-sm text-slate-500 mb-6">
                Email security tools open links before you do. Continue so this
                link is used by you, in this browser.
              </p>
              <form action={confirmEmailLink}>
                <input type="hidden" name="token_hash" value={tokenHash} />
                <input type="hidden" name="type" value={type} />
                <input type="hidden" name="next" value={next} />
                <input type="hidden" name="product" value={product} />
                <button
                  type="submit"
                  className="w-full bg-green-600 hover:bg-green-500 text-white rounded-lg px-4 py-2.5 text-sm font-medium transition-colors"
                >
                  {buttonLabel(type)}
                </button>
              </form>
            </>
          ) : (
            <div className="text-sm text-slate-500">
              <p>This link is missing its token. Request a new one and open that email.</p>
              <p className="mt-4">
                <Link href="/auth/forgot" className="text-green-400 hover:text-green-300">
                  Request a reset link
                </Link>
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
