// SPDX-License-Identifier: GPL-3.0-or-later
import type { CSSProperties } from "react";
import type { SettingsRanges } from "../ipc";
import { presetPreview, type EditorMode, type PresetDraft } from "./presetDraft";
import type { Caps, Preset } from "./viewmodel";

/** Every decision this section makes is the dialog's: it owns the draft, the writes and the
 *  unsaved-changes question, because closing the dialog and changing cutter are its actions too.
 *  What is left here is the shape of the controls. */
type Props = {
  presets: Preset[];
  caps: Caps;
  ranges: SettingsRanges;
  draft: PresetDraft | null;
  mode: EditorMode;
  dirty: boolean;
  /** What refuses the draft as typed, from `draftFault` — shown, and what withholds Save. */
  fault: string | null;
  /** A write the backend refused, in its own words. Named rather than swallowed, and the draft it
   *  refused is still on screen to be corrected. */
  error: string | null;
  /** A write is out; a second press would race it. */
  busy: boolean;
  onSelect: (id: string) => void;
  onNew: () => void;
  onCopy: () => void;
  onChange: (patch: Partial<PresetDraft>) => void;
  onSave: () => void;
  onDiscard: () => void;
  onDelete: () => void;
};

const btn: CSSProperties = {
  background: "var(--panel)",
  color: "var(--text)",
  border: "1px solid var(--border)",
  padding: "4px 10px",
  cursor: "pointer",
};

const rowStyle: CSSProperties = { display: "flex", alignItems: "center", gap: 8, fontSize: 12 };

const numeric = (value: string): number | null => (value === "" ? null : Number(value));

export function PresetEditor({
  presets,
  caps,
  ranges,
  draft,
  mode,
  dirty,
  fault,
  error,
  busy,
  onSelect,
  onNew,
  onCopy,
  onChange,
  onSave,
  onDiscard,
  onDelete,
}: Props) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <div style={rowStyle}>
        <strong>Material presets</strong>
        <select
          aria-label="Preset to manage"
          disabled={busy}
          // The unsaved entry is in the list rather than beside it: a picker showing the previously
          // selected preset while its fields hold a new one is a rename waiting to happen.
          value={mode === "creating" ? "" : (draft?.id ?? "")}
          onChange={(e) => onSelect(e.target.value)}
        >
          {mode === "creating" ? <option value="">New preset (unsaved)</option> : null}
          {draft === null ? <option value="">Choose a preset…</option> : null}
          {presets.map((p) => (
            <option key={p.id} value={p.id}>
              {p.builtin ? `${p.name} (built-in)` : p.name}
            </option>
          ))}
        </select>
        <button aria-label="New preset" style={btn} disabled={busy} onClick={onNew}>
          New
        </button>
        {mode === "builtin" || mode === "user" ? (
          // A builtin is copied, never edited: an entry saved under its pair shadows it in
          // `load_presets`, and nothing in this dialog could hand the shipped settings back.
          //
          // Withheld while the draft is dirty, because a copy is written from the *stored* entry:
          // offered there it would either silently drop the operator's edit or copy a version of
          // the preset that no longer exists (Codex on PR #264). Save or Discard first.
          <button
            aria-label={mode === "builtin" ? "Save as Copy" : "Duplicate preset"}
            style={btn}
            disabled={busy || dirty}
            onClick={onCopy}
          >
            {mode === "builtin" ? "Save as Copy" : "Duplicate"}
          </button>
        ) : null}
        {mode === "builtin" ? <span style={{ color: "var(--muted)" }}>built-in — read-only</span> : null}
      </div>

      {draft === null ? (
        <div style={{ fontSize: 12, color: "var(--muted)" }}>
          Choose a preset to edit, or add one of your own.
        </div>
      ) : (
        <>
          <div style={rowStyle}>
            <label>
              Name
              <input
                aria-label="Preset name"
                type="text"
                disabled={mode === "builtin" || busy}
                value={draft.name}
                onChange={(e) => onChange({ name: e.target.value })}
                style={{ width: 160, marginLeft: 4 }}
              />
            </label>
            <label>
              Speed
              <input
                aria-label="Preset speed"
                type="number"
                min={ranges.speed.min}
                max={ranges.speed.max}
                disabled={mode === "builtin" || busy}
                value={draft.speed ?? ""}
                onChange={(e) => onChange({ speed: numeric(e.target.value) })}
                style={{ width: 60, marginLeft: 4 }}
              />
            </label>
            <label>
              Force
              <input
                aria-label="Preset force"
                type="number"
                min={ranges.force.min}
                max={ranges.force.max}
                disabled={mode === "builtin" || busy}
                value={draft.force ?? ""}
                onChange={(e) => onChange({ force: numeric(e.target.value) })}
                style={{ width: 60, marginLeft: 4 }}
              />
            </label>
            <label>
              Repeat
              <input
                aria-label="Preset repeat count"
                type="number"
                min={ranges.repeatCount.min}
                max={ranges.repeatCount.max}
                disabled={mode === "builtin" || busy}
                value={draft.repeatCount ?? ""}
                onChange={(e) => onChange({ repeatCount: numeric(e.target.value) })}
                style={{ width: 50, marginLeft: 4 }}
              />
            </label>
          </div>

          {/* Speed and force stay editable on a cutter that takes them from its own panel, unlike a
              pass row's: the preset is stored for that machine and outlives this session, and a
              disabled field is one an operator cannot correct a value out of. What the machine
              ignores is said here instead. */}
          <div data-testid="preset-preview" style={{ fontSize: 12, color: "var(--muted)" }}>
            {presetPreview(draft, caps)}
          </div>

          {mode === "builtin" ? null : (
            <div style={rowStyle}>
              <button
                aria-label="Save preset"
                style={btn}
                disabled={busy || !dirty || fault !== null}
                onClick={onSave}
              >
                Save
              </button>
              {dirty ? (
                <button aria-label="Discard preset changes" style={btn} disabled={busy} onClick={onDiscard}>
                  Discard changes
                </button>
              ) : null}
              {mode === "user" ? (
                // Also withheld while dirty: a delete replaces the draft with a neighbour's, and
                // offered there it discards a typed edit without asking — the one thing the
                // unsaved-changes decision exists to prevent (Codex on PR #264).
                <button aria-label="Delete preset" style={btn} disabled={busy || dirty} onClick={onDelete}>
                  Delete
                </button>
              ) : null}
            </div>
          )}

          {/* One line for both, because they answer the same question — why this draft is not
              saved. The backend's refusal wins: it is the newer fact, and it is about a write that
              was actually attempted. */}
          {(error ?? fault) !== null ? (
            <div data-testid="preset-error" role="alert" style={{ fontSize: 12, color: "var(--cut)" }}>
              {error ?? fault}
            </div>
          ) : null}
        </>
      )}
    </div>
  );
}
