// SPDX-License-Identifier: GPL-3.0-or-later

/** Documents and IPC stay in millimetres. This is only how a length is shown. */
export type LengthUnit = "in" | "mm";

export const MM_PER_IN = 25.4;

/**
 * CLDR's everyday measurement system "US" (LDML Part 2): the United States,
 * Liberia, and Myanmar. Those regions are shown in inches. The UK mix, a
 * locale with no region, and everyone else are millimetres. Documents stay
 * in millimetres either way.
 *
 * [doc: Unicode Locale Data Markup Language Part 2: General, https://www.unicode.org/reports/tr35/tr35-general.html, Measurement System Data]
 */
const INCH_REGIONS = new Set(["US", "LR", "MM"]);

/** The region subtag of a BCP 47 tag, or null when the tag does not name one. */
function regionOf(locale: string): string | null {
  const tag = locale.trim().replace(/_/g, "-").split("@")[0] ?? "";
  const sub = tag.split("-").filter((part) => part.length > 0);
  if (sub.length === 0) return null;
  let index = 1;
  if (sub[index] && /^[A-Za-z]{4}$/.test(sub[index])) index += 1;
  const region = sub[index];
  if (!region) return null;
  if (/^[A-Za-z]{2}$/.test(region) || /^\d{3}$/.test(region)) return region.toUpperCase();
  return null;
}

/** Inches for a US-customary region, millimetres otherwise. */
export function unitForLocale(locale: string): LengthUnit {
  const region = regionOf(locale);
  return region !== null && INCH_REGIONS.has(region) ? "in" : "mm";
}

/**
 * A saved `in` or `mm` is the operator's choice and is not revised when the
 * region changes. Anything else is resolved once from `locale` and should be
 * written back like a choice they made.
 */
export function unitFromPreference(
  saved: LengthUnit | null | undefined,
  locale: string,
): { unit: LengthUnit; save: boolean } {
  if (saved === "in" || saved === "mm") return { unit: saved, save: false };
  return { unit: unitForLocale(locale), save: true };
}

export function unitToMm(value: number, unit: LengthUnit): number {
  return unit === "in" ? value * MM_PER_IN : value;
}

/** `digits` defaults to one millimetre tenth, or two inch hundredths, so 1.000 in reads back as 25.4 mm. */
export function formatLength(mm: number, unit: LengthUnit, digits?: number): string {
  const places = digits ?? (unit === "in" ? 2 : 1);
  const shown = unit === "in" ? mm / MM_PER_IN : mm;
  return shown.toFixed(places);
}
