"use client";

import { cloneElement, isValidElement, type ReactElement } from "react";
import { getHelp } from "@/lib/help/catalog";
import { TooltipWrap } from "./TooltipWrap";
import { useHelpOptional } from "./HelpProvider";

type HelpChild = ReactElement<{ className?: string }>;

/**
 * Marks a control as a help target (`data-help={id}`).
 * With inspect mode off, shows the catalog hover tooltip.
 */
export function HelpTarget({
  id,
  children,
  hover = true,
}: {
  id: string;
  children: HelpChild;
  hover?: boolean;
}) {
  const help = useHelpOptional();
  if (!isValidElement(children)) return children;

  const tagged = cloneElement(children, {
    ...children.props,
    "data-help": id,
  } as HelpChild["props"] & { "data-help": string });

  const entry = getHelp(id);
  if (!hover || !entry || help?.mode) return tagged;

  return <TooltipWrap content={entry.hover ?? entry.what}>{tagged}</TooltipWrap>;
}
