// SPDX-License-Identifier: GPL-3.0-or-later
export type Bounds = { x: number; y: number; w: number; h: number };
export type SceneNode = { id: number; bounds: Bounds };
export type Scene = { nodes: SceneNode[] };

export function hitTest(scene: Scene, x: number, y: number): number | null {
  for (let i = scene.nodes.length - 1; i >= 0; i--) {
    // topmost last
    const b = scene.nodes[i].bounds;
    if (x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h) return scene.nodes[i].id;
  }
  return null;
}
