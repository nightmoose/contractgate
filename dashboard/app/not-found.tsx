import Link from "next/link";

export const metadata = { title: "Page not found — ContractGate" };

export default function NotFound() {
  return (
    <div className="min-h-screen bg-[#0a0d12] flex flex-col items-center justify-center px-4 text-center">
      <p className="text-7xl font-bold text-[#1f2937] mb-6 select-none">404</p>

      <h1 className="text-2xl font-bold text-slate-100 mb-2">Page not found</h1>
      <p className="text-slate-500 text-sm mb-8 max-w-sm">
        The page you&apos;re looking for doesn&apos;t exist or has been moved.
      </p>

      <Link
        href="/contracts"
        className="px-5 py-2.5 bg-green-600 hover:bg-green-500 text-white rounded-lg text-sm font-medium transition-colors"
      >
        Back to dashboard
      </Link>
    </div>
  );
}
