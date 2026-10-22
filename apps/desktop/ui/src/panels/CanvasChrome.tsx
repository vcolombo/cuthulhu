// SPDX-License-Identifier: GPL-3.0-or-later
import { Button } from "../components/ui/button";
import type { LengthUnit } from "../length";
import { MM_PER_IN } from "../length";
import type { Size, View } from "../interaction/viewport";

/** Lovable's ruler thickness. Fit insets by this on the left and top while rulers are on. */
export const RULER_PX = 24;
/** Canvas px from the bottom edge up to the top of the zoom bar: `bottom-3` (12), the border
 *  (2), the padding (8) and the 32 px controls. Fit insets by this so the mat stays above the bar. */
export const ZOOM_BAR_CLEAR_PX = 12 + 2 + 8 + 32;

type RulerProps = {
  view: View;
  size: Size;
  unit: LengthUnit;
};

type Tick = { screen: number; label: string | null };

function ticks(world0: number, world1: number, scale: number, originScreen: number, unit: LengthUnit): Tick[] {
  if (!(scale > 0) || world1 < world0) return [];
  const major = unit === "in" ? MM_PER_IN : 10;
  const minor = unit === "in" ? MM_PER_IN / 4 : 5;
  const first = Math.ceil((world0 - 1e-6) / minor) * minor;
  const out: Tick[] = [];
  for (let mm = first; mm <= world1 + 1e-6; mm += minor) {
    const majorTick = Math.abs(mm / major - Math.round(mm / major)) < 1e-4;
    out.push({
      screen: mm * scale + originScreen,
      label: majorTick ? tickLabel(mm, unit) : null,
    });
    if (out.length > 400) break;
  }
  return out;
}

function tickLabel(mm: number, unit: LengthUnit): string {
  if (unit === "mm") return String(Math.round(mm));
  const inches = mm / MM_PER_IN;
  const rounded = Math.round(inches * 4) / 4;
  return Number.isInteger(rounded) ? String(rounded) : rounded.toFixed(2).replace(/0$/, "");
}

export function Rulers({ view, size, unit }: RulerProps) {
  if (size.w <= RULER_PX || size.h <= RULER_PX) return null;
  const horizontal = ticks((RULER_PX - view.tx) / view.scale, (size.w - view.tx) / view.scale, view.scale, view.tx, unit);
  const vertical = ticks((RULER_PX - view.ty) / view.scale, (size.h - view.ty) / view.scale, view.scale, view.ty, unit);
  return (
    <svg className="pointer-events-none absolute inset-0 z-[1] text-muted-foreground" width={size.w} height={size.h} aria-hidden>
      <rect width={size.w} height={RULER_PX} className="fill-panel" />
      <rect width={RULER_PX} height={size.h} className="fill-panel" />
      <line x1={0} y1={RULER_PX - 0.5} x2={size.w} y2={RULER_PX - 0.5} className="stroke-border" />
      <line x1={RULER_PX - 0.5} y1={0} x2={RULER_PX - 0.5} y2={size.h} className="stroke-border" />
      {horizontal.map((tick) =>
        tick.screen < RULER_PX ? null : (
          <g key={`x${tick.screen}`}>
            <line
              x1={tick.screen}
              y1={tick.label ? 8 : 16}
              x2={tick.screen}
              y2={RULER_PX}
              className="stroke-border"
            />
            {tick.label ? (
              <text x={tick.screen} y={10} textAnchor="middle" fontSize={10} fill="currentColor">
                {tick.label}
              </text>
            ) : null}
          </g>
        ),
      )}
      {vertical.map((tick) =>
        tick.screen < RULER_PX ? null : (
          <g key={`y${tick.screen}`}>
            <line
              x1={tick.label ? 8 : 16}
              y1={tick.screen}
              x2={RULER_PX}
              y2={tick.screen}
              className="stroke-border"
            />
            {tick.label ? (
              <text x={12} y={tick.screen} textAnchor="middle" fontSize={10} fill="currentColor" transform={`rotate(-90 12 ${tick.screen})`}>
                {tick.label}
              </text>
            ) : null}
          </g>
        ),
      )}
    </svg>
  );
}

type ZoomProps = {
  zoomPercent: number;
  rulers: boolean;
  onZoomOut: () => void;
  onZoomIn: () => void;
  onFitMat: () => void;
  onFitContent: () => void;
  onToggleRulers: () => void;
};

export function ZoomControls({ zoomPercent, rulers, onZoomOut, onZoomIn, onFitMat, onFitContent, onToggleRulers }: ZoomProps) {
  return (
    <div data-testid="zoom-controls" className="absolute bottom-3 right-3 z-[2] flex items-center gap-0.5 rounded-md border border-border bg-panel p-1 shadow-sm">
      <Button variant="ghost" size="icon" className="text-foreground" aria-label="Zoom out" onClick={onZoomOut}>
        −
      </Button>
      <span className="tabular min-w-12 px-1 text-center text-xs text-foreground">{zoomPercent}%</span>
      <Button variant="ghost" size="icon" className="text-foreground" aria-label="Zoom in" onClick={onZoomIn}>
        +
      </Button>
      <Button variant="ghost" size="sm" className="text-foreground" onClick={onFitMat}>
        Fit mat
      </Button>
      <Button variant="ghost" size="sm" className="text-foreground" onClick={onFitContent}>
        Fit content
      </Button>
      <Button
        variant="ghost"
        size="sm"
        aria-pressed={rulers}
        onClick={onToggleRulers}
        className={rulers ? "bg-primary-foreground text-primary" : undefined}
      >
        Rulers
      </Button>
    </div>
  );
}
