// SPDX-License-Identifier: GPL-3.0-or-later
import type { Scene } from "./hittest";

export type NodeId = number;

export interface Renderer {
  setScene(s: Scene): void;
  markDirty(id: NodeId): void;
  setSelection(ids: NodeId[]): void;
  draw(): void;
}
