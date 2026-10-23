// SPDX-License-Identifier: GPL-3.0-or-later
import type { CSSProperties } from "react";
import { AT, AXIS, type AlignMode, type Axis, type DistributeBlock } from "../interaction/align";

type Props = {
  /** Selected ids that move as a piece; see `outermost`. */
  unitCount: number;
  /** Why every button is off for now, or null; see `CanvasInteraction.editsLock`. */
  locked: string | null;
  /** Per axis, from `distributeBlock`: why distribute cannot act there, or null. */
  distributeBlocked: Record<Axis, DistributeBlock | null>;
  onAlign: (mode: AlignMode) => void;
  onDistribute: (axis: Axis) => void;
};

const ALIGNS: { mode: AlignMode; label: string }[] = [
  { mode: "left", label: "Align left edges" },
  { mode: "hcenter", label: "Align horizontal centres" },
  { mode: "right", label: "Align right edges" },
  { mode: "top", label: "Align top edges" },
  { mode: "vmiddle", label: "Align vertical middles" },
  { mode: "bottom", label: "Align bottom edges" },
];
const DISTRIBUTES: { axis: Axis; label: string }[] = [
  { axis: "x", label: "Distribute horizontal spacing" },
  { axis: "y", label: "Distribute vertical spacing" },
];

const btn: CSSProperties = {
  background: "var(--panel)",
  color: "var(--text)",
  border: "1px solid var(--border)",
  padding: 3,
  cursor: "pointer",
  lineHeight: 0,
};

/** Drawn for x and turned a quarter for y, so the two families cannot drift apart. The line sits
 *  where the mode lines up (0, ½ or 1 across), with a long and a short bar against it. */
function AlignIcon({ at, vertical }: { at: number; vertical: boolean }) {
  const lx = 2 + at * 12;
  const bar = (y: number, w: number) => <rect x={lx - at * w} y={y} width={w} height={3} />;
  return (
    <svg width={16} height={16} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1}
         style={vertical ? { transform: "rotate(90deg) scaleY(-1)" } : undefined} aria-hidden>
      <line x1={lx} y1={1} x2={lx} y2={15} />
      {bar(3.5, 10)}
      {bar(9.5, 6)}
    </svg>
  );
}

function DistributeIcon({ vertical }: { vertical: boolean }) {
  return (
    <svg width={16} height={16} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1}
         style={vertical ? { transform: "rotate(90deg)" } : undefined} aria-hidden>
      <rect x={1.5} y={4.5} width={3} height={7} />
      <rect x={6.5} y={2.5} width={3} height={11} />
      <rect x={11.5} y={5.5} width={3} height={5} />
    </svg>
  );
}

const BLOCKED: Record<DistributeBlock, string> = {
  few: "select three or more pieces",
  // Per axis: pieces in a column with one shared width are "stacked" for horizontal distribute
  // without overlapping, so the reason names the axis, not an overlap (Copilot on #301).
  stacked: "more than one piece spans the selection on this axis",
  tight: "the pieces do not fit inside the one around them",
  crowded: "the pieces overlap too much to space out evenly",
  nested: "a piece inside the one around them spans the rest too",
};

/** Exported for its own test: the reason is the only thing a disabled button can tell. */
export function distributeTitle(label: string, block: DistributeBlock | null): string {
  return block ? `${label}: ${BLOCKED[block]}` : label;
}

export function AlignRow({ unitCount, locked, distributeBlocked, onAlign, onDistribute }: Props) {
  // One unit aligns to the artboard, so align needs one. Distribute's reasons come from
  // `distributeBlock`, per axis, and the tooltip names the one that disabled it.
  return (
    // Two rows, as the spec draws them: run together, the rotated distribute icon read as one more
    // align beside "Align bottom edges".
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
        {ALIGNS.map(({ mode, label }) => (
          <button key={mode} aria-label={label} title={locked ? `${label}: ${locked}` : label} disabled={locked !== null || unitCount === 0} style={btn}
                  onClick={() => onAlign(mode)}>
            <AlignIcon at={AT[mode]} vertical={AXIS[mode] === "y"} />
          </button>
        ))}
      </div>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
        {DISTRIBUTES.map(({ axis, label }) => (
          <button key={axis} aria-label={label} disabled={locked !== null || distributeBlocked[axis] !== null} style={btn}
                  title={locked ? `${label}: ${locked}` : distributeTitle(label, distributeBlocked[axis])}
                  onClick={() => onDistribute(axis)}>
            <DistributeIcon vertical={axis === "y"} />
          </button>
        ))}
      </div>
    </div>
  );
}
