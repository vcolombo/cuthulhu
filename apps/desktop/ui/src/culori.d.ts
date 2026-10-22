// SPDX-License-Identifier: GPL-3.0-or-later
// culori 4.0.2 ships JavaScript only. These are the calls vinyl.ts makes.
declare module "culori" {
  export interface Oklch {
    mode: "oklch";
    l: number;
    c: number;
    h?: number;
    alpha?: number;
  }
  export interface Rgb {
    mode: "rgb";
    r: number;
    g: number;
    b: number;
    alpha?: number;
  }
  export type Color = Oklch | Rgb | { mode: string; alpha?: number };

  export function converter(mode: "oklch"): (color: Color | string) => Oklch | undefined;
  export function converter(mode: "rgb"): (color: Color | string) => Rgb | undefined;
  export function formatHex(color: Color | string | undefined): string | undefined;
  export function clampChroma(color: Oklch, mode?: string, rgbGamut?: string): Oklch | undefined;
  export function wcagContrast(a: Color | string, b: Color | string): number;
  export function interpolate(colors: Array<Color | string>, mode?: string): (t: number) => Color;
  export function differenceCiede2000(kL?: number, kC?: number, kH?: number): (a: Color | string, b: Color | string) => number;
}
