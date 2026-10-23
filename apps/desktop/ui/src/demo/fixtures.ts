// SPDX-License-Identifier: GPL-3.0-or-later

/** Session key the dev menu writes before reload. The fake reads the same string; it cannot import this constant. */
export const DEMO_STORAGE_KEY = "cuthulhu.demo";

export const IMPORT_FAILED = "Couldn't import “logo-final.svg”. The file is not valid SVG.";

export type DemoId = "normal" | "noCutters" | "emptyCanvas" | "importFailed" | "hostOffline";

/** Flags the fake applies. The menu and the e2e tests pass this object; neither one re-implements the state. */
export type DemoFixture = {
  id: DemoId;
  label: string;
  clearDevices: boolean;
  clearShapes: boolean;
  seedShapes: boolean;
  importError: string | null;
  hostOffline: boolean;
};

export const DEMO_FIXTURES: readonly DemoFixture[] = [
  { id: "normal", label: "Normal", clearDevices: false, clearShapes: false, seedShapes: true, importError: null, hostOffline: false },
  { id: "noCutters", label: "No cutters", clearDevices: true, clearShapes: false, seedShapes: true, importError: null, hostOffline: false },
  { id: "emptyCanvas", label: "Empty canvas", clearDevices: false, clearShapes: true, seedShapes: false, importError: null, hostOffline: false },
  { id: "importFailed", label: "Import failed", clearDevices: false, clearShapes: false, seedShapes: true, importError: IMPORT_FAILED, hostOffline: false },
  { id: "hostOffline", label: "Host offline", clearDevices: false, clearShapes: false, seedShapes: true, importError: null, hostOffline: true },
];

export function demoFixture(id: DemoId): DemoFixture {
  const fixture = DEMO_FIXTURES.find((item) => item.id === id);
  if (!fixture) throw new Error(`unknown demo state: ${id}`);
  return fixture;
}
