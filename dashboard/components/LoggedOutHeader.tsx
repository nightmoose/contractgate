import Link from "next/link";
import { loggedOutAuthHrefs } from "@/lib/logged-out-cta";

// Persistent Sign up / Sign in for a logged-out session. Rendered by the
// root layout only when resolveVisitor() says there is no user, so the
// links are in the first HTML and a signed-in session does not see them.
export default function LoggedOutHeader({
  pathname,
  product,
}: {
  pathname: string;
  product: string | null;
}) {
  const { signUpHref, signInHref } = loggedOutAuthHrefs(pathname, product);

  return (
    <header className="fixed z-30 top-14 md:top-0 left-0 md:left-64 right-0 h-12 flex items-center justify-end gap-2 border-b border-[#1f2937] bg-[#0d1117]/95 px-3 sm:px-4 backdrop-blur">
      <p className="mr-auto min-w-0 truncate text-xs text-slate-400">
        Cloud Free needs no credit card
      </p>
      <Link
        href={signInHref}
        className="shrink-0 rounded-lg border border-[#374151] bg-[#1f2937] px-3 py-1.5 text-sm font-medium text-slate-200 hover:bg-[#374151] transition-colors"
      >
        Sign in
      </Link>
      <Link
        href={signUpHref}
        className="shrink-0 rounded-lg bg-green-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-green-500 transition-colors"
      >
        Sign up
      </Link>
    </header>
  );
}
