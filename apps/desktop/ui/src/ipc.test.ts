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
// document ones plus `cut` and `savePreset` — pass a caller's object straight through, so for those
// only the command name is checked. Typing them is #70's.
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

// Four exports are not commands: two open the dialog plugin's own picker, and two only read a
// rejected value. Each must invoke nothing at all, which is as much a fact about the seam as the
// others.
const NOT_COMMANDS = ["pickSavePath", "pickOpenPath", "ipcErrorCode", "ipcErrorMessage"];

// A wrapper is named after the command it calls, so the pairing is derived rather than kept by hand
// in what would be a fourth copy of the surface. Two wrappers cannot follow the rule: `delete` is a
// reserved word in TypeScript, and `pickImagePath` is named for what it returns.
const RENAMED: Record<string, string> = { deleteNodes: "delete", pickImagePath: "pick_image" };

const commandsExpectedOf = (wrapper: string) => {
  if (NOT_COMMANDS.includes(wrapper)) return [];
  return [RENAMED[wrapper] ?? wrapper.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`)];
};

const checkRecordedCalls = (wrapper: string) => {
  // Exactly its own command, exactly once. Membership alone would let two wrappers swap the
  // literals they invoke — `newDoc` calling `snapshot` and back — while every name stayed
  // registered and every command stayed observed.
  expect(calls.map((c) => c.cmd), `what ${wrapper} invoked`).toEqual(commandsExpectedOf(wrapper));
  for (const { cmd, args } of calls) {
    const undeclaredKeys = Object.keys(args ?? {}).filter((k) => !declared[cmd].includes(k));
    expect(undeclaredKeys, `${wrapper} sends a key ${cmd} does not declare`).toEqual([]);
    observed.add(cmd);
  }
};

describe("every ipc.ts wrapper calls its own registered command", () => {
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
    checkRecordedCalls("traceImage");
  });

  test("loadImagePreview", async () => {
    calls.length = 0;
    await ipc.loadImagePreview({ path: "/tmp/trace.png" });
    checkRecordedCalls("loadImagePreview");
  });
});

// Declared last on purpose: vitest runs a file's tests in order, so this reads what the tests above
// recorded.
//
// The tests above each hold one wrapper to one command, which leaves the other direction: a command
// registered in Rust that no wrapper names at all. Nothing else here would notice it, and it is
// worth failing over — `CLAUDE.md` has the UI reaching Rust only through `ui/src/ipc.ts`, so a
// registered command with no wrapper there is one nothing can invoke.
test("every registered command is witnessed by a wrapper", () => {
  expect([...observed].sort()).toEqual(Object.keys(declared).sort());
});
