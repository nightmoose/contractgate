import type { Metadata } from "next";
import "./globals.css";
import Sidebar from "@/components/Sidebar";
import dynamic from "next/dynamic";

// OrgProvider calls createBrowserClient (Supabase) on mount — it must never
// run during SSR or static prerendering, where Supabase env vars are absent.
const OrgProvider = dynamic(() => import("@/components/OrgProvider"), {
  ssr: false,
});

export const metadata: Metadata = {
  title: "ContractGate — Semantic Contract Enforcement",
  description:
    "Real-time semantic contract validation gateway — Patent Pending",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" className="dark">
      <body className="min-h-screen bg-[#0a0d12] text-slate-200 flex">
        <Sidebar />
        <OrgProvider />
        <main className="flex-1 ml-64 p-8 min-h-screen">{children}</main>
      </body>
    </html>
  );
}
