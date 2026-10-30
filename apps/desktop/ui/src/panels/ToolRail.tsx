// SPDX-License-Identifier: GPL-3.0-or-later
import { Circle, ImageUp, LassoSelect, MousePointer2, Square, SquaresExclude, SquaresIntersect, SquaresSubtract, SquaresUnite, Trash2, Type } from "lucide-react";
import type { BoolOp } from "../App";
import { Tip } from "../components/Tip";
import { cn } from "../lib/cn";

type Props = {
  tool: string;
  selectionCount: number;
  onSelectTool: (tool: string) => void;
  onAddRect: () => void;
  onAddEllipse: () => void;
  onAddText: () => void;
  onTrace: () => void;
  onBoolean: (op: BoolOp) => void;
  onDelete: () => void;
};

const btn =
  "flex w-14 flex-col items-center gap-0.5 rounded-md border border-transparent px-0.5 py-1.5 text-[11px] leading-tight tracking-tight text-muted-foreground hover:bg-subtle hover:text-foreground disabled:pointer-events-none";

const BOOLS: { op: BoolOp; label: string; tip: string; kbd: string; Icon: typeof Square }[] = [
  { op: "Union", label: "Union", tip: "Weld / Union", kbd: "⌘U", Icon: SquaresUnite },
  { op: "Subtract", label: "Subtract", tip: "Subtract", kbd: "⌘⌥S", Icon: SquaresSubtract },
  { op: "Intersect", label: "Intersect", tip: "Intersect", kbd: "⌘⌥I", Icon: SquaresIntersect },
  { op: "Exclude", label: "Exclude", tip: "Exclude", kbd: "⌘⌥E", Icon: SquaresExclude },
];

export function ToolRail({
  tool,
  selectionCount,
  onSelectTool,
  onAddRect,
  onAddEllipse,
  onAddText,
  onTrace,
  onBoolean,
  onDelete,
}: Props) {
  const canBoolean = selectionCount >= 2;
  const canDelete = selectionCount > 0;
  const active = tool === "select";
  const lasso = tool === "lasso";
  return (
    <nav className="flex w-16 shrink-0 flex-col items-center gap-1 border-r border-border bg-panel py-2" aria-label="Tools">
      <Tip label="Select" kbd="V" side="right">
        <button
          type="button"
          aria-label="Select"
          aria-pressed={active}
          data-ink={active ? "accent" : "muted"}
          onClick={() => onSelectTool("select")}
          className={cn(btn, "cursor-pointer", active && "border-brand bg-tint/25 text-accent-ink dark:bg-tint/10")}
        >
          <MousePointer2 className="size-4" aria-hidden />
          Select
        </button>
      </Tip>
      <Tip label="Lasso" kbd="L" side="right">
        <button
          type="button"
          aria-label="Lasso"
          aria-pressed={lasso}
          data-ink={lasso ? "accent" : "muted"}
          onClick={() => onSelectTool("lasso")}
          className={cn(btn, "cursor-pointer", lasso && "border-brand bg-tint/25 text-accent-ink dark:bg-tint/10")}
        >
          <LassoSelect className="size-4" aria-hidden />
          Lasso
        </button>
      </Tip>
      <Tip label="Rectangle" kbd="R" side="right">
        <button type="button" aria-label="Rectangle" data-ink="muted" onClick={onAddRect} className={cn(btn, "cursor-pointer")}>
          <Square className="size-4" aria-hidden />
          Rectangle
        </button>
      </Tip>
      <Tip label="Ellipse" kbd="E" side="right">
        <button type="button" aria-label="Ellipse" data-ink="muted" onClick={onAddEllipse} className={cn(btn, "cursor-pointer")}>
          <Circle className="size-4" aria-hidden />
          Ellipse
        </button>
      </Tip>
      <Tip label="Text" kbd="T" side="right">
        <button type="button" aria-label="Text" data-ink="muted" onClick={onAddText} className={cn(btn, "cursor-pointer")}>
          <Type className="size-4" aria-hidden />
          Text
        </button>
      </Tip>
      <Tip label="Trace image" kbd="⇧T" side="right">
        <button type="button" aria-label="Trace" data-ink="muted" onClick={onTrace} className={cn(btn, "cursor-pointer")}>
          <ImageUp className="size-4" aria-hidden />
          Trace
        </button>
      </Tip>
      <div className="my-1 h-px w-10 bg-border" />
      {BOOLS.map(({ op, label, tip, kbd, Icon }) => (
        <Tip key={op} label={canBoolean ? tip : "Select 2 or more shapes"} kbd={kbd} side="right" disabled={!canBoolean}>
          <button type="button" aria-label={label} data-ink="muted" disabled={!canBoolean} onClick={() => onBoolean(op)} className={btn}>
            <Icon className="size-4" aria-hidden />
            {label}
          </button>
        </Tip>
      ))}
      <div className="mt-auto mb-1 h-px w-10 bg-border" />
      <Tip label="Delete" kbd="Del" side="right" disabled={!canDelete}>
        <button type="button" aria-label="Delete" data-ink="muted" disabled={!canDelete} onClick={onDelete} className={btn}>
          <Trash2 className="size-4" aria-hidden />
          Delete
        </button>
      </Tip>
    </nav>
  );
}
