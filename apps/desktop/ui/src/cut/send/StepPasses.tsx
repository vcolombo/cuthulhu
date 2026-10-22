// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useId, useState } from "react";
import { ChevronDown, GripVertical } from "lucide-react";
import type { LengthUnit } from "../../length";
import { formatLength, unitToMm } from "../../length";
import { vinylDisplay } from "../../render/vinyl";
import type { Scene } from "../../render/hittest";
import type { ResolvedTheme } from "../../theme";
import * as ipc from "../../ipc";
import { cn } from "../../lib/cn";
import { PresetEditor } from "../PresetEditor";
import { type EditorMode, type PresetDraft } from "../presetDraft";
import {
  effectiveSettings,
  passRowLabel,
  presetIdForKey,
  presetPicker,
  type Caps,
  type Preset,
  type PresetLookup,
  type WeedDraft,
  type WeedField,
  type WeedRead,
} from "../viewmodel";
import { MiniMat } from "./MiniMat";
import { type PassIssue } from "./passFields";
import type { PassRow } from "./useCutPlan";

/** Material, grouping, weeding and the pass rows. Banners stay in the sheet, above whichever
 *  step is open, so a stale plan is visible where the rows are and where Start cut is. */
export function StepPasses({
  connected,
  ranges,
  rangesError,
  weed,
  unit,
  theme,
  scene,
  artboard,
  presets,
  presetsLoaded,
  presetListError,
  caps,
  draft,
  presetMode,
  presetDirty,
  presetFault,
  presetError,
  presetBusy,
  aiming,
  editorOpen,
  onToggleEditor,
  guardUnsaved,
  onSelectPreset,
  onNewPreset,
  onCopyPreset,
  onChangeDraft,
  onSavePreset,
  onDiscardPreset,
  onDeletePreset,
  grouping,
  setGrouping,
  replanning,
  replan,
  weedDraft,
  weedRead,
  changeWeed,
  onWeedEditing,
  planRows,
  planGrouping,
  skippedNotCut,
  presetLookup,
  issues,
  updateRow,
  setPassEnabled,
  movePass,
  movePassTo,
  onApplyMaterial,
}: {
  connected: ipc.DeviceInfo | null;
  ranges: ipc.SettingsRanges | null;
  rangesError: string | null;
  weed: WeedDraft | null;
  unit: LengthUnit;
  theme: ResolvedTheme;
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  presets: Preset[];
  presetsLoaded: boolean;
  presetListError: string | null;
  caps: Caps;
  draft: PresetDraft | null;
  presetMode: EditorMode;
  presetDirty: boolean;
  presetFault: string | null;
  presetError: string | null;
  presetBusy: boolean;
  aiming: boolean;
  editorOpen: boolean;
  onToggleEditor: () => void;
  guardUnsaved: (run: () => void) => void;
  onSelectPreset: (id: string) => void;
  onNewPreset: () => void;
  onCopyPreset: () => void;
  onChangeDraft: (patch: Partial<PresetDraft>) => void;
  onSavePreset: () => void;
  onDiscardPreset: () => void;
  onDeletePreset: () => void;
  grouping: ipc.Grouping;
  setGrouping: (grouping: ipc.Grouping) => void;
  replanning: boolean;
  replan: (mode?: ipc.Grouping) => void;
  weedDraft: WeedDraft;
  weedRead: WeedRead;
  changeWeed: (next: WeedDraft) => void;
  /** True while an inch field holds a fragment that is not a millimetre value yet. */
  onWeedEditing: (editing: boolean) => void;
  planRows: PassRow[] | null;
  planGrouping: ipc.Grouping | null;
  skippedNotCut: number | null;
  presetLookup: PresetLookup;
  issues: PassIssue[];
  updateRow: (i: number, patch: Partial<PassRow>) => void;
  setPassEnabled: (i: number, enabled: boolean) => void;
  movePass: (i: number, dir: -1 | 1) => void;
  movePassTo: (from: number, to: number) => void;
  onApplyMaterial: (presetId: string | null) => void;
}) {
  // Open, so the weed controls are on the step the moment it is. Collapsing is a look, not a gate.
  const [weedOpen, setWeedOpen] = useState(true);
  const [highlight, setHighlight] = useState<number | null>(null);
  // The inch text of a field that is not a finished number yet (`1.`). It stays out of the
  // millimetre draft: writing it would either snap the field or plan a length nobody typed.
  const [inchPending, setInchPending] = useState<Partial<Record<WeedField, string>>>({});
  useEffect(() => {
    setInchPending({});
    onWeedEditing(false);
    // Leaving the step drops the fragment with it. The sheet would otherwise keep Next
    // gated on an edit the fields are no longer showing.
    return () => onWeedEditing(false);
  }, [unit, onWeedEditing]);
  const rows = planRows ?? [];
  const showSpeed = caps.supportsSpeed;
  const showForce = caps.supportsForce;
  const skipped = skippedNotCut ?? 0;
  const material = materialValue(rows);

  // Not disabled while a replan is in flight, unlike the row controls: every keystroke replans,
  // and a field that locked itself mid-word would lose the caret. These fields are not rows, so
  // nothing typed here is discarded by the plan that lands.
  const setPending = (next: Partial<Record<WeedField, string>>) => {
    setInchPending(next);
    onWeedEditing(Object.keys(next).length > 0);
  };

  const weedInput = (field: WeedField, name: string, off: boolean) => {
    const error = weedRead.ok ? undefined : weedRead.errors[field];
    const label = `${name} (${unit})`;
    const pending = unit === "in" ? inchPending[field] : undefined;
    return (
      <label className="flex items-center gap-2 text-sm">
        {label}
        <input
          aria-label={`Weed ${label.toLowerCase()}`}
          type="number"
          step="0.1"
          value={pending !== undefined ? pending : unit === "mm" ? weedDraft[field] : lengthFromDraft(weedDraft[field])}
          disabled={off}
          aria-invalid={error !== undefined || pending !== undefined}
          onChange={(event) => {
            const typed = event.target.value;
            if (unit !== "mm" && !inchReady(typed)) {
              setPending({ ...inchPending, [field]: typed });
              return;
            }
            if (pending !== undefined) {
              const rest = { ...inchPending };
              delete rest[field];
              setPending(rest);
            }
            changeWeed({
              ...weedDraft,
              [field]: unit === "mm" ? typed : lengthToDraft(typed),
            });
          }}
          className={cn("h-8 w-24 rounded-md border bg-background px-2 text-sm", error || pending !== undefined ? "border-destructive" : "border-input")}
        />
      </label>
    );
  };

  const issueFor = (index: number, field: PassIssue["field"]) =>
    issues.find((issue) => issue.index === index && issue.field === field)?.message ?? null;

  const shown = highlight ?? (rows.length > 0 ? 0 : null);
  const shownRow = shown === null ? null : rows[shown] ?? null;
  const shownLabel = shownRow ? passRowLabel(shownRow.key, presetLookup, planGrouping ?? grouping) : null;

  return (
    <div className="space-y-5">
      {/* A preset belongs to one machine, so there is nothing to manage until this dialog is
          aimed at one, and nothing to manage safely until that cutter's own list has arrived.
          Withheld outright when the ranges could not be read — an editor that cannot say what a
          legal force is would offer saves the cut path then refuses. */}
      {connected === null ? null : rangesError !== null || presetListError !== null ? (
        <div className="text-sm text-destructive">
          Material presets are unavailable: {rangesError ?? presetListError}
        </div>
      ) : ranges !== null && presetsLoaded ? (
        <div className="space-y-3">
          <button type="button" className="text-sm font-medium text-accent-ink underline-offset-2 hover:underline" onClick={onToggleEditor}>
            Edit presets…
          </button>
          {editorOpen ? (
            <PresetEditor
              presets={presets}
              caps={caps}
              ranges={ranges}
              draft={draft}
              mode={presetMode}
              dirty={presetDirty}
              fault={presetFault}
              error={presetError}
              busy={presetBusy || aiming}
              onSelect={(id) => guardUnsaved(() => onSelectPreset(id))}
              onNew={() => guardUnsaved(onNewPreset)}
              onCopy={onCopyPreset}
              onChange={onChangeDraft}
              onSave={onSavePreset}
              onDiscard={onDiscardPreset}
              onDelete={onDeletePreset}
            />
          ) : null}
        </div>
      ) : (
        <div className="text-sm text-muted-foreground">Reading this cutter's presets…</div>
      )}

      {/* ponytail: one preset for every pass, in the slot a per-colour material picker (#310)
          will take. Speed and force stay in the row until that picker can move them under Advanced. */}
      <div>
        <div className="mb-1 text-2xs text-muted-foreground">Material preset</div>
        <select
          aria-label="Material preset"
          value={material}
          disabled={replanning || rows.length === 0}
          onChange={(event) => {
            const next = event.target.value;
            if (next === "mixed") return;
            onApplyMaterial(next === "no-preset" ? null : next.slice("preset:".length));
          }}
          className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
        >
          <option value="no-preset">No preset</option>
          {material === "mixed" ? <option value="mixed">Mixed</option> : null}
          {material.startsWith("preset:") && !presets.some((preset) => `preset:${preset.id}` === material) ? (
            <option value={material}>{material.slice("preset:".length)}</option>
          ) : null}
          {presets.map((preset) => (
            <option key={preset.id} value={`preset:${preset.id}`}>{preset.name}</option>
          ))}
        </select>
      </div>

      <label className="flex items-center gap-2 text-sm">
        Group passes by
        <select
          aria-label="Group passes by"
          value={grouping}
          disabled={replanning}
          onChange={(event) => {
            const next = event.target.value as ipc.Grouping;
            setGrouping(next);
            // Replanned at once: the rows, the skipped count, the travel and the preview are
            // all derived from the mode, and showing the previous mode's rows beside the new
            // selection is the disagreement this avoids.
            replan(next);
          }}
          className="h-9 rounded-md border border-input bg-background px-2 text-sm"
        >
          <option value="Color">Colour (stroke, else fill)</option>
          <option value="Stroke">Stroke colour</option>
          <option value="Fill">Fill colour</option>
          <option value="Preset">Material preset</option>
          <option value="Single">One pass over everything</option>
        </select>
      </label>

      <div>
        <div
          className="grid items-center gap-3 px-2 pb-1.5 text-2xs uppercase tracking-wide text-muted-foreground"
          style={{ gridTemplateColumns: columns(showSpeed, showForce) }}
        >
          <span />
          <span>Colour pass (cut order)</span>
          {showSpeed ? <span>Speed{ranges ? ` (${ranges.speed.min}–${ranges.speed.max})` : ""}</span> : null}
          {showForce ? <span>Force level</span> : null}
          <span>Passes</span>
          <span>On</span>
        </div>
        <div className="space-y-1.5">
          {rows.map((row, i) => {
            const effective = effectiveSettings(row, presetLookup);
            const label = passRowLabel(row.key, presetLookup, planGrouping ?? grouping);
            const swatch = label.swatch !== null ? vinylDisplay(label.swatch, theme).swatch : null;
            const picker = presetPicker(row.presetId, presetLookup);
            return (
              <div
                key={row.key}
                data-testid="cut-pass-row"
                onMouseEnter={() => setHighlight(i)}
                onFocusCapture={() => setHighlight(i)}
                onDragOver={(event) => {
                  // The index lives on the drag, not in state: a drop can land before the
                  // drag-start render commits, and a dragover that waits for that state
                  // never accepts the drop.
                  if (replanning || !event.dataTransfer.types.includes("text/plain")) return;
                  event.preventDefault();
                }}
                onDrop={(event) => {
                  if (replanning) return;
                  const from = Number(event.dataTransfer.getData("text/plain"));
                  if (!Number.isInteger(from)) return;
                  movePassTo(from, i);
                }}
                className={cn(
                  "grid items-start gap-3 rounded-lg border border-border bg-card px-2 py-2",
                  highlight === i && "border-foreground/40",
                  !row.enabled && "opacity-55",
                )}
                style={{ gridTemplateColumns: columns(showSpeed, showForce) }}
              >
                <button
                  type="button"
                  aria-label={`Reorder pass ${i + 1}`}
                  draggable={!replanning}
                  disabled={replanning}
                  className="mt-1 cursor-grab text-muted-foreground disabled:cursor-not-allowed"
                  onDragStart={(event) => {
                    event.dataTransfer.setData("text/plain", String(i));
                    event.dataTransfer.effectAllowed = "move";
                  }}
                  onKeyDown={(event) => {
                    if (!event.altKey || replanning) return;
                    if (event.key === "ArrowUp") {
                      event.preventDefault();
                      movePass(i, -1);
                    } else if (event.key === "ArrowDown") {
                      event.preventDefault();
                      movePass(i, 1);
                    }
                  }}
                >
                  <GripVertical className="size-4" aria-hidden />
                </button>
                <div className="min-w-0">
                  <div className="flex h-8 items-center gap-2">
                    <div className="grid size-5 place-items-center rounded-full bg-subtle text-2xs tabular">{i + 1}</div>
                    {swatch !== null ? (
                      <span className="inline-block size-4 rounded-sm border border-foreground/20" style={{ background: swatch }} />
                    ) : null}
                    {label.text !== null ? <span className="truncate text-sm">{label.text}</span> : null}
                    <span className="text-xs text-muted-foreground">{row.shapeCount} shape{row.shapeCount === 1 ? "" : "s"}</span>
                  </div>
                  <select
                    aria-label={`Preset for pass ${i + 1}`}
                    disabled={replanning}
                    value={picker.selected}
                    onChange={(event) => updateRow(i, { presetId: presetIdForKey(event.target.value as ipc.PassKey) })}
                    className="mt-1 h-8 w-full rounded-md border border-input bg-background px-2 text-xs"
                  >
                    {picker.options.map((option) => (
                      <option key={option.value} value={option.value}>{option.label}</option>
                    ))}
                  </select>
                </div>
                {showSpeed ? (
                  <LevelField
                    label={`Speed for pass ${i + 1}`}
                    value={effective.speed}
                    range={ranges?.speed ?? null}
                    message={issueFor(i, "speed")}
                    disabled={replanning}
                    onChange={(speed) => updateRow(i, { speed })}
                  />
                ) : null}
                {showForce ? (
                  <LevelField
                    label={`Force for pass ${i + 1}`}
                    value={effective.force}
                    range={ranges?.force ?? null}
                    message={issueFor(i, "force")}
                    disabled={replanning}
                    onChange={(force) => updateRow(i, { force })}
                  />
                ) : null}
                <LevelField
                  label={`Repeat count for pass ${i + 1}`}
                  value={effective.repeatCount}
                  range={ranges?.repeatCount ?? null}
                  message={issueFor(i, "repeat")}
                  disabled={replanning}
                  onChange={(repeatCount) => updateRow(i, { repeatCount })}
                />
                <label className="relative mt-1 inline-flex h-5 w-9 shrink-0">
                  <input
                    type="checkbox"
                    aria-label="Enabled"
                    checked={row.enabled}
                    disabled={replanning}
                    onChange={(event) => setPassEnabled(i, event.target.checked)}
                    className="peer absolute inset-0 m-0 size-full cursor-pointer appearance-none rounded-full border border-input bg-muted opacity-100 checked:bg-primary disabled:cursor-not-allowed"
                  />
                  <span aria-hidden className="pointer-events-none absolute left-0.5 top-0.5 size-4 rounded-full bg-background shadow-sm transition-transform peer-checked:translate-x-4" />
                </label>
              </div>
            );
          })}
        </div>
      </div>

      {shownRow && shownLabel ? (
        <div className="flex items-center gap-4 rounded-lg border border-border bg-card p-3">
          <MiniMat scene={scene} artboard={artboard} passes={rows} highlight={shown} theme={theme} />
          <div className="text-sm">
            <div className="flex items-center gap-2 font-medium">
              {shownLabel.swatch !== null ? (
                <span className="inline-block size-3 rounded-sm" style={{ background: vinylDisplay(shownLabel.swatch, theme).swatch }} />
              ) : null}
              {shownLabel.text ?? "This pass"}
            </div>
            <p className="mt-1 text-xs text-muted-foreground">
              {shownRow.shapeCount} shape{shownRow.shapeCount === 1 ? "" : "s"} cut in this pass. Hover or click a pass to see its shapes on the mat.
            </p>
          </div>
        </div>
      ) : null}

      <div className="rounded-lg border border-border">
        <button
          type="button"
          className="flex w-full items-center justify-between px-3 py-2.5 text-sm font-medium"
          aria-expanded={weedOpen}
          onClick={() => setWeedOpen((open) => !open)}
        >
          Weeding
          <ChevronDown className={cn("size-4 transition-transform", weedOpen && "rotate-180")} aria-hidden />
        </button>
        {weedOpen ? (
          <fieldset className="space-y-3 border-t border-border px-3 py-3">
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                aria-label="Weed border"
                checked={weedDraft.border}
                disabled={weed === null && ranges === null}
                title={weed !== null || ranges !== null ? undefined
                  : rangesError !== null ? `Weeding is unavailable: ${rangesError}`
                  : "Waiting for the weed defaults"}
                onChange={(event) => changeWeed({ ...weedDraft, border: event.target.checked })}
              />
              Border
            </label>
            {weedInput("margin", "Margin", !weedDraft.border)}
            <label className="flex items-center gap-2 text-sm">
              Lines
              <select
                aria-label="Weed lines"
                value={weedDraft.lines}
                disabled={!weedDraft.border}
                onChange={(event) => changeWeed({ ...weedDraft, lines: event.target.value as ipc.WeedLines })}
                className="h-8 rounded-md border border-input bg-background px-2 text-sm"
              >
                <option value="None">None</option>
                <option value="Horizontal">Horizontal</option>
                <option value="Vertical">Vertical</option>
                <option value="Both">Both</option>
              </select>
            </label>
            {weedInput("spacing", "Spacing", !weedDraft.border || weedDraft.lines === "None")}
            {weedInput("clearance", "Clearance", !weedDraft.border || weedDraft.lines === "None")}
            {!weedRead.ok ? (
              <div role="alert" className="text-sm text-destructive">
                {Object.values(weedRead.errors).map((message) => weedMessage(message, unit)).join(". ")}.
              </div>
            ) : null}
          </fieldset>
        ) : null}
      </div>

      <div className="text-xs text-muted-foreground">
        Not cut: {skipped} shape{skipped === 1 ? "" : "s"} marked No Cut
      </div>
    </div>
  );
}

function columns(showSpeed: boolean, showForce: boolean): string {
  return ["20px", "minmax(0,1fr)", showSpeed ? "104px" : null, showForce ? "104px" : null, "72px", "44px"]
    .filter((column) => column !== null)
    .join(" ");
}

function materialValue(rows: PassRow[]): string {
  if (rows.length === 0) return "no-preset";
  const first = rows[0]?.presetId ?? null;
  if (!rows.every((row) => row.presetId === first)) return "mixed";
  return first === null ? "no-preset" : `preset:${first}`;
}

/** A finished inch amount. `1.` is still being typed: `Number` would call it 1 and the field would snap. */
function settledInch(typed: string): number | null {
  const trimmed = typed.trim();
  if (!/^-?\d+(\.\d+)?$/.test(trimmed)) return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : null;
}

/** True when the inch field can be written into the millimetre draft. Empty is written. */
export function inchReady(typed: string): boolean {
  return typed.trim() === "" || settledInch(typed) !== null;
}

/** Inches are a view of the millimetre draft. A fragment that is not a number yet is left as typed. */
export function lengthFromDraft(stored: string): string {
  if (stored.trim() === "") return "";
  const value = settledInch(stored);
  return value === null ? stored : formatLength(value, "in");
}

export function lengthToDraft(typed: string): string {
  if (typed.trim() === "") return "";
  const value = settledInch(typed);
  return value === null ? typed : String(unitToMm(value, "in"));
}

/** The weed sentences are written in millimetres, which is what the draft stores. An inch field quotes the same bounds in inches. */
export function weedMessage(message: string, unit: LengthUnit): string {
  if (unit === "mm") return message;
  return message
    .replace(/(\d+(?:\.\d+)?)–(\d+(?:\.\d+)?) mm/g, (_, min: string, max: string) =>
      `${formatLength(Number(min), "in")}–${formatLength(Number(max), "in")} in`)
    .replace(/ of mm/g, " of in");
}

function LevelField({
  label,
  value,
  range,
  message,
  disabled,
  onChange,
}: {
  label: string;
  value: number | null;
  range: { min: number; max: number } | null;
  message: string | null;
  disabled: boolean;
  onChange: (value: number | null) => void;
}) {
  const hintId = useId();
  return (
    <div>
      <input
        aria-label={label}
        aria-invalid={message !== null}
        aria-describedby={hintId}
        type="number"
        disabled={disabled}
        value={value ?? ""}
        onChange={(event) => onChange(event.target.value === "" ? null : Number(event.target.value))}
        className={cn(
          "h-8 w-full rounded-md border bg-background px-2 text-sm tabular",
          message ? "border-destructive" : "border-input",
        )}
      />
      <div id={hintId} className={cn("mt-0.5 text-2xs tabular", message ? "text-destructive" : "text-muted-foreground")}>
        {message ?? (range ? `${range.min}–${range.max}` : "")}
      </div>
    </div>
  );
}
