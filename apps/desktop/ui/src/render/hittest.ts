// SPDX-License-Identifier: GPL-3.0-or-later
export type Bounds = { x: number; y: number; w: number; h: number };
export type Affine6 = [number, number, number, number, number, number];
export type ShapeGeom =
  | { t: "rect"; w: number; h: number }
  | { t: "ellipse"; rx: number; ry: number }
  | { t: "path"; d: string };
export type SceneNode = { id: number; bounds: Bounds; shape?: ShapeGeom; world?: Affine6 };
export type Scene = { nodes: SceneNode[] };

export function hitTest(scene: Scene, x: number, y: number): number | null {
  for (let i = scene.nodes.length - 1; i >= 0; i--) {
    // topmost last
    const b = scene.nodes[i].bounds;
    if (x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h) return scene.nodes[i].id;
  }
  return null;
}
