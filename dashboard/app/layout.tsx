import type { Metadata } from "next";
import "./globals.css";
import Sidebar from "@/components/Sidebar";
import ClientOrgProvider from "@/components/ClientOrgProvider";
import { DEMO_MODE } from "@/lib/demo";
import DemoBanner from "@/components/DemoBanner";

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
        {/* Demo mode: fixed 36px banner at top; main shifts down with pt-9. */}
        {DEMO_MODE && <DemoBanner />}
        <Sidebar />
        <ClientOrgProvider />
        <main className={`flex-1 ml-0 md:ml-64 p-8 min-h-screen${DEMO_MODE ? " pt-[calc(2rem+36px)]" : ""}`}>
          {children}
        </main>
      </body>
    </html>
  );
}
