// SPDX-License-Identifier: GPL-3.0-or-later

/** Documents and IPC stay in millimetres. This is only how a length is shown. */
export type LengthUnit = "in" | "mm";

export const MM_PER_IN = 25.4;

export function unitToMm(value: number, unit: LengthUnit): number {
  return unit === "in" ? value * MM_PER_IN : value;
}

/** `digits` defaults to one millimetre tenth, or two inch hundredths, so 1.000 in reads back as 25.4 mm. */
export function formatLength(mm: number, unit: LengthUnit, digits?: number): string {
  const places = digits ?? (unit === "in" ? 2 : 1);
  const shown = unit === "in" ? mm / MM_PER_IN : mm;
  return shown.toFixed(places);
}
