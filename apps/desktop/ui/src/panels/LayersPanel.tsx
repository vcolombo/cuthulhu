// SPDX-License-Identifier: GPL-3.0-or-later
import { useMemo, useRef, useState, type MutableRefObject } from "react";
import { ChevronDown, ChevronRight, Circle, Group as GroupIcon, Layers, Scissors, Spline, Square, Type } from "lucide-react";
import type { DocSnapshot } from "../App";
import type { Grouping } from "../ipc";
import type { PresetLookup } from "../cut/viewmodel";
import { vinylDisplay, type VinylTheme } from "../render/vinyl";
import { cn } from "../lib/cn";
import type { CutLineTypeJson } from "./cutLineType";
import { layerFooter, layerHeading, layerModel, reorderTarget, type LayerIcon, type LayerNodeRow } from "./layerRows";

type Props = {
  doc: DocSnapshot | null;
  grouping: Grouping;
  theme: VinylTheme;
  /** Names a preset-keyed block. Colour blocks do not read it. */
  presets: PresetLookup;
  /** Resolved material per node, from `effectiveMaterials`. The pass key reads this, not a second walk. */
  materials: Readonly<Record<number, string | null>>;
  selected: number[];
  /** Why a cut toggle or a drag is refused, or null. */
  editsLocked: string | null;
  onSelect: (id: number, shiftKey: boolean) => void;
  /** A colour block selects its shapes, not the groups that happen to hold them. */
  onSelectShapes: (ids: number[], shiftKey: boolean) => void;
  onToggleCut: (ids: number[], value: CutLineTypeJson) => void;
  onReorder: (id: number, newIndex: number) => void;
};

const ICONS = {
  rectangle: Square,
  ellipse: Circle,
  text: Type,
  path: Spline,
  group: GroupIcon,
  layer: Layers,
} as const;

function RowIcon({ icon }: { icon: LayerIcon }) {
  const Cmp = ICONS[icon];
  return <Cmp className="size-3.5 shrink-0" aria-hidden />;
}

function NodeRow({
  doc,
  row,
  selected,
  locked,
  dragId,
  onSelect,
  onReorder,
}: {
  doc: DocSnapshot;
  row: LayerNodeRow;
  selected: boolean;
  locked: string | null;
  dragId: MutableRefObject<number | null>;
  onSelect: (id: number, shiftKey: boolean) => void;
  onReorder: (id: number, newIndex: number) => void;
}) {
  return (
    <div
      data-testid="layer-row"
      data-selected={selected}
      draggable={locked === null}
      onDragStart={(event) => {
        dragId.current = row.id;
        event.dataTransfer.setData("text/plain", String(row.id));
        event.dataTransfer.effectAllowed = "move";
      }}
      onDragEnd={() => {
        dragId.current = null;
      }}
      onDragOver={(event) => {
        const from = dragId.current;
        if (from === null || locked !== null) return;
        if (reorderTarget(doc, from, row.id)) event.preventDefault();
      }}
      onDrop={(event) => {
        const from = dragId.current ?? Number(event.dataTransfer.getData("text/plain"));
        dragId.current = null;
        if (!Number.isInteger(from) || locked !== null) return;
        const target = reorderTarget(doc, from, row.id);
        if (!target) return;
        event.preventDefault();
        onReorder(target.id, target.newIndex);
      }}
      onClick={(event) => onSelect(row.id, event.shiftKey)}
      className={cn(
        "flex cursor-pointer items-center gap-2 rounded-sm border px-1.5 py-1 text-xs",
        selected ? "border-primary bg-accent" : "border-transparent",
        row.repeated && "text-muted-foreground",
        locked === null && "cursor-grab",
      )}
      style={{ marginLeft: 28 + row.depth * 12, marginRight: 4 }}
      title={row.repeated ? "This group’s shapes continue in this block" : undefined}
    >
      <RowIcon icon={row.icon} />
      <span className="min-w-0 flex-1 truncate">{row.name}</span>
    </div>
  );
}

export function LayersPanel({
  doc,
  grouping,
  theme,
  presets,
  materials,
  selected,
  editsLocked,
  onSelect,
  onSelectShapes,
  onToggleCut,
  onReorder,
}: Props) {
  const model = useMemo(
    () => (doc ? layerModel(doc, grouping, materials, presets) : { blocks: [], trailing: [] }),
    [doc, grouping, materials, presets],
  );
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const dragId = useRef<number | null>(null);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="px-2 py-1 text-2xs uppercase text-muted-foreground">{layerHeading(grouping)}</div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {doc
          ? model.blocks.map((block) => {
              const open = !collapsed.has(block.key);
              const allSelected = block.shapeIds.length > 0 && block.shapeIds.every((id) => selected.includes(id));
              const swatch = block.swatch ? vinylDisplay(block.swatch, theme).swatch : null;
              return (
                <div key={block.key} data-testid="colour-layer">
                  <div className="flex items-center gap-0.5 pr-1">
                    <button
                      type="button"
                      data-ink="muted"
                      aria-expanded={open}
                      aria-label={`${open ? "Collapse" : "Expand"} ${block.name}`}
                      className="cursor-pointer rounded-sm p-1"
                      onClick={() => {
                        setCollapsed((prev) => {
                          const next = new Set(prev);
                          if (next.has(block.key)) next.delete(block.key);
                          else next.add(block.key);
                          return next;
                        });
                      }}
                    >
                      {open ? <ChevronDown className="size-3.5" aria-hidden /> : <ChevronRight className="size-3.5" aria-hidden />}
                    </button>
                    <button
                      type="button"
                      data-selected={allSelected}
                      aria-current={allSelected ? "true" : undefined}
                      className={cn(
                        "flex min-w-0 flex-1 cursor-pointer items-center gap-2 rounded-sm border px-1.5 py-1 text-left text-xs",
                        allSelected ? "border-primary bg-accent" : "border-transparent",
                      )}
                      onClick={(event) => onSelectShapes(block.shapeIds, event.shiftKey)}
                    >
                      {swatch ? (
                        <span
                          className="inline-block size-3.5 shrink-0 rounded-sm border border-input"
                          style={{ background: swatch }}
                        />
                      ) : (
                        <span className="inline-block size-3.5 shrink-0 rounded-sm border border-input bg-muted" />
                      )}
                      <span className="min-w-0 flex-1 truncate">{block.name}</span>
                      <span className="tabular">{block.shapeIds.length}</span>
                    </button>
                    <button
                      type="button"
                      data-ink="muted"
                      data-toggle="quiet"
                      aria-label={`Cut ${block.name}`}
                      aria-pressed={block.cut === "Cut" ? true : block.cut === "mixed" ? "mixed" : false}
                      title={editsLocked ? `Cut ${block.name}: ${editsLocked}` : `Cut ${block.name}`}
                      disabled={editsLocked !== null || block.shapeIds.length === 0}
                      className={cn(
                        "cursor-pointer rounded-sm border p-1",
                        block.cut === "mixed" ? "border-dashed border-primary" : "border-transparent",
                      )}
                      onClick={() => onToggleCut(block.shapeIds, block.cut === "Cut" ? "NoCut" : "Cut")}
                    >
                      <Scissors className="size-3.5" aria-hidden />
                    </button>
                  </div>
                  {open
                    ? block.rows.map((row) => (
                        <NodeRow
                          key={row.id}
                          doc={doc}
                          row={row}
                          selected={selected.includes(row.id)}
                          locked={editsLocked}
                          dragId={dragId}
                          onSelect={onSelect}
                          onReorder={onReorder}
                        />
                      ))
                    : null}
                </div>
              );
            })
          : null}
        {doc
          ? model.trailing.map((row) => (
              <NodeRow
                key={row.id}
                doc={doc}
                row={row}
                selected={selected.includes(row.id)}
                locked={editsLocked}
                dragId={dragId}
                onSelect={onSelect}
                onReorder={onReorder}
              />
            ))
          : null}
      </div>
      <p className="shrink-0 border-t border-border px-2 py-2 text-2xs text-muted-foreground">
        {layerFooter(grouping)}
      </p>
    </div>
  );
}
