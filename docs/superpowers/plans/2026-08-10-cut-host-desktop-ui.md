<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Cut Host desktop UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the desktop a way to pair with a Cut Host, see every cutter it owns, and watch a cut running on one — the UI half of phase 2.

**Architecture:** TypeScript only. Every Rust command this needs already exists and is green on `main`'s stack (#105, #106, #110, #111). Each dialog splits into a pure `viewmodel.ts` (unit-tested with vitest) and a thin `.tsx`, matching the existing `cut/` and `trace/` layout. The UI reaches Rust only through `ui/src/ipc.ts`.

**Tech Stack:** React, TypeScript, vitest, Playwright, Canvas2D (untouched here).

## Global Constraints

- **Do not touch `apps/desktop/src/` or `crates/`.** This plan is TypeScript. If a task appears to need a Rust change, stop and report it — the Rust half is complete and any gap is a plan defect.
- **`apps/desktop/ui/dist/` is committed and CI fails on a stale bundle.** Run `npm --prefix apps/desktop/ui run build` and commit `dist/` in the same commit as any `ui/src` change.
- **A caller is told about a cut through one value.** Render controls from `CutStatus.actions`. Never re-derive what is legal from `phase`. No test may reach for a phase to decide legality — this is the rule `DeviceState` is `pub(crate)` in Rust to enforce, and the UI is where it was historically broken.
- **`DeviceInfo` is mirrored by hand in four places (#70):** `ui/src/ipc.ts`, `ui/src/cut/viewmodel.ts`, `ui/src/cut/CutDialog.tsx`, `ui/e2e/smoke.spec.ts`. A change to its shape must land in all of them in one commit. `viewmodel.test.ts` carries fixtures that mirror it too.
- **`e2e/smoke.spec.ts` installs an in-page fake Tauri backend.** It must keep mirroring the real command surface, including the host commands.
- SPDX header on every new file: `<!-- ... -->` in Markdown, `// SPDX-License-Identifier: GPL-3.0-or-later` in TS/TSX.
- Comments explain why, not what. `// ponytail:` marks a deliberate simplification with its ceiling and upgrade path.
- Commit subjects are imperative with the reason attached. Keep the repo's `Co-Authored-By:` trailer. No process narration in prose.
- Verify each task with `npm --prefix apps/desktop/ui test` and, where the task touches the device list or dialogs, `npm --prefix apps/desktop/ui run e2e`.

## The Rust surface this consumes

All of these exist. Signatures are Rust; the TS wrappers are Task 1's job.

| Command | Rust signature | Notes |
| --- | --- | --- |
| `list_devices` | `() -> Vec<DeviceInfo>` | local hardware ++ every paired host's cutters |
| `list_hosts` | `() -> Vec<PairedHostView>` | `{ id, name, address, unreachable: Option<String> }` |
| `probe_host` | `(address: String) -> String` | TLS handshake only; returns the fingerprint. **Sends no token.** |
| `test_host` | `(address, token, fingerprint) -> Vec<DeviceInfo>` | proves the host; saves nothing |
| `pair_host` | `(name, address, token, fingerprint) -> PairedHostView` | proves, then persists |
| `forget_host` | `(id: HostId) -> ()` | refuses while a cut is active on that host |
| `get_device_state` | `() -> CutStatus` | routes to whichever device is aimed at |

`DeviceInfo.host` is `Option<HostId>` in Rust — `string | null` in TS. `null` means "attached to this computer".

---

## File Structure

**Create**
- `apps/desktop/ui/src/hosts/pairing.ts` — the pairing state machine, pure
- `apps/desktop/ui/src/hosts/pairing.test.ts`
- `apps/desktop/ui/src/hosts/PairHostDialog.tsx` — thin view over `pairing.ts`
- `apps/desktop/ui/src/hosts/deviceList.ts` — grouping and badge derivation, pure
- `apps/desktop/ui/src/hosts/deviceList.test.ts`

**Modify**
- `apps/desktop/ui/src/ipc.ts` — `DeviceInfo.host`, `PairedHostView`, five command wrappers
- `apps/desktop/ui/src/cut/viewmodel.ts`, `viewmodel.test.ts`, `CutDialog.tsx` — the mirror, plus host sections in the device picker
- `apps/desktop/ui/e2e/smoke.spec.ts` — fake backend gains `host` and the five commands
- `apps/desktop/ui/dist/` — rebuilt and committed with every task

---

### Task 1: `DeviceInfo.host` and the host command surface

**Files:**
- Modify: `apps/desktop/ui/src/ipc.ts`
- Modify: `apps/desktop/ui/src/cut/viewmodel.ts`, `apps/desktop/ui/src/cut/CutDialog.tsx`
- Modify: `apps/desktop/ui/e2e/smoke.spec.ts`
- Test: `apps/desktop/ui/src/cut/viewmodel.test.ts`

**Interfaces:**
- Produces: `DeviceInfo.host: string | null`, `PairedHostView`, and `listHosts`/`probeHost`/`testHost`/`pairHost`/`forgetHost` — every later task consumes these.

- [ ] **Step 1: Add the field and the type to `ipc.ts`**

```ts
export type DeviceInfo = {
  instance_id: string;
  machine_id: string;
  transport: TransportKind;
  candidate: boolean;
  // null means this cutter is attached to this computer. A Cut Host's cutters carry the id of
  // the host that owns them, which is what every call routes on.
  host: string | null;
};

export type PairedHostView = {
  id: string;
  name: string;
  address: string;
  /** Why this host cannot be reached, or null when it can. */
  unreachable: string | null;
};
```

- [ ] **Step 2: Add the five wrappers, beside the existing device commands**

```ts
export async function listHosts(): Promise<PairedHostView[]> {
  return invoke("list_hosts");
}

/** The fingerprint a host presents, for the operator to confirm. Sends no token. */
export async function probeHost(address: string): Promise<string> {
  return invoke("probe_host", { address });
}

export async function testHost(
  address: string, token: string, fingerprint: string,
): Promise<DeviceInfo[]> {
  return invoke("test_host", { address, token, fingerprint });
}

export async function pairHost(
  name: string, address: string, token: string, fingerprint: string,
): Promise<PairedHostView> {
  return invoke("pair_host", { name, address, token, fingerprint });
}

export async function forgetHost(id: string): Promise<void> {
  return invoke("forget_host", { id });
}
```

Match the existing wrappers' exact `invoke` idiom in that file rather than the sketch above if they differ.

- [ ] **Step 3: Update the other three mirrors**

Every `DeviceInfo` literal must gain `host`. In `e2e/smoke.spec.ts` that is the local `type DeviceInfo` declaration plus both fixtures in `const devices` — both are local hardware, so both take `host: null`.

- [ ] **Step 4: Give the e2e fake the five host commands**

Add to the fake backend's command map. `list_hosts` returns `[]` by default so existing e2e assertions are unchanged; the others are stubs a later task will exercise.

- [ ] **Step 5: Write the failing test**

In `viewmodel.test.ts`:

```ts
it("distinguishes a cutter on this computer from one on a Cut Host", () => {
  const local: DeviceInfo = { ...aDevice(), host: null };
  const remote: DeviceInfo = { ...aDevice(), instance_id: "usb:sn:PI", host: "host-1" };
  expect(local.host).toBeNull();
  expect(remote.host).toBe("host-1");
});
```

- [ ] **Step 6: Run the tests, build, commit**

```sh
npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run e2e
npm --prefix apps/desktop/ui run build
git add apps/desktop/ui && git commit
```

---

### Task 2: group the device list by host, and keep unreachable cutters visible

**Files:**
- Create: `apps/desktop/ui/src/hosts/deviceList.ts`, `deviceList.test.ts`

**Interfaces:**
- Consumes: `DeviceInfo`, `PairedHostView` (Task 1)
- Produces: `groupDevices(devices, hosts): DeviceSection[]`

- [ ] **Step 1: Write the failing tests**

```ts
it("keeps an unreachable host's cutters listed, with the reason", () => {
  const hosts: PairedHostView[] = [{ id: "host-1", name: "Workshop Pi", address: "pi.local:7878",
                                     unreachable: "the host could not be reached (timed out)" }];
  const devices: DeviceInfo[] = [{ ...aDevice(), instance_id: "usb:sn:A", host: "host-1" }];
  const sections = groupDevices(devices, hosts);
  const remote = sections.find(s => s.hostId === "host-1")!;
  expect(remote.devices).toHaveLength(1);
  expect(remote.unreachable).toContain("could not be reached");
});

it("lists a paired host that has no cutters, rather than hiding it", () => {
  const sections = groupDevices([], [{ id: "host-1", name: "Workshop Pi",
                                       address: "pi.local:7878", unreachable: null }]);
  expect(sections.find(s => s.hostId === "host-1")).toBeDefined();
});

it("puts this computer's cutters first", () => {
  const sections = groupDevices(
    [{ ...aDevice(), host: "host-1" }, { ...aDevice(), instance_id: "usb:sn:L", host: null }],
    [{ id: "host-1", name: "Workshop Pi", address: "pi.local:7878", unreachable: null }],
  );
  expect(sections[0].hostId).toBeNull();
});
```

A cutter that vanishes looks like one that was never paired (#42) — that is why the unreachable section keeps its rows instead of dropping them.

- [ ] **Step 2: Run them and watch them fail** — `npm --prefix apps/desktop/ui test -- deviceList`

- [ ] **Step 3: Implement**

```ts
export type DeviceSection = {
  /** null for cutters attached to this computer. */
  hostId: string | null;
  title: string;
  address: string | null;
  unreachable: string | null;
  devices: DeviceInfo[];
};

export function groupDevices(
  devices: DeviceInfo[], hosts: PairedHostView[],
): DeviceSection[] {
  const local: DeviceSection = {
    hostId: null, title: "This computer", address: null, unreachable: null,
    devices: devices.filter(d => d.host === null),
  };
  return [local, ...hosts.map(h => ({
    hostId: h.id, title: h.name, address: h.address, unreachable: h.unreachable,
    devices: devices.filter(d => d.host === h.id),
  }))];
}
```

- [ ] **Step 4: Tests pass, build, commit**

---

### Task 3: render every control from `actions`

**Files:**
- Modify: `apps/desktop/ui/src/hosts/deviceList.ts`, `deviceList.test.ts`

**Interfaces:**
- Produces: `deviceBadge(status: CutStatus | null): { label: string; tone: "idle" | "busy" | "attention" | "gone" }`

- [ ] **Step 1: Write the failing test**

```ts
it("decides what is offered from actions, never from the phase", () => {
  // A cutter on a host we cannot reach reports Disconnected with every action false. The badge
  // must say so, and nothing may offer a cut for it.
  const gone: CutStatus = { ...aStatus(), actions: { cut: false, cancel: false, resume: false, confirm: false } };
  expect(deviceBadge(gone).tone).toBe("gone");

  // Same phase string, different actions: a cut is legal. If the badge read `phase` these two
  // would be indistinguishable, which is the bug this guards.
  const ready: CutStatus = { ...gone, actions: { ...gone.actions, cut: true } };
  expect(deviceBadge(ready).tone).toBe("idle");
});
```

- [ ] **Step 2: Run it and watch it fail**

- [ ] **Step 3: Implement, reading only `actions` and `ended`** — never `phase` for legality. `phase` may still be shown as descriptive text; it must not decide anything.

- [ ] **Step 4: Tests pass, build, commit**

---

### Task 4: the pairing flow — probe, confirm, test, save

**Files:**
- Create: `apps/desktop/ui/src/hosts/pairing.ts`, `pairing.test.ts`, `PairHostDialog.tsx`

**Interfaces:**
- Consumes: `probeHost`, `testHost`, `pairHost` (Task 1)
- Produces: `PairingState`, `pairingReducer` — a pure state machine the `.tsx` renders

The order is the point and comes straight from the spec: **the fingerprint is shown before the token is sent, and nothing is saved until a Test lists the host's cutters.** A pairing that saves first and discovers later is how users end up re-adding hosts.

- [ ] **Step 1: Write the failing tests**

```ts
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
```

- [ ] **Step 2: Run them and watch them fail**

- [ ] **Step 3: Implement the state machine**

States: `idle` → `probing` → `confirm` (carries the fingerprint) → `testing` → `paired` | `failed`. Injected effects (`probe`/`test`/`save`) keep it testable without a backend.

- [ ] **Step 4: Write the thin `PairHostDialog.tsx`** over it — address, token, name; the fingerprint shown as confirmable text; the Test result rendered as the list of cutters found. Errors render the `IpcError.message` as-is; do not reword a refusal in TypeScript, since the Rust side owns that prose (#94).

- [ ] **Step 5: Tests pass, build, commit**

---

### Task 5: forget a host, and the four error surfaces

**Files:**
- Modify: `apps/desktop/ui/src/hosts/deviceList.ts`, `deviceList.test.ts`, `PairHostDialog.tsx`, `CutDialog.tsx`

- [ ] **Step 1: Write the failing tests** covering the spec's error table:

```ts
it("keeps a host that refuses to be forgotten, and says why", async () => {
  // The Rust side refuses while a cut is active on that host: the desktop would otherwise
  // discard the token for a Job it can no longer cancel.
  const s = await runForget("host-1", async () => {
    throw { code: "host_busy", message: "a cut is active on this host; cancel it before forgetting" };
  });
  expect(s.hosts.map(h => h.id)).toContain("host-1");
  expect(s.message).toContain("cancel it before forgetting");
});
```

- [ ] **Step 2: Run and watch fail**

- [ ] **Step 3: Implement** the forget action plus:
  - token rejected → the host row says it needs re-pairing; **the stored token is not cleared** (the Rust side keeps it until replaced)
  - a poll that fails mid-cut → the row goes **stale, not blank** — the cut is still running on the Pi, and a blank row reads as "finished"

- [ ] **Step 4: Tests pass, build, commit**

---

### Task 6: poll at 1 Hz while a cut is being watched, and stop when it is not

**Files:**
- Modify: `apps/desktop/ui/src/cut/CutDialog.tsx`, `apps/desktop/ui/src/hosts/deviceList.ts`
- Test: `apps/desktop/ui/e2e/smoke.spec.ts`

- [ ] **Step 1: Write the failing e2e assertion** — with a dialog open, the fake backend records repeated `get_device_state` calls; with it closed, the count stops rising.

- [ ] **Step 2: Run and watch fail**

- [ ] **Step 3: Implement** a 1 Hz interval that starts when a dialog opens and is cleared when it closes or the component unmounts. A leaked interval keeps a Cut Host connection warm forever, and the daemon caps clients at eight (#103) — so clearing it is the deliverable, not a tidiness.

  `// ponytail:` one interval for the whole list, not one per host — `list_devices` already refreshes every host in one call.

- [ ] **Step 4: Full verification, build, commit**

```sh
npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run e2e
npm --prefix apps/desktop/ui run build
cargo test --workspace --locked    # the mirror must still match the Rust shape
```

---

## Out of scope

- **#107** — warning when re-pairing an address whose fingerprint changed. `probe_host` returns a bare `String`, so the dialog cannot yet tell "never paired" from "certificate changed". Filed; not this plan.
- **#70** — generating the mirrors instead of hand-writing them. This plan pays the tax one more time.
- **Event push.** Progress comes from polling, by decision; a reader thread on the request/reply connection was rejected in the spec.
- **Hardware verification** — phase 3, recorded in `apps/desktop/MANUAL-CHECKLIST.md`.
