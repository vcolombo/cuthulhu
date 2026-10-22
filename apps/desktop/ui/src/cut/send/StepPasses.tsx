// SPDX-License-Identifier: GPL-3.0-or-later
import type { ReactNode } from "react";
import { vinylDisplay } from "../../render/vinyl";
import type { ResolvedTheme } from "../../theme";
import * as ipc from "../../ipc";
import { PresetEditor } from "../PresetEditor";
import { type EditorMode, type PresetDraft } from "../presetDraft";
import {
  effectiveSettings,
  fieldDisabled,
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
import { btn } from "./chrome";
import type { PassRow } from "./useCutPlan";

/** Material, grouping, weeding and the pass rows. `mid` is the banners the shell still owns
 *  (unsaved preset, machine mismatch, already-accepted, stale plan): they sit between the editor
 *  and the grouping control, and moving them would change the scroll. */
export function StepPasses({
  mid,
  connected,
  ranges,
  rangesError,
  weed,
  theme,
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
  planRows,
  planGrouping,
  skippedNotCut,
  presetLookup,
  updateRow,
  setPassEnabled,
  movePass,
}: {
  mid: ReactNode;
  connected: ipc.DeviceInfo | null;
  ranges: ipc.SettingsRanges | null;
  rangesError: string | null;
  weed: WeedDraft | null;
  theme: ResolvedTheme;
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
  planRows: PassRow[] | null;
  planGrouping: ipc.Grouping | null;
  skippedNotCut: number | null;
  presetLookup: PresetLookup;
  updateRow: (i: number, patch: Partial<PassRow>) => void;
  setPassEnabled: (i: number, enabled: boolean) => void;
  movePass: (i: number, dir: -1 | 1) => void;
}) {
  // Not disabled while a replan is in flight, unlike the row controls: every keystroke replans,
  // and a field that locked itself mid-word would lose the caret. These fields are not rows, so
  // nothing typed here is discarded by the plan that lands.
  const weedInput = (field: WeedField, label: string, off: boolean, range: ipc.SettingRange | undefined) => {
    const error = weedRead.ok ? undefined : weedRead.errors[field];
    return (
      <label style={{ display: "flex", alignItems: "center", gap: 4 }}>
        {label}
        <input
          // Contains the visible text, so a voice user saying "Margin" reaches the field.
          aria-label={`Weed ${label.toLowerCase()}`}
          type="number"
          step="0.1"
          value={weedDraft[field]}
          disabled={off}
          aria-invalid={error !== undefined}
          title={range ? `${range.min}–${range.max} mm` : undefined}
          style={{ width: 56, ...(error ? { borderColor: "var(--cut)" } : {}) }}
          onChange={(e) => changeWeed({ ...weedDraft, [field]: e.target.value })}
        />
      </label>
    );
  };

  const skipped = skippedNotCut ?? 0;

  return (
    <>
      {/* A preset belongs to one machine, so there is nothing to manage until this dialog is
          aimed at one, and nothing to manage safely until that cutter's own list has arrived:
          what is already stored is what a new entry's name and id have to avoid. Withheld
          outright when the ranges could not be read — an editor that cannot say what a legal
          force is would offer saves the cut path then refuses. */}
      {connected === null ? null : rangesError !== null || presetListError !== null ? (
        <div style={{ fontSize: 12, color: "var(--cut)" }}>
          Material presets are unavailable: {rangesError ?? presetListError}
        </div>
      ) : ranges !== null && presetsLoaded ? (
        <PresetEditor
          presets={presets}
          caps={caps}
          ranges={ranges}
          draft={draft}
          mode={presetMode}
          dirty={presetDirty}
          fault={presetFault}
          error={presetError}
          // An aim change is as much a write as a save is, as far as this section is concerned:
          // both end with the draft replaced by what the backend says.
          busy={presetBusy || aiming}
          onSelect={(id) => guardUnsaved(() => onSelectPreset(id))}
          onNew={() => guardUnsaved(onNewPreset)}
          onCopy={onCopyPreset}
          onChange={onChangeDraft}
          onSave={onSavePreset}
          onDiscard={onDiscardPreset}
          onDelete={onDeletePreset}
        />
      ) : (
        <div style={{ fontSize: 12, color: "var(--muted-foreground)" }}>Reading this cutter's presets…</div>
      )}

      {mid}

      <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
        Group passes by
        <select
          aria-label="Group passes by"
          value={grouping}
          disabled={replanning}
          onChange={(e) => {
            const next = e.target.value as ipc.Grouping;
            setGrouping(next);
            // Replanned at once: the rows, the skipped count, the travel and the preview are
            // all derived from the mode, and showing the previous mode's rows beside the new
            // selection is the disagreement this avoids.
            replan(next);
          }}
        >
          <option value="Color">Colour (stroke, else fill)</option>
          <option value="Stroke">Stroke colour</option>
          <option value="Fill">Fill colour</option>
          <option value="Preset">Material preset</option>
          <option value="Single">One pass over everything</option>
        </select>
      </label>

      <fieldset style={{ border: "1px solid var(--border)", padding: 8, display: "flex", flexWrap: "wrap", alignItems: "center", gap: 10, fontSize: 12 }}>
        <legend>Weeding</legend>
        <label style={{ display: "flex", alignItems: "center", gap: 4 }}>
          <input
            type="checkbox"
            aria-label="Weed border"
            checked={weedDraft.border}
            // Until the defaults arrive there is nothing to fill the fields with, and a border
            // ticked then would start from an empty margin.
            disabled={weed === null && ranges === null}
            title={weed !== null || ranges !== null ? undefined
              : rangesError !== null ? `Weeding is unavailable: ${rangesError}`
              : "Waiting for the weed defaults"}
            onChange={(e) => changeWeed({ ...weedDraft, border: e.target.checked })}
          />
          Border
        </label>
        {weedInput("margin", "Margin (mm)", !weedDraft.border, ranges?.weed.margin_mm)}
        {/* Lines without a border would stop in the open sheet, so they wait for one. */}
        <label style={{ display: "flex", alignItems: "center", gap: 4 }}>
          Lines
          <select
            aria-label="Weed lines"
            value={weedDraft.lines}
            disabled={!weedDraft.border}
            onChange={(e) => changeWeed({ ...weedDraft, lines: e.target.value as ipc.WeedLines })}
          >
            <option value="None">None</option>
            <option value="Horizontal">Horizontal</option>
            <option value="Vertical">Vertical</option>
            <option value="Both">Both</option>
          </select>
        </label>
        {weedInput("spacing", "Spacing (mm)", !weedDraft.border || weedDraft.lines === "None", ranges?.weed.spacing_mm)}
        {weedInput("clearance", "Clearance (mm)", !weedDraft.border || weedDraft.lines === "None", ranges?.weed.clearance_mm)}
        {!weedRead.ok ? (
          <div role="alert" style={{ color: "var(--cut)", width: "100%" }}>
            {Object.values(weedRead.errors).join(". ")}.
          </div>
        ) : null}
      </fieldset>

      {/* Every row control below is unavailable while `replanning`: these rows belong to the
          previous grouping, and the arriving plan replaces them wholesale — an edit accepted
          in that window is discarded without a trace (Greptile reproduced exactly that). */}
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {(planRows ?? []).map((row, i) => {
          const eff = effectiveSettings(row, presetLookup);
          const speedDisabled = fieldDisabled("speed", caps);
          const forceDisabled = fieldDisabled("force", caps);
          // A pass keyed on a preset has no swatch to be recognised by, so
          // the row says what it holds instead.
          const label = passRowLabel(row.key, presetLookup, planGrouping ?? grouping);
          const swatch = label.swatch !== null ? vinylDisplay(label.swatch, theme).swatch : null;
          /** Every option the material picker offers, keyed as this row's `PassKey` grammar keys
           *  a pass: a bare-id picker has to spend the empty string as its "no preset" sentinel,
           *  and an id can be any string an operator typed, that one included. */
          const picker = presetPicker(row.presetId, presetLookup);
          return (
            <div
              key={row.key}
              data-testid="cut-pass-row"
              style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12, border: "1px solid var(--border)", padding: 6 }}
            >
              {swatch !== null ? (
                <span
                  style={{
                    width: 12,
                    height: 12,
                    display: "inline-block",
                    background: swatch,
                  }}
                />
              ) : null}
              {label.text !== null ? <span>{label.text}</span> : null}
              <span>{row.shapeCount} shape(s)</span>
              <label>
                <input type="checkbox" disabled={replanning} checked={row.enabled} onChange={(e) => setPassEnabled(i, e.target.checked)} />
                Enabled
              </label>
              <select
                aria-label={`Preset for pass ${i + 1}`}
                disabled={replanning}
                value={picker.selected}
                onChange={(e) => updateRow(i, { presetId: presetIdForKey(e.target.value) })}
              >
                {picker.options.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              <input
                aria-label={`Speed for pass ${i + 1}`}
                type="number"
                disabled={speedDisabled || replanning}
                value={eff.speed ?? ""}
                placeholder="speed"
                onChange={(e) => updateRow(i, { speed: e.target.value === "" ? null : Number(e.target.value) })}
                style={{ width: 60 }}
              />
              <input
                aria-label={`Force for pass ${i + 1}`}
                type="number"
                disabled={forceDisabled || replanning}
                value={eff.force ?? ""}
                placeholder="force"
                onChange={(e) => updateRow(i, { force: e.target.value === "" ? null : Number(e.target.value) })}
                style={{ width: 60 }}
              />
              <input
                aria-label={`Repeat count for pass ${i + 1}`}
                type="number"
                disabled={replanning}
                min={ranges?.repeatCount.min}
                max={ranges?.repeatCount.max}
                value={eff.repeatCount ?? ""}
                placeholder="repeat"
                onChange={(e) => updateRow(i, { repeatCount: e.target.value === "" ? null : Number(e.target.value) })}
                style={{ width: 50 }}
              />
              {speedDisabled || forceDisabled ? <span style={{ color: "var(--muted-foreground)" }}>set on the Puma's panel</span> : null}
              <button style={btn} onClick={() => movePass(i, -1)} disabled={replanning || i === 0}>
                Up
              </button>
              <button style={btn} onClick={() => movePass(i, 1)} disabled={replanning || i === (planRows?.length ?? 0) - 1}>
                Down
              </button>
            </div>
          );
        })}
      </div>

      <div style={{ fontSize: 12, color: "var(--muted-foreground)" }}>
        Not cut: {skipped} shape{skipped === 1 ? "" : "s"} marked No Cut
      </div>
    </>
  );
}
