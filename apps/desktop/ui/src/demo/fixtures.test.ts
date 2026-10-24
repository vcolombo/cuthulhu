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

  it("returns the menu label for a known id and rejects any other", () => {
    expect(DEMO_FIXTURES.map((fixture) => demoFixture(fixture.id).label)).toEqual([
      "Normal",
      "No cutters",
      "Empty canvas",
      "Import failed",
      "Host offline",
    ]);
    expect(IMPORT_FAILED).toBe("Couldn't import “logo-final.svg”. The file is not valid SVG.");
    expect(() => demoFixture("missing" as DemoId)).toThrow(/unknown demo state/);
  });
});
