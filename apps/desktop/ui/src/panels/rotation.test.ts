// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, test } from "vitest";
import { rotateAbout } from "../render/affine";
import { rotationMove, worldRotationDegrees } from "./rotation";

const local = { x: 0, y: 0, w: 10, h: 10 };

describe("rotation field", () => {
  test("an identity shape reads 0° and a 90° edit is the canvas handle's matrix", () => {
    const world: [number, number, number, number, number, number] = [1, 0, 0, 1, 4, 6];
    expect(worldRotationDegrees(world)).toBe(0);
    const turned = rotationMove(world, local, 90);
    const center = { x: 4 + 5, y: 6 + 5 };
    expect(turned.map((v) => Math.round(v * 1e6) / 1e6)).toEqual(
      rotateAbout(Math.PI / 2, center).map((v) => Math.round(v * 1e6) / 1e6),
    );
    expect(worldRotationDegrees(turned)).toBe(90);
  });

  test("committing the angle already shown is a no-op", () => {
    const world = rotateAbout(Math.PI / 6, { x: 5, y: 5 });
    const shown = worldRotationDegrees(world);
    const again = rotationMove(world, local, shown);
    again.forEach((value, i) => expect(value).toBeCloseTo([1, 0, 0, 1, 0, 0][i], 6));
  });
});
