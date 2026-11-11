"use client";

import { createContext, useContext } from "react";

// null: the server did not say. AuthGate then checks the browser session.
const VisitorAuthedContext = createContext<boolean | null>(null);

export function VisitorSessionProvider({
  authed,
  children,
}: {
  authed: boolean | null;
  children: React.ReactNode;
}) {
  return <VisitorAuthedContext.Provider value={authed}>{children}</VisitorAuthedContext.Provider>;
}

export function useVisitorAuthed(): boolean | null {
  return useContext(VisitorAuthedContext);
}
