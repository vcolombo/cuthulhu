// SPDX-License-Identifier: GPL-3.0-or-later
import type { ReactElement, ReactNode } from "react";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex h-4 min-w-4 items-center justify-center rounded-sm border border-input bg-subtle px-1 font-sans text-2xs text-muted-foreground">
      {children}
    </kbd>
  );
}

/**
 * A disabled control does not receive hover or focus, so the tooltip trigger is a span that does.
 * The span is in the tab order only while the control is disabled; otherwise the control itself is.
 */
export function Tip({
  label,
  kbd,
  side = "bottom",
  disabled = false,
  children,
}: {
  label: string;
  kbd?: string;
  side?: "top" | "bottom" | "left" | "right";
  disabled?: boolean;
  children: ReactElement;
}) {
  const trigger = disabled ? (
    <span tabIndex={0} className="inline-flex rounded-md">
      {children}
    </span>
  ) : (
    children
  );
  return (
    <Tooltip delayDuration={300}>
      <TooltipTrigger asChild>{trigger}</TooltipTrigger>
      <TooltipContent side={side}>
        {label}
        {kbd ? <Kbd>{kbd}</Kbd> : null}
      </TooltipContent>
    </Tooltip>
  );
}
