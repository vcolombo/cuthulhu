// SPDX-License-Identifier: GPL-3.0-or-later
//
// Every wrapper in ipc.ts against the commands the desktop registers.
//
// The e2e fake checks the same inventory, but only for calls a test actually makes: `forceQuit` is
// invoked from the close guard and nothing in the suite drives it, so a command renamed on one side
// and not the other would still have shipped green there. This calls every wrapper, so each one is
// witnessed once whether or not a scenario reaches it (#85).
//
// What it can state depends on how the wrapper is typed. A wrapper with named parameters builds the
// payload itself, so its keys are checked here; the fifteen document wrappers pass a caller's
// `Args` object straight through, so only their command name is. Typing those is #70.
import { describe, expect, test, vi } from "vitest";

import inventory from "../../ipc-inventory.json" with { type: "json" };
import * as ipc from "./ipc";

const calls: { cmd: string; args: Record<string, unknown> | undefined }[] = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args });
    return Promise.resolve(null);
  },
}));

// `pickSavePath`/`pickOpenPath` reach the dialog plugin rather than a command of ours; stubbed so
// calling them is harmless, and they record nothing to check.
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: () => Promise.resolve(null),
  open: () => Promise.resolve(null),
}));

const declared: Record<string, string[]> = inventory;

describe("every ipc.ts wrapper names a registered command", () => {
  for (const [wrapper, exported] of Object.entries(ipc)) {
    if (typeof exported !== "function") continue;

    test(wrapper, async () => {
      calls.length = 0;
      // No arguments: a wrapper's payload keys come from its own object literal, so they are
      // present and undefined rather than absent. What the backend would do with the values is not
      // the question — the names are.
      await (exported as () => Promise<unknown>)();

      for (const { cmd, args } of calls) {
        expect(Object.keys(declared), `${wrapper} invokes an unregistered command`).toContain(cmd);
        const undeclaredKeys = Object.keys(args ?? {}).filter((k) => !declared[cmd].includes(k));
        expect(undeclaredKeys, `${wrapper} sends a key ${cmd} does not declare`).toEqual([]);
      }
    });
  }
});
