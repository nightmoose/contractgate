"use client";

/**
 * RFC-020 tooltip primitive, extracted so HelpProvider can own the
 * single Tooltip.Provider. Dynamic one-off copy still uses this;
 * catalog-backed copy goes through HelpTarget.
 */

import * as Tooltip from "@radix-ui/react-tooltip";

export function TooltipWrap({
  children,
  content,
}: {
  children: React.ReactNode;
  content: string;
}) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content
          className="max-w-xs text-xs bg-[#1f2937] text-slate-200 rounded-lg px-3 py-2 shadow-xl border border-[#374151] z-[200] leading-relaxed whitespace-pre-wrap"
          sideOffset={4}
        >
          {content}
          <Tooltip.Arrow className="fill-[#1f2937]" />
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
