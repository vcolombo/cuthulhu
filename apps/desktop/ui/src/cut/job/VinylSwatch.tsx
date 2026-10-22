// SPDX-License-Identifier: GPL-3.0-or-later
import { cn } from "../../lib/cn";
import { vinylDisplay, type VinylTheme } from "../../render/vinyl";

/** The only paint a job swatch uses. A pass with no colour (every shape, a preset) stays on the subtle token. */
export function VinylSwatch({ color, theme, className }: { color: string | null; theme: VinylTheme; className?: string }) {
  const paint = color === null ? "var(--subtle)" : vinylDisplay(color, theme).swatch;
  return (
    <span
      aria-hidden
      className={cn("inline-block shrink-0 border border-foreground/20", className)}
      style={{ background: paint }}
    />
  );
}
