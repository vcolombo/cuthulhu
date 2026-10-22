// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useId, useRef, useState } from "react";
import { ChevronDown, GripVertical } from "lucide-react";
import type { LengthUnit } from "../../length";
import { formatLength, MM_PER_IN, unitToMm } from "../../length";
import { vinylDisplay } from "../../render/vinyl";
import type { Scene } from "../../render/hittest";
import type { ResolvedTheme } from "../../theme";
import * as ipc from "../../ipc";
import { cn } from "../../lib/cn";
import { PresetEditor } from "../PresetEditor";
import { type EditorMode, type PresetDraft } from "../presetDraft";
import {
  droppedPassIndex,
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
  // The last payload a row received, including one that is not an index. An untouched order
  // is also what a drop that never arrived looks like, so the row has to show which it was.
  const [dropped, setDropped] = useState<{ index: number; payload: string } | null>(null);
  // Inch text that has not been committed. A number input never reports `1.` — Chromium
  // turns both the `1` and the `.` into `"1"` — so the field is text, and the millimetre
  // draft is written only once the text is a finished amount. Writing `1` on the keystroke
  // would format it back as `1.00` and the decimal could not be typed.
  const [inchPending, setInchPending] = useState<Partial<Record<WeedField, string>>>({});
  const inchPendingRef = useRef(inchPending);
  const weedDraftRef = useRef(weedDraft);
  const changeWeedRef = useRef(changeWeed);
  const rangesRef = useRef(ranges);
  inchPendingRef.current = inchPending;
  weedDraftRef.current = weedDraft;
  changeWeedRef.current = changeWeed;
  rangesRef.current = ranges;
  useEffect(() => {
    inchPendingRef.current = {};
    setInchPending({});
    onWeedEditing(false);
    return () => {
      // A finished inch that never blurred still has to land, or leaving the step cuts the
      // margin from before the keystrokes. A fragment (`1.`) is dropped: it is not a length,
      // and Next was held for as long as the field was showing it.
      const pending = inchPendingRef.current;
      const draft = { ...weedDraftRef.current };
      let commit = false;
      for (const field of Object.keys(pending) as WeedField[]) {
        const typed = pending[field];
        if (typed === undefined) continue;
        const next = inchCommit(draft[field], typed, rangeFor(field, rangesRef.current));
        if (next === null) continue;
        draft[field] = next;
        commit = true;
      }
      inchPendingRef.current = {};
      if (commit) changeWeedRef.current(draft);
      onWeedEditing(false);
    };
  }, [unit, onWeedEditing]);
  const rows = planRows ?? [];
  const showSpeed = caps.supportsSpeed;
  const showForce = caps.supportsForce;
  const skipped = skippedNotCut ?? 0;
  const material = materialValue(rows);

  // Not disabled while a replan is in flight, unlike the row controls. A millimetre keystroke
  // replans at once, and a field that locked itself mid-word would lose the caret. An inch
  // stays as typed until blur, so the draft is not a length the field has already stopped showing.
  const setPending = (next: Partial<Record<WeedField, string>>) => {
    inchPendingRef.current = next;
    setInchPending(next);
    onWeedEditing(Object.keys(next).length > 0);
  };

  const commitInch = (field: WeedField, typed: string) => {
    // A fragment stays on screen. Clearing it would show the previous length and let Next through.
    if (!inchReady(typed)) return;
    const rest = { ...inchPendingRef.current };
    delete rest[field];
    const next = inchCommit(weedDraftRef.current[field], typed, rangeFor(field, ranges));
    setPending(rest);
    if (next === null) return;
    changeWeed({ ...weedDraftRef.current, [field]: next });
  };

  const weedInput = (field: WeedField, name: string, off: boolean) => {
    const error = weedRead.ok ? undefined : weedRead.errors[field];
    const label = `${name} (${unit})`;
    const pending = unit === "in" ? inchPending[field] : undefined;
    // A fragment is invalid on its own. A committed draft error stays hidden while a newer
    // keystroke is still on screen, or the alert would describe a length the field is not showing.
    const fragment = pending !== undefined && !inchReady(pending);
    const showError = fragment || (error !== undefined && pending === undefined);
    return (
      <label className="flex items-center gap-2 text-sm">
        {label}
        <input
          aria-label={`Weed ${label.toLowerCase()}`}
          type="text"
          inputMode="decimal"
          autoComplete="off"
          value={pending !== undefined ? pending : unit === "mm" ? weedDraft[field] : lengthFromDraft(weedDraft[field], rangeFor(field, ranges))}
          disabled={off}
          aria-invalid={showError}
          onChange={(event) => {
            const typed = event.target.value;
            if (unit === "mm") {
              changeWeed({ ...weedDraft, [field]: typed });
              return;
            }
            setPending({ ...inchPendingRef.current, [field]: typed });
          }}
          onBlur={() => {
            if (unit !== "in") return;
            const typed = inchPendingRef.current[field];
            if (typed === undefined) return;
            commitInch(field, typed);
          }}
          className={cn("h-8 w-24 rounded-md border bg-background px-2 text-sm", showError ? "border-destructive" : "border-input")}
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
                data-drop-payload={dropped?.index === i ? dropped.payload : undefined}
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
                  const payload = event.dataTransfer.getData("text/plain");
                  setDropped({ index: i, payload });
                  if (replanning) return;
                  const from = droppedPassIndex(payload);
                  if (from === null) return;
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
              <WeedAlert errors={weedRead.errors} pending={inchPending} unit={unit} />
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

/** Inches are a view of the millimetre draft. A fragment that is not a number yet is left as typed.
 *  When the ordinary hundredth would fall outside `range`, the next hundredth inward is shown:
 *  50 mm is 1.97 in, and 1.97 in is past 50 mm. */
export function lengthFromDraft(stored: string, range?: { min: number; max: number } | null): string {
  if (stored.trim() === "") return "";
  const mm = settledInch(stored);
  if (mm === null) return stored;
  const shown = formatLength(mm, "in");
  if (!range || mm < range.min || mm > range.max) return shown;
  return inchInside(shown, range);
}

/** What blur writes. `null` keeps the stored draft: a fragment is not a length, and committing
 *  the inches already on screen must not walk a legal millimetre onto a different hundredth. */
export function inchCommit(stored: string, typed: string, range: { min: number; max: number } | null): string | null {
  if (!inchReady(typed)) return null;
  if (typed.trim() === "") return "";
  const storedMm = settledInch(stored);
  const shown = lengthFromDraft(stored, range);
  const storedOk = storedMm !== null && (range === null || (storedMm >= range.min && storedMm <= range.max));
  if (storedOk && shown !== "" && Number(typed) === Number(shown)) return null;
  return lengthToDraft(typed);
}

function rangeFor(field: WeedField, ranges: ipc.SettingsRanges | null): { min: number; max: number } | null {
  if (!ranges) return null;
  if (field === "margin") return ranges.weed.margin_mm;
  if (field === "spacing") return ranges.weed.spacing_mm;
  return ranges.weed.clearance_mm;
}

function inchInside(shown: string, range: { min: number; max: number }): string {
  const back = unitToMm(Number(shown), "in");
  if (back >= range.min && back <= range.max) return shown;
  const hundredths = Math.round(Number(shown) * 100);
  const step = back > range.max ? -1 : 1;
  for (let h = hundredths + step; h >= 0 && h <= 10000; h += step) {
    const next = (h / 100).toFixed(2);
    const mmBack = unitToMm(Number(next), "in");
    if (mmBack >= range.min && mmBack <= range.max) return next;
  }
  return shown;
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
      `${inchBound(Number(min), "min")}–${inchBound(Number(max), "max")} in`)
    .replace(/ of mm/g, " of in");
}

/** Inch hundredths that `unitToMm` still places inside the millimetre bound. `formatLength`
 *  rounds 50 mm out to 1.97 in, and 1.97 in is 50.038 mm, which the same check then refuses. */
function inchBound(mm: number, edge: "min" | "max"): string {
  const hundredths = (mm / MM_PER_IN) * 100;
  const nearest = Math.round(hundredths);
  const exact = Math.abs(hundredths - nearest) < 1e-6 ? nearest : hundredths;
  const stepped = edge === "min" ? Math.ceil(exact) : Math.floor(exact);
  return (stepped / 100).toFixed(2);
}

function WeedAlert({
  errors,
  pending,
  unit,
}: {
  errors: Partial<Record<WeedField, string>>;
  pending: Partial<Record<WeedField, string>>;
  unit: LengthUnit;
}) {
  const text = (Object.entries(errors) as [WeedField, string][])
    .filter(([field]) => pending[field] === undefined)
    .map(([, message]) => weedMessage(message, unit))
    .join(". ");
  if (text === "") return null;
  return (
    <div role="alert" className="text-sm text-destructive">
      {text}.
    </div>
  );
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
