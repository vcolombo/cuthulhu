// SPDX-License-Identifier: GPL-3.0-or-later
import type { Bounds } from "./hittest";
import { MM_PER_IN, formatLength, unitToMm, type LengthUnit } from "../length";

export type MatKind = "12x12" | "12x24" | "roll";

/** Lovable's roll starts at 36 in. Six inches is the shortest length the field will commit. */
export const ROLL_DEFAULT_MM = 36 * MM_PER_IN;
export const ROLL_MIN_MM = 6 * MM_PER_IN;

const IN_12 = 12 * MM_PER_IN;
const IN_24 = 24 * MM_PER_IN;

/**
 * Cameo offers a 12×12, a 12×24 and a roll. Puma is roll only. An unknown machine, including
 * a document that has not been designed for one yet, uses the Cameo list: that is the artboard
 * the empty document already has.
 */
export function matKindsFor(machineId: string | null): MatKind[] {
  if (machineId === "puma") return ["roll"];
  return ["12x12", "12x24", "roll"];
}

export function defaultMatKind(machineId: string | null): MatKind {
  return machineId === "puma" ? "roll" : "12x12";
}

/** The kind to draw. The caller's choice is kept when this returns something else, so switching
 *  back to a Cameo restores a 12×24 that a Puma could not show. */
export function offeredKind(chosen: MatKind, machineId: string | null): MatKind {
  const kinds = matKindsFor(machineId);
  return kinds.includes(chosen) ? chosen : defaultMatKind(machineId);
}

export function clampRollLength(mm: number): number {
  if (!Number.isFinite(mm)) return ROLL_DEFAULT_MM;
  return Math.max(ROLL_MIN_MM, mm);
}

/**
 * The length a roll field should store. Blank is not zero: `Number("")` is 0, which the
 * clamp would turn into the six-inch minimum and shorten the mat. Non-numeric is a revert.
 */
export function committedRollLength(text: string, unit: LengthUnit): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const n = Number(trimmed);
  if (!Number.isFinite(n)) return null;
  return clampRollLength(unitToMm(n, unit));
}

/**
 * The mat in document millimetres, from the origin. Sheet mats are 12 in wide unless the machine
 * is narrower. A roll is as wide as the machine, which is the clamp: the mat is not the bed.
 */
export function matBounds(kind: MatKind, machineWidthMm: number, rollLengthMm: number): Bounds {
  const width = Math.max(machineWidthMm, 1);
  if (kind === "roll") return { x: 0, y: 0, w: width, h: clampRollLength(rollLengthMm) };
  return { x: 0, y: 0, w: Math.min(IN_12, width), h: kind === "12x12" ? IN_12 : IN_24 };
}

export function matKindLabel(kind: MatKind, unit: LengthUnit): string {
  if (kind === "roll") return "Roll";
  const name = kind === "12x12" ? "12 × 12 in" : "12 × 24 in";
  if (unit === "in") return name;
  const height = kind === "12x12" ? IN_12 : IN_24;
  // Whole millimetres: 12 in is 304.8, and the preset name is the inch size.
  return `${name} (${formatLength(IN_12, "mm", 0)} × ${formatLength(height, "mm", 0)} mm)`;
}
