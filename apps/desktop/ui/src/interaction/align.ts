// SPDX-License-Identifier: GPL-3.0-or-later
import { unionBounds, type Affine6, type Bounds } from "../render/hittest";
import { translate } from "../render/affine";

/** What one align or distribute click moves as a piece: a selected id, with the world bounds of
 *  every shape it moves, so a Group lines up by all of its shapes, as a drag would move it. */
export type Unit = { ids: number[]; bounds: Bounds };
/** One unit's translation, in the shape `commit_transforms` takes. */
export type Move = { ids: number[]; m: Affine6 };
export type AlignMode = "left" | "hcenter" | "right" | "top" | "vmiddle" | "bottom";
export type Axis = "x" | "y";

/** Exported so the panel's icons and the queue key read the same tables the moves do. */
export const AXIS: Record<AlignMode, Axis> = {
  left: "x", hcenter: "x", right: "x", top: "y", vmiddle: "y", bottom: "y",
};
/** Where on the unit's span the mode lines up: 0 at the start edge, 1 at the far edge. */
export const AT: Record<AlignMode, number> = { left: 0, hcenter: 0.5, right: 1, top: 0, vmiddle: 0.5, bottom: 1 };

const start = (b: Bounds, a: Axis) => (a === "x" ? b.x : b.y);
const size = (b: Bounds, a: Axis) => (a === "x" ? b.w : b.h);
const along = (a: Axis, d: number): Affine6 => (a === "x" ? translate(d, 0) : translate(0, d));

/** Units that would not move are left out, so a click that changes nothing commits nothing. So is
 *  a non-finite move: it crosses IPC as null and comes back as an unreadable error. */
function moved(entries: [Unit, number][], axis: Axis): Move[] {
  return entries.filter(([, d]) => Number.isFinite(d) && d !== 0).map(([u, d]) => ({ ids: u.ids, m: along(axis, d) }));
}

/** Two or more units line up against their own bounds; a lone unit lines up against the artboard,
 *  since aligning one thing to itself would do nothing.
 *  // ponytail: no key unit (Illustrator's "key object"), so five pieces cannot be aligned to a
 *  sixth that stays put. Upgrade: a key unit picked by clicking a selected unit again.
 *  // ponytail: a unit lines up by its axis-aligned bounds, as snapping does, so a rotated piece's
 *  outline sits off the line by however far its box overhangs it. Upgrade: line up each unit's
 *  outline extreme along the axis. */
export function alignMoves(units: Unit[], mode: AlignMode, artboard: Bounds | null): Move[] {
  const target = units.length >= 2 ? unionBounds(units.map((u) => u.bounds)) : artboard;
  if (units.length === 0 || !target) return [];
  const axis = AXIS[mode];
  const at = AT[mode];
  const line = start(target, axis) + at * size(target, axis);
  return moved(units.map((u) => [u, line - (start(u.bounds, axis) + at * size(u.bounds, axis))]), axis);
}

const end = (u: Unit, a: Axis) => start(u.bounds, a) + size(u.bounds, a);
/** mm. Edges an operator sees as shared can differ by float noise once transformed; compared
 *  exactly, a piece flush with its border started a hair before it and the border stopped being
 *  the frame (silent-failure-hunter). */
const EPS = 1e-6;

/** Why distribute cannot act on an axis: fewer than three units, several units spanning the rest
 *  (stacked copies), a frame too small for what is inside it, pieces that would pass each other,
 *  or a second frame inside the first. */
export type DistributeBlock = "few" | "stacked" | "tight" | "crowded" | "nested";

/** How a distribute is anchored, or null when it has nothing to do.
 *  - "ends": the unit that starts first and the unit that reaches furthest stay put, and the rest
 *    go between them.
 *  - "frame": one unit spans all the others, as a weed border or backing plate does when the
 *    whole design is selected. It stays put and the rest are spaced inside it, with the margins
 *    at its edges equal to the gaps between them. Sure Cuts A Lot offers the same as a separate
 *    "Distribute to Selection Below" mode; here the selection says which is meant, so there is no
 *    mode to set.
 *  Blocked when more than one unit spans the rest, since no one of them is the frame, and when the
 *  pieces inside a frame are longer than it: spacing them evenly would push the outer ones through
 *  the border that was meant to hold them (silent-failure-hunter). Also blocked when a piece inside
 *  the frame spans the others in turn, as a weed border on a backing plate does with both selected:
 *  spaced as one more piece, the border left its letters behind. Spacing them inside the innermost
 *  frame instead is a choice of layout this rule does not make. */
type Anchors =
  | { kind: "ends"; order: Unit[]; li: number }
  | { kind: "frame"; frame: Unit; inner: Unit[] }
  | { kind: "blocked"; reason: DistributeBlock };

function anchors(units: Unit[], axis: Axis): Anchors {
  if (units.length < 3) return { kind: "blocked", reason: "few" };
  // Array.prototype.sort is stable, so document order breaks ties.
  const order = [...units].sort((a, b) => start(a.bounds, axis) - start(b.bounds, axis));
  // The true far edge, taken once: comparing each unit with the last pick let the tolerance chain,
  // walking the far end back until a unit that spans nothing looked like a frame (Copilot on #301).
  // A loop, not Math.max(...): one argument per unit overflows the engine's limit on a selection
  // of a few hundred thousand, and the panel runs this as it renders (Copilot on #301).
  let hi = -Infinity;
  for (const u of order) hi = Math.max(hi, end(u, axis));
  // Of the units reaching it exactly, the one that starts last. Exactly, not within EPS: a piece
  // ending just short is not the far end, and taking it moved the one that is (CodeRabbit on #301).
  // The tolerance belongs to the frame test below, which also catches the first unit reaching the
  // far edge, so the far end in "ends" mode is never the first unit.
  let li = 0;
  order.forEach((u, i) => {
    if (end(u, axis) === hi) li = i;
  });
  // Spanning is asked of every unit, not read off the ends: a plate sharing its start with a piece
  // earlier in the document sorts second, and was taken as the far end (code-reviewer).
  const spanning = spanningAll(order, axis);
  if (spanning.length > 1) return { kind: "blocked", reason: "stacked" };
  if (spanning.length === 1) {
    const frame = spanning[0];
    const inner = order.filter((u) => u !== frame);
    if (spanningAll(inner, axis).length > 0) return { kind: "blocked", reason: "nested" };
    const left = room(frame, inner, axis);
    if (left < -EPS) return { kind: "blocked", reason: "tight" };
    // The margin is the gap here, so the rule for two ends below holds inside a frame too: with the
    // pieces filling it, a line of no width lands level with a neighbour and swaps with it on the
    // next click.
    const gap = left / (inner.length + 1);
    if (inner.some((u) => size(u.bounds, axis) + gap <= EPS)) return { kind: "blocked", reason: "crowded" };
    return { kind: "frame", frame, inner };
  }
  // Between two ends, gaps may be negative and the pieces overlap evenly, but only while every
  // piece, the far end included, is longer than the overlap. Then the landed starts and ends both
  // strictly increase, so the next click finds the same first unit, far end and order, and moves
  // nothing. A piece no longer than the overlap is passed by the next or left level with it, and the
  // tie or reversal hands the next click a different anchor, so the layout drifts with every click
  // (Copilot on #301).
  const gap = endsGap(order, li, axis);
  if (order.some((u) => size(u.bounds, axis) + gap <= EPS)) return { kind: "blocked", reason: "crowded" };
  return { kind: "ends", order, li };
}

/** The units reaching both ends of the span `units` cover, each end within EPS. */
function spanningAll(units: Unit[], axis: Axis): Unit[] {
  let lo = Infinity, hi = -Infinity;
  for (const u of units) {
    lo = Math.min(lo, start(u.bounds, axis));
    hi = Math.max(hi, end(u, axis));
  }
  return units.filter((u) => start(u.bounds, axis) <= lo + EPS && end(u, axis) >= hi - EPS);
}

/** What the frame has left once its pieces are laid end to end. */
function room(frame: Unit, inner: Unit[], axis: Axis): number {
  return size(frame.bounds, axis) - inner.reduce((s, u) => s + size(u.bounds, axis), 0);
}

/** Why distribute cannot act, or null when it can, so the button says so rather than doing
 *  nothing. */
export function distributeBlock(units: Unit[], axis: Axis): DistributeBlock | null {
  const a = anchors(units, axis);
  return a.kind === "blocked" ? a.reason : null;
}

/** Equal gaps between neighbours, because gaps are what a weeder works between; see `Anchors` for
 *  what stays put. Taking the far end from whichever unit starts last threw small pieces past each
 *  other when a wide one started first (code-reviewer). Between two ends, units longer than the
 *  span get negative gaps and overlap evenly, which is still the arithmetic answer; a frame they
 *  do not fit is blocked instead (see `Anchors`). */
export function distributeMoves(units: Unit[], axis: Axis): Move[] {
  const a = anchors(units, axis);
  if (a.kind === "blocked") return [];
  // The units that stay put are never listed, so float drift cannot turn them into a move.
  if (a.kind === "frame") {
    return place(a.inner, start(a.frame.bounds, axis), room(a.frame, a.inner, axis) / (a.inner.length + 1), axis);
  }
  const { order, li } = a;
  const middle = order.filter((_, i) => i !== 0 && i !== li);
  return place(middle, end(order[0], axis), endsGap(order, li, axis), axis);
}

/** The equal gap between two ends: the span from the first unit's start to the far end, less
 *  every unit's length, over the spaces between them. */
function endsGap(order: Unit[], li: number, axis: Axis): number {
  const span = end(order[li], axis) - start(order[0].bounds, axis);
  const total = order.reduce((s, u) => s + size(u.bounds, axis), 0);
  return (span - total) / (order.length - 1);
}

/** Lays `units` out in order from `from`, with `gap` before each one. */
function place(units: Unit[], from: number, gap: number, axis: Axis): Move[] {
  const entries: [Unit, number][] = [];
  let cursor = from + gap;
  for (const u of units) {
    entries.push([u, cursor - start(u.bounds, axis)]);
    cursor += size(u.bounds, axis) + gap;
  }
  return moved(entries, axis);
}
