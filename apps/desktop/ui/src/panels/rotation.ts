// SPDX-License-Identifier: GPL-3.0-or-later
import { apply, rotateAbout } from "../render/affine";
import type { Affine6, Bounds } from "../render/hittest";

/** Degrees of the world matrix's x axis, the angle the canvas rotate handle sweeps. */
export function worldRotationDegrees(world: Affine6): number {
  const degrees = (Math.atan2(world[1], world[0]) * 180) / Math.PI;
  const rounded = Math.round(degrees * 10) / 10;
  return rounded === 0 ? 0 : rounded;
}

/**
 * A world-space rotation about the shape's own centre, by the delta from the angle it has to
 * `degrees`. The same matrix the canvas handle commits (`rotateAbout` on the box centre):
 * `commit_transform` applies it in world space.
 */
export function rotationMove(world: Affine6, local: Bounds, degrees: number): Affine6 {
  const current = Math.atan2(world[1], world[0]);
  const delta = (degrees * Math.PI) / 180 - current;
  const center = apply(world, { x: local.x + local.w / 2, y: local.y + local.h / 2 });
  return rotateAbout(delta, center);
}
