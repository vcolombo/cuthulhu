// SPDX-License-Identifier: GPL-3.0-or-later

/** Session key the dev menu writes before reload. The fake reads the same string; it cannot import this constant. */
export const DEMO_STORAGE_KEY = "cuthulhu.demo";

export const IMPORT_FAILED = "Couldn't import “logo-final.svg”. The file is not valid SVG.";

export type DemoId = "normal" | "noCutters" | "emptyCanvas" | "importFailed" | "hostOffline";

/** The id is the whole fixture. The fake switches on it; a second bag of flags could name one state and apply another. */
export type DemoFixture = {
  id: DemoId;
  label: string;
};

export const DEMO_FIXTURES: readonly DemoFixture[] = [
  { id: "normal", label: "Normal" },
  { id: "noCutters", label: "No cutters" },
  { id: "emptyCanvas", label: "Empty canvas" },
  { id: "importFailed", label: "Import failed" },
  { id: "hostOffline", label: "Host offline" },
];

export function demoFixture(id: DemoId): DemoFixture {
  const fixture = DEMO_FIXTURES.find((item) => item.id === id);
  if (!fixture) throw new Error(`unknown demo state: ${id}`);
  return fixture;
}
