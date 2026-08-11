// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { runPairing } from "./pairing";
import type { PairedHostView } from "../ipc";

const aHostView = (): PairedHostView => ({
  id: "host-1",
  name: "Workshop Pi",
  address: "pi.local:7878",
  unreachable: null,
});

// Each test records the order the effects ran in, because the order *is* the requirement:
// the fingerprint is shown before anything carrying the token is sent, and nothing is saved
// until a Test reached the host. "The right calls happened" would pass while the token leaked
// to whatever answered at the address the operator typed.
describe("runPairing", () => {
  it("shows the fingerprint for confirmation before any token is sent", async () => {
    const calls: string[] = [];
    const s = await runPairing({ address: "pi.local:7878", token: "t" }, {
      probe: async () => { calls.push("probe"); return "AB:CD"; },
      test: async () => { calls.push("test"); return []; },
      save: async () => { calls.push("save"); return aHostView(); },
    }, { confirmFingerprint: true });
    expect(calls).toEqual(["probe", "test", "save"]);
    expect(s.kind).toBe("paired");
  });

  it("saves nothing when the operator rejects the fingerprint", async () => {
    const calls: string[] = [];
    await runPairing({ address: "pi.local:7878", token: "t" }, {
      probe: async () => "AB:CD",
      test: async () => { calls.push("test"); return []; },
      save: async () => { calls.push("save"); return aHostView(); },
    }, { confirmFingerprint: false });
    expect(calls).toEqual([]);
  });

  it("saves nothing when the token is refused", async () => {
    const calls: string[] = [];
    const s = await runPairing({ address: "pi.local:7878", token: "wrong" }, {
      probe: async () => "AB:CD",
      test: async () => { throw { code: "host_unreachable", message: "the token was refused" }; },
      save: async () => { calls.push("save"); return aHostView(); },
    }, { confirmFingerprint: true });
    expect(calls).toEqual([]);
    expect(s.kind).toBe("failed");
    expect(s.message).toContain("refused");
  });

  it("carries the fingerprint through confirm and testing, and reports each step", async () => {
    const seen: string[] = [];
    await runPairing({ address: "pi.local:7878", token: "t", name: "Workshop Pi" }, {
      probe: async () => "AB:CD",
      test: async () => [],
      save: async () => aHostView(),
    }, {
      confirmFingerprint: async fp => { seen.push(`asked:${fp}`); return true; },
      onState: s => seen.push(s.fingerprint ? `${s.kind}:${s.fingerprint}` : s.kind),
    });
    expect(seen).toEqual([
      "probing", "confirm:AB:CD", "asked:AB:CD", "testing:AB:CD", "paired:AB:CD",
    ]);
  });
});
