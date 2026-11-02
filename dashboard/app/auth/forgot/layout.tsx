import type { Metadata } from "next";
import { pageMeta } from "@/lib/page-meta";

export const metadata: Metadata = pageMeta(
  "Reset password",
  "Email yourself a link to reset your DataContractGate password.",
);

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
