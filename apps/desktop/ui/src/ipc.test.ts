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
// payload itself, so its keys are checked here; the fourteen wrappers typed `Args` — twelve
// document ones plus `cut` and `savePreset` — pass a caller's object straight through, so only
// their command name is. Typing those is #70.
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

const observed = new Set<string>();

const checkRecordedCalls = (wrapper: string) => {
  for (const { cmd, args } of calls) {
    expect(Object.keys(declared), `${wrapper} invokes an unregistered command`).toContain(cmd);
    const undeclaredKeys = Object.keys(args ?? {}).filter((k) => !declared[cmd].includes(k));
    expect(undeclaredKeys, `${wrapper} sends a key ${cmd} does not declare`).toEqual([]);
    observed.add(cmd);
  }
};

describe("every ipc.ts wrapper names a registered command", () => {
  for (const [wrapper, exported] of Object.entries(ipc)) {
    if (typeof exported !== "function") continue;

    test(wrapper, async () => {
      calls.length = 0;
      // No arguments: a wrapper's payload keys come from its own object literal, so they are
      // present and undefined rather than absent. What the backend would do with the values is not
      // the question — the names are.
      await (exported as () => Promise<unknown>)();
      checkRecordedCalls(wrapper);
    });
  }
});

// Two wrappers forward the caller's object rather than building one, so calling them with nothing
// records nothing and says nothing. Their parameter types name the keys, which makes a literal
// here checked twice: by `tsc` against the wrapper's own type, and below against the inventory. A
// wrapper typed `{ path, opts }` — #85's own incident — fails the first before reaching the second.
describe("a wrapper that forwards its caller's payload", () => {
  test("traceImage", async () => {
    calls.length = 0;
    await ipc.traceImage({
      path: "/tmp/trace.png",
      controls: { mode: "binary", speckle: 4, smoothing: 60, detail: 9, colors: 2 },
    });
    expect(calls.map((c) => c.cmd)).toEqual(["trace_image"]);
    checkRecordedCalls("traceImage");
  });

  test("loadImagePreview", async () => {
    calls.length = 0;
    await ipc.loadImagePreview({ path: "/tmp/trace.png" });
    expect(calls.map((c) => c.cmd)).toEqual(["load_image_preview"]);
    checkRecordedCalls("loadImagePreview");
  });
});

// Declared last on purpose: vitest runs a file's tests in order, so this reads what the tests above
// recorded.
//
// Without it the checks above are conditional on a call being made at all — gut `forceQuit` to a
// bare `return` and every one of them still passes, because a wrapper that invokes nothing has
// nothing to disagree with. That is the same silence #85 is about, one level up: the command stops
// being witnessed and no test says so.
//
// Equality, not containment. A registered command with no wrapper is one the desktop cannot call
// from the only place it reaches Rust (`CLAUDE.md`: the UI reaches Rust only through `ui/src/ipc.ts`),
// so it is worth failing over rather than leaving for someone to notice.
test("every registered command is witnessed by a wrapper", () => {
  expect([...observed].sort()).toEqual(Object.keys(declared).sort());
});
