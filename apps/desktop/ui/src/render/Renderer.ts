// SPDX-License-Identifier: GPL-3.0-or-later
import type { Affine6, Bounds, Scene } from "./hittest";
import type { Pt } from "./affine";
import type { Box } from "../interaction/selectionBox";
import type { Guide } from "../interaction/snap";

export type NodeId = number;

/** What is drawn over the artwork: the selection's box and handles, a marquee band or a lasso
 *  loop, and the smart guides of a snapped gesture. `crossing` is the band's right-to-left
 *  mode: the stroke is dashed. A containing band is solid. */
export type Overlay = { box: Box | null; marquee: Bounds | null; crossing: boolean; lasso: Pt[] | null; guides: Guide[] };

export interface Renderer {
  setScene(s: Scene): void;
  markDirty(id: NodeId): void;
  setSelection(ids: NodeId[]): void;
  /** CSS size of the canvas and the display's pixel ratio; sizes the backing store. */
  resize(cssW: number, cssH: number, dpr: number): void;
  /** World mm → CSS px. */
  setView(m: Affine6): void;
  setOverlay(o: Overlay): void;
  draw(): void;
}
