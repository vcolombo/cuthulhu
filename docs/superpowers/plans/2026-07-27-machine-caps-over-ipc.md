<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# MachineCaps over IPC Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The cut dialog learns a machine's speed/force capability by asking the backend, instead of holding its own hardcoded table of the same booleans.

**Architecture:** Each `Driver` already declares `MachineCaps`. Add a `machine_caps(machine_id)` Tauri command that resolves the driver through the factory the desktop already holds and returns its caps, mirroring the existing `list_presets(machine_id)` command. The dialog fetches caps in the two places it already fetches presets, and the literal table at `CutDialog.tsx:20-24` is deleted.

**Tech Stack:** Rust (serde, Tauri v2 commands), TypeScript/React, vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-07-27-machine-caps-over-ipc-design.md`

## Global Constraints

- SPDX header on every file: `// SPDX-License-Identifier: GPL-3.0-or-later` (or the language's comment form).
- Comments explain *why*, not *what*. Do not add restating comments.
- `cargo test --workspace --locked` is what CI runs. `--locked` is mandatory. **This change adds no dependency, so `Cargo.lock` must not change.** If it does, something went wrong.
- `apps/desktop/ui/dist/` is committed. Any change under `ui/src` requires `npm --prefix apps/desktop/ui run build` and committing the rebuilt `dist/` in the same change, or CI fails.
- Vocabulary from `CONTEXT.md` is normative: **MachineCaps**, **Driver**, **Preflight**. Never write "capabilities flags", "support matrix", "backend", or "plugin".
- Commit subjects: imperative with the reason attached.
- Do **not** modify `apps/desktop/ui/src/cut/viewmodel.ts` or `apps/desktop/ui/src/cut/viewmodel.test.ts`. Leaving them untouched is the signal that the seam was cut in the right place. If you find yourself needing to edit `fieldDisabled`'s signature, stop and re-read the spec.

---

## File Structure

| File | Responsibility after this change |
| --- | --- |
| `crates/driver-core/src/lib.rs` | `MachineCaps` becomes serializable to the UI, in camelCase, exactly as `Actions` already is |
| `apps/desktop/src/device.rs` | `DeviceManagerHandle::caps_for` — the one place that turns a machine id into that Driver's caps |
| `apps/desktop/src/ipc.rs` | Thin `machine_caps` command; no logic (project rule) |
| `apps/desktop/src/main.rs` | Command registration |
| `apps/desktop/ui/src/ipc.ts` | `machineCaps(machineId)` wrapper, untyped like its neighbour `listPresets` |
| `apps/desktop/ui/src/cut/CutDialog.tsx` | Holds caps as state fetched from the backend; owns the not-connected fallback |
| `apps/desktop/ui/e2e/smoke.spec.ts` | Fake answers `machine_caps` with one constant, deliberately not a per-machine table |

---

## Task 1: Backend answers `machine_caps(machine_id)`

**Files:**
- Modify: `crates/driver-core/src/lib.rs:21-22`
- Modify: `apps/desktop/src/device.rs` (add method after `list_devices` at `:70-72`; edit `TestFactory::driver_for` at `:308-318`; add tests in the `mod tests` block)
- Modify: `apps/desktop/src/ipc.rs` (after `list_presets` at `:155-157`)
- Modify: `apps/desktop/src/main.rs:58` (invoke handler list)

**Interfaces:**
- Consumes: `driver_core::MachineCaps`, `DeviceBackendFactory::driver_for`, `Driver::caps`, `crate::device::IpcError` — all already exist.
- Produces:
  - `MachineCaps` serializes to `{"supportsSpeed": bool, "supportsForce": bool, "needsOperatorPassConfirm": bool}`
  - `DeviceManagerHandle::caps_for(&self, machine_id: &str) -> Result<MachineCaps, IpcError>`
  - Tauri command `machine_caps`, invoked from JS as `invoke("machine_caps", { machineId })`

### Steps

- [ ] **Step 1: Make the test factory honour the machine id**

`TestFactory::driver_for` currently ignores its argument and returns `Some` for any id, so a test for the unknown-machine case cannot fail. Real registries match on the id; this one should too.

In `apps/desktop/src/device.rs`, replace the `driver_for` body at `:308-318`:

```rust
        fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
            // Mirrors the real registry: an id nobody claims resolves to nothing,
            // rather than silently handing back some other machine's encoder.
            if machine_id != "cameo5" {
                return None;
            }
            Some(Box::new(TestDriver {
                profile: MachineProfile { id: "cameo5".into(), name: "Test Cameo".into(), width_mm: 500.0, height_mm: 500.0 },
                // A machine that cannot be polled parks the cut at `AwaitingConfirmation`
                // instead of driving it to completion, so a cut submitted here stops at a
                // stable mid-flight phase. `MockTransport` answers no status query, so a
                // pollable machine would instead sit out the manager's 60s completion
                // budget and then fail.
                caps: MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true },
            }))
        }
```

- [ ] **Step 2: Confirm the existing suite is still green after that edit**

Run: `cargo test -p desktop`
Expected: PASS. Every existing test connects with `machine_id: "cameo5"` (`test_instance()` at `:324-331`), so none of them hit the new `None` branch.

- [ ] **Step 3: Write the three failing tests**

Add to the `mod tests` block in `apps/desktop/src/device.rs`, at the end (before the closing `}`):

```rust
    #[test]
    fn caps_for_returns_the_drivers_own_answer() {
        let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
        let caps = dev.caps_for("cameo5").expect("known machine id");
        assert_eq!(
            caps,
            MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true }
        );
    }

    #[test]
    fn caps_for_unknown_machine_is_an_error_not_a_default() {
        let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
        let err = dev.caps_for("nope").expect_err("no driver claims this id");
        assert_eq!(err.code, "unknown_machine");
    }

    /// The UI reads `caps.supportsSpeed`. Drop the serde rename and it reads
    /// `undefined`, `!undefined` is `true`, and every field greys out on every
    /// machine — silent, and wrong in the direction that looks plausible.
    #[test]
    fn machine_caps_serializes_in_the_casing_the_ui_reads() {
        let json = serde_json::to_value(MachineCaps {
            supports_speed: true,
            supports_force: false,
            needs_operator_pass_confirm: true,
        })
        .unwrap();
        assert_eq!(json["supportsSpeed"], serde_json::json!(true));
        assert_eq!(json["supportsForce"], serde_json::json!(false));
        assert_eq!(json["needsOperatorPassConfirm"], serde_json::json!(true));
    }
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p desktop caps`
Expected: FAIL to compile, with two distinct errors — `no method named 'caps_for' found for struct 'DeviceManagerHandle'`, and `the trait bound 'MachineCaps: Serialize' is not satisfied`.

A compile failure is the correct red here: both tests name things that do not exist yet.

- [ ] **Step 5: Make `MachineCaps` serializable**

In `crates/driver-core/src/lib.rs`, replace lines 21-22:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineCaps { pub supports_speed: bool, pub supports_force: bool, pub needs_operator_pass_confirm: bool }
```

`Serialize` only, not `Deserialize`: caps travel Rust-to-UI and never back. This copies `Actions` at `crates/driver-core/src/status.rs:41-48` exactly — the closest sibling in the tree, and for the same reason.

`serde::Serialize` is already imported at `lib.rs:3`. No `Cargo.toml` change, in this crate or any other.

- [ ] **Step 6: Add `caps_for`**

In `apps/desktop/src/device.rs`, immediately after `list_devices` (which ends at `:72`):

```rust
    /// Capability is the Driver's answer, not ours — asking the factory keeps the
    /// thing that encodes the bytes the thing that says what they can carry.
    /// `Result`, not `Option`: an id the registry cannot build means the caller
    /// is out of sync with it, which is worth surfacing rather than defaulting.
    pub fn caps_for(&self, machine_id: &str) -> Result<MachineCaps, IpcError> {
        self.factory
            .driver_for(machine_id)
            .map(|d| d.caps())
            .ok_or_else(|| IpcError::new("unknown_machine", format!("no driver for '{machine_id}'")))
    }
```

Add `MachineCaps` to the `driver_core` import at `device.rs:11`:

```rust
use driver_core::{CutStatus, DeviceBackendFactory, DeviceInfo, MachineCaps};
```

Leave the test module's own `use driver_core::{... MachineCaps ...}` at `:292` alone. An explicit import shadows the one arriving via `use super::*` without a warning, and both are used.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p desktop caps`
Expected: PASS, 3 tests.

- [ ] **Step 8: Add the IPC command**

In `apps/desktop/src/ipc.rs`, after `list_presets` (ends `:157`):

```rust
#[tauri::command]
pub fn machine_caps(dev: tauri::State<DeviceManagerHandle>, machine_id: String) -> Result<MachineCaps, IpcError> {
    dev.caps_for(&machine_id)
}
```

Add `MachineCaps` to the `driver_core` import at `ipc.rs:5`:

```rust
use driver_core::{CutStatus, DeviceInfo, MachineCaps};
```

Keep it thin — the logic stays in `device.rs`, per `CLAUDE.md`.

- [ ] **Step 9: Register the command**

In `apps/desktop/src/main.rs`, add to the `invoke_handler` list after `ipc::list_presets` (`:58`):

```rust
            ipc::list_presets,
            ipc::machine_caps,
```

- [ ] **Step 10: Run the full workspace suite**

Run: `cargo test --workspace --locked`
Expected: PASS. An unregistered or misnamed command will not fail here — registration is checked by the e2e run in Task 2.

- [ ] **Step 11: Verify `Cargo.lock` did not change**

Run: `git status --short Cargo.lock`
Expected: no output. This change adds no dependency; a modified lock file means something unintended was pulled in, and CI's `--locked` will reject it.

- [ ] **Step 12: Commit**

```bash
git add crates/driver-core/src/lib.rs apps/desktop/src/device.rs apps/desktop/src/ipc.rs apps/desktop/src/main.rs
git commit -m "Let the desktop ask a Driver what it can be told, so the UI stops guessing

MachineCaps gains Serialize plus camelCase renaming, copying Actions, which
is outbound-only for the same reason. caps_for resolves a machine id through
the factory the handle already holds and errors on an id no Driver claims,
rather than defaulting to something permissive.

The casing is pinned by a test: without the rename the UI reads undefined,
!undefined is true, and every field greys out on every machine."
```

---

## Task 2: The dialog reads caps instead of declaring them

**Files:**
- Modify: `apps/desktop/ui/src/ipc.ts` (after `listPresets` at `:201-203`)
- Modify: `apps/desktop/ui/src/cut/CutDialog.tsx:16-24` (delete), `:80-105` (mount fetch), `:136-146` (connect fetch), `:148` (caps derivation)
- Modify: `apps/desktop/ui/e2e/smoke.spec.ts:418` (fake handler)
- Modify: `apps/desktop/ui/dist/` (rebuilt, committed)
- **Do not modify:** `apps/desktop/ui/src/cut/viewmodel.ts`, `apps/desktop/ui/src/cut/viewmodel.test.ts`

**Interfaces:**
- Consumes: `machine_caps` command from Task 1, invoked as `invoke("machine_caps", { machineId })` and resolving to `{ supportsSpeed, supportsForce, needsOperatorPassConfirm }`.
- Consumes: `Caps` and `fieldDisabled(field, caps)` from `./viewmodel`, both unchanged.
- Produces: nothing later tasks depend on. This is the last task.

### Steps

- [ ] **Step 1: Add the e2e fake handler first**

This is test infrastructure for a command that does not exist in the fake yet; without it, every e2e test that connects a device throws.

In `apps/desktop/ui/e2e/smoke.spec.ts`, replace line 418:

```ts
    list_presets: () => [],
    // Deliberately one constant, not a per-machine table: that mapping is pinned in
    // Rust by each Driver's own caps test, and restating it here would recreate the
    // copy this change removed — in a file nobody thinks of as production code.
    machine_caps: () => ({ supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false }),
```

- [ ] **Step 2: Add the `machineCaps` wrapper**

In `apps/desktop/ui/src/ipc.ts`, after `listPresets` (ends `:203`):

```ts
export async function machineCaps(machineId: string) {
  return invoke("machine_caps", { machineId });
}
```

No return type, matching `listPresets` directly above it — the caller casts, and `Caps` stays in `cut/viewmodel.ts` beside the other cut wire types.

- [ ] **Step 3: Delete the literal table**

In `apps/desktop/ui/src/cut/CutDialog.tsx`, delete lines 16-24 entirely (the `ponytail:` comment, `CAPS`, and `DEFAULT_CAPS`) and put in their place:

```tsx
// What the fields allow before any machine has been asked. Not a machine's claim —
// a placeholder that keeps passes editable offline. Preflight ignores speed/force a
// machine does not support, so an optimistic default here cannot mis-send anything.
const ALL_ENABLED: Caps = { supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false };
```

- [ ] **Step 4: Hold caps as state, keyed by the machine they describe**

In `apps/desktop/ui/src/cut/CutDialog.tsx`, after the `connected` state at `:81`:

```tsx
  // The machine id rides along with the caps: `connected` can change before an
  // in-flight fetch resolves, and showing one machine's capability against another
  // is the exact defect this fetch was added to remove.
  const [capsFor, setCapsFor] = useState<{ machineId: string; caps: Caps } | null>(null);
```

- [ ] **Step 5: Fetch caps on mount**

In the mount `useEffect`, the `getConnectedDevice` chain currently reads (`:96-103`):

```tsx
    ipc
      .getConnectedDevice()
      .then((info) => {
        setConnected(info);
        if (!info) return;
        return ipc.listPresets(info.machine_id).then((p) => setPresets(p as Preset[]));
      })
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
```

Replace the `if (!info) return;` line and the line after it with:

```tsx
        if (!info) return;
        // Separate chains on purpose: a corrupt presets file must not leave caps
        // unfetched, and an unknown machine must not blank the preset dropdown.
        ipc
          .machineCaps(info.machine_id)
          .then((c) => setCapsFor({ machineId: info.machine_id, caps: c as Caps }))
          .catch((e) => onError(ipc.ipcErrorMessage(e)));
        return ipc.listPresets(info.machine_id).then((p) => setPresets(p as Preset[]));
```

- [ ] **Step 6: Fetch caps on connect**

Replace the body of `connect` at `:136-146`:

```tsx
  const connect = (info: ipc.DeviceInfo) => {
    ipc
      .connectDevice(info)
      .then(() => {
        setConnected(info);
        refreshDeviceState();
        ipc
          .machineCaps(info.machine_id)
          .then((c) => setCapsFor({ machineId: info.machine_id, caps: c as Caps }))
          .catch((e) => onError(ipc.ipcErrorMessage(e)));
        return ipc.listPresets(info.machine_id);
      })
      .then((p) => setPresets(p as Preset[]))
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };
```

- [ ] **Step 7: Derive `caps` from the keyed state**

Replace line 148:

```tsx
  const caps = connected && capsFor?.machineId === connected.machine_id ? capsFor.caps : ALL_ENABLED;
```

The `connected &&` guard matters: without it, both being `null` makes the comparison `undefined === undefined`, which is `true`, and `capsFor.caps` throws.

- [ ] **Step 8: Typecheck and run the unit tests**

Run: `npm --prefix apps/desktop/ui run build`
Expected: PASS. This runs `tsc` then `vite build`, so it catches the type wiring.

Run: `npm --prefix apps/desktop/ui test`
Expected: PASS, with `src/cut/viewmodel.test.ts` unchanged and green. If those four `fieldDisabled` tests needed edits, the seam was cut in the wrong place — stop and re-read the spec.

- [ ] **Step 9: Run the e2e suite**

Run: `npm --prefix apps/desktop/ui exec -- playwright install --with-deps chromium` (once per checkout only; skip if already installed)

Run: `npm --prefix apps/desktop/ui run e2e`
Expected: PASS. This is what proves the command name matches end to end — a typo in `machine_caps` or a missed registration in `main.rs` surfaces here as an error banner in the connect tests.

- [ ] **Step 10: Verify the actual behaviour once, by hand**

There is no component-test infrastructure in this repo, so the link from "fetched caps" to "greyed field" is checked once manually.

Run: `(cd apps/desktop && cargo tauri dev)`

With no device connected, open the cut dialog and confirm speed and force are editable. Then connect a Puma (or any serial candidate) and confirm both fields grey out; connect a Cameo and confirm they do not.

If no hardware is available, say so in the commit body rather than claiming it was verified. Per-machine correctness is already pinned in Rust by `crates/driver-hpgl/src/encode.rs:96` and `crates/driver-silhouette/src/encode.rs:136`; what this check adds is only that the value reaches the field.

- [ ] **Step 11: Confirm the untouchable files are untouched**

Run: `git status --short apps/desktop/ui/src/cut/viewmodel.ts apps/desktop/ui/src/cut/viewmodel.test.ts Cargo.lock`
Expected: no output.

- [ ] **Step 12: Commit, including the rebuilt bundle**

`dist/` is committed because `tauri::generate_context!` reads `frontendDist` at Rust compile time; CI rebuilds it and fails if the committed bundle differs.

```bash
git add apps/desktop/ui/src/ipc.ts apps/desktop/ui/src/cut/CutDialog.tsx apps/desktop/ui/e2e/smoke.spec.ts apps/desktop/ui/dist
git commit -m "Ask the backend what a machine supports, instead of keeping a second copy

CutDialog held a keyed literal of the same three booleans each Driver
declares, so fieldDisabled greyed the Puma's knobs from a TypeScript copy
while preflight range-checked them from the Rust one. They agreed because
someone typed them to agree.

Caps now arrive from machine_caps and carry the machine id they describe,
so an in-flight fetch cannot show one machine's capability against another.
The e2e fake answers with a single constant rather than a per-machine table,
which would have put the deleted literal straight back into the repo."
```

---

## Verification

Full gate, from the repository root:

```sh
cargo test --workspace --locked
npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run build
npm --prefix apps/desktop/ui run e2e
git status --short          # expect only intended files; Cargo.lock absent
```

## Done means

- `grep -rn "supportsSpeed" apps/desktop/ui/src` returns only `viewmodel.ts`'s type declaration, `fieldDisabled`'s two reads, and `ALL_ENABLED` in `CutDialog.tsx`. No per-machine table anywhere in `src/`.
- `apps/desktop/ui/src/cut/viewmodel.ts` and its test file are byte-identical to `main`.
- `Cargo.lock` is byte-identical to `main`.
