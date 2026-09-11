"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import * as Tooltip from "@radix-ui/react-tooltip";
import { HelpCard } from "./HelpCard";
import { HelpToggle } from "./HelpToggle";

type ActiveHelp = { id: string; rect: DOMRect };

type HelpContextValue = {
  mode: boolean;
  setMode: (v: boolean) => void;
  toggle: () => void;
  active: ActiveHelp | null;
  open: (id: string, el: Element) => void;
  close: () => void;
};

const HelpContext = createContext<HelpContextValue | null>(null);

export function useHelp(): HelpContextValue {
  const ctx = useContext(HelpContext);
  if (!ctx) throw new Error("useHelp must be used within HelpProvider");
  return ctx;
}

export function useHelpOptional(): HelpContextValue | null {
  return useContext(HelpContext);
}

function isTypingTarget(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable;
}

export default function HelpProvider({ children }: { children: ReactNode }) {
  const [mode, setModeState] = useState(false);
  const [active, setActive] = useState<ActiveHelp | null>(null);
  const [miss, setMiss] = useState<{ x: number; y: number } | null>(null);

  const close = useCallback(() => setActive(null), []);

  const setMode = useCallback((v: boolean) => {
    setModeState(v);
    if (!v) setActive(null);
  }, []);

  const toggle = useCallback(() => {
    setModeState((m) => {
      if (m) setActive(null);
      return !m;
    });
  }, []);

  const open = useCallback((id: string, el: Element) => {
    setActive({ id, rect: el.getBoundingClientRect() });
  }, []);

  useEffect(() => {
    document.documentElement.toggleAttribute("data-help-mode", mode);
    document.body.style.cursor = mode ? "help" : "";
    return () => {
      document.documentElement.removeAttribute("data-help-mode");
      document.body.style.cursor = "";
    };
  }, [mode]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (active) {
          e.preventDefault();
          close();
          return;
        }
        if (mode) {
          e.preventDefault();
          setMode(false);
        }
        return;
      }
      if (isTypingTarget(e.target)) return;
      if (e.key === "?" || (e.key === "/" && e.shiftKey)) {
        e.preventDefault();
        toggle();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [mode, active, close, setMode, toggle]);

  useEffect(() => {
    if (!mode) return;
    const onClick = (e: MouseEvent) => {
      const t = e.target as HTMLElement | null;
      if (!t) return;
      if (t.closest("[data-help-ui]")) return;
      e.preventDefault();
      e.stopPropagation();
      const hit = t.closest("[data-help]");
      if (hit) {
        const id = hit.getAttribute("data-help");
        if (id) open(id, hit);
        setMiss(null);
      } else {
        setActive(null);
        setMiss({ x: e.clientX, y: e.clientY });
      }
    };
    document.addEventListener("click", onClick, true);
    return () => document.removeEventListener("click", onClick, true);
  }, [mode, open]);

  useEffect(() => {
    if (!miss) return;
    const t = window.setTimeout(() => setMiss(null), 1600);
    return () => window.clearTimeout(t);
  }, [miss]);

  const value = useMemo(
    () => ({ mode, setMode, toggle, active, open, close }),
    [mode, setMode, toggle, active, open, close]
  );

  return (
    <HelpContext.Provider value={value}>
      <Tooltip.Provider delayDuration={300}>
        {children}
        {!mode && (
          <div data-help-ui className="fixed z-[60] bottom-4 right-4 md:hidden">
            <HelpToggle compact mode={mode} onToggle={toggle} />
          </div>
        )}
        {mode && (
          <div
            data-help-ui
            className="fixed z-[55] bottom-4 left-1/2 -translate-x-1/2 flex items-center gap-3 bg-[#111827] border border-green-800/50 text-slate-200 text-xs rounded-full px-4 py-2 shadow-xl"
          >
            <span>
              <span className="text-green-400 font-medium">What’s this?</span>
              {" — "}click a highlighted control
            </span>
            <kbd className="text-[10px] text-slate-500 border border-[#374151] rounded px-1.5 py-0.5">
              Esc
            </kbd>
            <button
              type="button"
              onClick={() => setMode(false)}
              className="text-slate-400 hover:text-white"
            >
              Exit
            </button>
          </div>
        )}
        {active && <HelpCard id={active.id} rect={active.rect} onClose={close} />}
        {miss && (
          <div
            data-help-ui
            className="fixed z-[220] pointer-events-none bg-[#1f2937] text-slate-300 text-xs rounded-lg px-3 py-1.5 border border-[#374151] shadow-xl"
            style={{
              top: Math.min(miss.y + 12, window.innerHeight - 40),
              left: Math.min(miss.x + 12, window.innerWidth - 220),
            }}
          >
            No description for this yet.
          </div>
        )}
      </Tooltip.Provider>
    </HelpContext.Provider>
  );
}
