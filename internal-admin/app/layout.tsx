import "./globals.css";
import type { ReactNode } from "react";

export const metadata = {
  title: "DataContractGate — Internal Admin",
  robots: "noindex, nofollow",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
