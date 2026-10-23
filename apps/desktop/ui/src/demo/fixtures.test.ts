// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { DEMO_FIXTURES, DEMO_STORAGE_KEY, IMPORT_FAILED, demoFixture, type DemoId } from "./fixtures";

describe("demo fixtures", () => {
  it("names the five states the dev menu and the e2e fake share", () => {
    expect(DEMO_FIXTURES.map((fixture) => fixture.id)).toEqual([
      "normal",
      "noCutters",
      "emptyCanvas",
      "importFailed",
      "hostOffline",
    ]);
    expect(new Set(DEMO_FIXTURES.map((fixture) => fixture.label)).size).toBe(DEMO_FIXTURES.length);
    expect(DEMO_STORAGE_KEY).toBe("cuthulhu.demo");
  });

  it("points each state at the screen the menu is for", () => {
    const noCutters = demoFixture("noCutters");
    expect(noCutters.clearDevices).toBe(true);
    expect(noCutters.seedShapes).toBe(true);

    const empty = demoFixture("emptyCanvas");
    expect(empty.clearShapes).toBe(true);
    expect(empty.seedShapes).toBe(false);

    expect(demoFixture("importFailed").importError).toBe(IMPORT_FAILED);
    expect(demoFixture("hostOffline")).toMatchObject({ hostOffline: true, importError: null, seedShapes: true });
    expect(demoFixture("normal").clearDevices).toBe(false);
    expect(() => demoFixture("missing" as DemoId)).toThrow(/unknown demo state/);
  });
});
