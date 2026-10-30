// SPDX-License-Identifier: GPL-3.0-or-later

/** Arrow keys, in world mm. Alt is the fine step; Shift is the coarse one. They are document
 *  distances, not screen pixels, so a nudge at 400% moves the same vinyl as a nudge at fit. */
export const NUDGE_MM = 1;
export const NUDGE_FINE_MM = 0.1;
export const NUDGE_COARSE_MM = 10;
/** Bracket keys, in degrees. Shift takes the single degree; the plain press matches the rotate
 *  handle's 15° step. */
export const NUDGE_ROTATE_DEG = 15;
export const NUDGE_ROTATE_FINE_DEG = 1;

export type Nudge = { t: "move"; dx: number; dy: number } | { t: "rotate"; deg: number };

type KeyInfo = { key: string; code: string; shiftKey: boolean; altKey: boolean; metaKey: boolean; ctrlKey: boolean };

/** What an arrow or a bracket does to the selection. Null for every other key, and for a chord
 *  with ⌘/Ctrl, which already belongs to undo, select-all, and the booleans. */
export function nudgeFromKey(e: KeyInfo): Nudge | null {
  if (e.metaKey || e.ctrlKey) return null;
  if (e.code === "BracketLeft" || e.code === "BracketRight") {
    const step = e.shiftKey ? NUDGE_ROTATE_FINE_DEG : NUDGE_ROTATE_DEG;
    const sign = e.code === "BracketLeft" ? -1 : 1;
    return { t: "rotate", deg: sign * step };
  }
  const step = e.altKey ? NUDGE_FINE_MM : e.shiftKey ? NUDGE_COARSE_MM : NUDGE_MM;
  if (e.key === "ArrowLeft") return { t: "move", dx: -step, dy: 0 };
  if (e.key === "ArrowRight") return { t: "move", dx: step, dy: 0 };
  if (e.key === "ArrowUp") return { t: "move", dx: 0, dy: -step };
  if (e.key === "ArrowDown") return { t: "move", dx: 0, dy: step };
  return null;
}

/** A field, a dialog, or a menu. Arrow keys and brackets typed there belong to that control,
 *  including a focused button that is not one of these — a toolbar button must not swallow a nudge. */
export function isEditingTarget(t: EventTarget | null): boolean {
  if (t === null || typeof t !== "object" || !("closest" in t)) return false;
  const el = t as HTMLElement;
  if (typeof el.closest !== "function") return false;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable) return true;
  return el.closest("[role='dialog'], [role='menu']") !== null;
}
