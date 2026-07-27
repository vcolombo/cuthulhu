<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# MachineCaps over IPC — design

Date: 2026-07-27
Status: approved (brainstorming complete)

## Purpose

Candidate 4 of `docs/superpowers/reviews/2026-07-27-architecture-review.md`: the cut dialog holds a
keyed literal table of the same three booleans each Driver already declares, so `fieldDisabled` greys
the Puma's speed and force from a TypeScript copy while `preflight` range-checks them from the Rust
one. They agree today because someone typed them to agree.

Make the driver's `MachineCaps` reach the UI over IPC and delete the copy.

This is a UX-correctness fix, not a safety one. `preflight` *ignores* out-of-range speed and force on
machines that do not support them (`crates/cutplan/src/preflight.rs:100,107` — "unsupported speed is
ignored (drivers skip it)"), so a wrong table can only mis-grey a field. It cannot send a machine
something it would mishandle.

## Two corrections to the review

The review says of `MachineCaps`: *"The type exists, already serializes."* It does not.
`crates/driver-core/src/lib.rs:21` derives `Clone, Copy, Debug, PartialEq` — no `Serialize`.

The review also says the change lets "the dialog stop knowing machine names". It does not go that
far, and is not asked to. `CutDialog` still compares `machine_id` strings for the mismatch banner and
the "Convert to {id}" button (`CutDialog.tsx:149,224`). Those are machine *identity*, not machine
*capability*; removing them is Candidate 3's territory.

## Scope

In scope: delete `CAPS` and `DEFAULT_CAPS` (`CutDialog.tsx:16-24`) and the `ponytail:` comment above
them; caps arrive from the backend.

Out of scope, decided deliberately:

- **`machine_id` comparisons in the dialog.** See above.
- **Removing `needsOperatorPassConfirm` from the TypeScript `Caps` type.** Nothing in the UI reads
  it — the dialog renders its confirm button from `status.actions.confirm` (`CutDialog.tsx:340`). It stays
  because the type now mirrors a Rust struct sent whole; a field that is present because the backend
  sends it is honest in a way a hand-typed unused field was not.
- **`document::machine::builtin_profiles()`** (`crates/document/src/machine.rs:14`), which holds a
  further copy of the machine ids that `driver-registry`'s test does not pin. Real, and Candidate 3's.

## Decisions

**Caps are per machine *model*, not per device instance.** `CONTEXT.md:88` defines MachineCaps as
"What a machine model can be told and what it needs from the operator". `DeviceInfo` is per instance
— `instance_id`, `transport`, `candidate`. This is what rules out the review's suggested shape.

**No-device fallback keeps today's behaviour**: speed and force stay enabled before a device is
connected, so passes can be set up offline. `DEFAULT_CAPS` becomes a single unkeyed `ALL_ENABLED`
constant, staying in `CutDialog.tsx` where `DEFAULT_CAPS` was — it is the dialog's pre-connection
placeholder, not a wire type, and commented as such rather than as any machine's claim.

**`MachineCaps` copies `Actions` exactly** (`crates/driver-core/src/status.rs:41-48`): `Serialize`
only — it never travels UI-to-Rust — plus `#[serde(rename_all = "camelCase")]`. `Actions` is the
closest sibling in the tree: an outbound-only struct of booleans a caller renders controls from. The
camelCase choice means the existing TypeScript `Caps` type needs no edit and `fieldDisabled` keeps
its signature.

## Architecture

One new backend question, keyed the way `list_presets` is already keyed.

```
CutDialog                    ipc.rs              device.rs                 driver-registry
   │                            │                    │                            │
   ├─ machineCaps(machine_id) ──► machine_caps ──────► caps_for(machine_id) ──────► driver_for(id)
   │                            │                    │                            └─► .caps()
   ◄──────── MachineCaps ───────┴────────────────────┴────────────────────────────────┘
```

The rule that a Puma has no speed knob stays in `crates/driver-hpgl/src/encode.rs:18-20` and is now
read rather than retyped.

`DeviceManagerHandle` already holds `factory: Arc<dyn DeviceBackendFactory>`
(`apps/desktop/src/device.rs:45`), so the lookup is one method:

```rust
// Capability is the driver's answer, not ours; asking the factory keeps the thing
// that encodes the bytes the thing that says what they can carry.
pub fn caps_for(&self, machine_id: &str) -> Result<MachineCaps, IpcError> {
    self.factory
        .driver_for(machine_id)
        .map(|d| d.caps())
        .ok_or_else(|| IpcError::new("unknown_machine", format!("no driver for '{machine_id}'")))
}
```

`Result`, not `Option`: `driver_for` returning `None` means the UI asked about a machine the registry
cannot build, which is a desync worth surfacing. (Contrast `list_presets`, where an empty vec is a
legitimate answer.)

The dialog already fetches presets by `machine_id` in the two places it needs caps — on mount
(`CutDialog.tsx:96-103`) and after connect (`:142`) — so caps need no new fetch lifecycle, only a
second call at each site.

### Rejected alternatives

**`caps` field on `DeviceInfo`** (the review's proposal). Smallest UI diff, but: nine construction
sites, six of them tests; `DeviceInfo` round-trips, so `connect_device(info)` would accept caps from
the client that the backend must ignore — a serialized field the server is obliged to distrust; and
it attaches caps to `candidate: true` serial ports, where the machine is only a guess, so the dialog
would grey a Puma's knobs before anyone confirmed it is a Puma.

**Widen `get_connected_device` to `{ info, caps }`.** No `DeviceInfo` churn and caps appear only
where meaningful, but `connect()` sets `connected` from the *list* record (`CutDialog.tsx:140`), so
the dialog would need a re-fetch after connecting that it does not do today. New lifecycle, no gain
over the chosen approach, and it changes an IPC return shape the e2e fake mirrors.

**Extend `list_machines`.** It returns `document::builtin_profiles()` — `document`'s copy of machine
identity, not the driver's. Wrong owner: it would open a seventh home rather than close one.

## Components

| File | Change |
| --- | --- |
| `crates/driver-core/src/lib.rs:21-22` | Add `Serialize` and `#[serde(rename_all = "camelCase")]` to `MachineCaps` |
| `apps/desktop/src/device.rs` | New `DeviceManagerHandle::caps_for` (above) |
| `apps/desktop/src/ipc.rs` | New `#[tauri::command] machine_caps(dev, machine_id)`, thin — logic stays in `device.rs` per `CLAUDE.md` |
| `apps/desktop/src/main.rs:47-58` | Register `ipc::machine_caps` in the invoke handler |
| `apps/desktop/ui/src/ipc.ts` | Add `machineCaps(machineId)` beside `listPresets`, untyped to match it |
| `apps/desktop/ui/src/cut/CutDialog.tsx:16-24,148` | Delete `CAPS`, `DEFAULT_CAPS`, the `ponytail:` comment; add caps state and the two fetches |
| `apps/desktop/ui/e2e/smoke.spec.ts` | Add a deliberately constant `machine_caps` handler (see Testing) |
| `apps/desktop/ui/dist/` | Rebuild and commit — CI gate |

`cut/viewmodel.ts` is **not** modified: `Caps` stays there. An earlier draft moved it to `ipc.ts` on the
theory that wire types belong there, but `viewmodel.ts:19` already hosts `ConfiguredPassDto` and
`CutRequest` under a "Wire types" heading, and `listPresets` (`ipc.ts:201-203`) declares no return
type at all — `CutDialog` casts `p as Preset[]` with `Preset` defined in `viewmodel.ts`. `machineCaps`
copies that: untyped in `ipc.ts`, cast to `Caps` at the call site.

Following the existing pattern also avoids a real hazard. No unit test imports `ipc.ts` today, so
making `viewmodel.ts` import it would newly pull `@tauri-apps/api/core` into vitest's module graph
for `viewmodel.test.ts` — a new failure mode in the one test file this change most wants to leave
alone.

## Error handling

**Failures stay independent.** Caps and presets are fetched in separate promise chains, not a
`Promise.all`. Their failure causes are unrelated — a corrupt `presets.json` must not leave caps
unfetched, and an unknown machine must not blank the preset dropdown.

```ts
ipc.machineCaps(info.machine_id).then(...).catch(e => onError(ipc.ipcErrorMessage(e)));
ipc.listPresets(info.machine_id).then(p => setPresets(p as Preset[])).catch(...);
```

**Unknown `machine_id`** → `IpcError { code: "unknown_machine" }`, surfaced through the dialog's
existing `onError`. Caps stay unset and fields stay enabled; preflight ignores unsupported values
regardless, so nothing unsendable results.

**Not connected** → caps unset, fields enabled. Unchanged from today.

**Device switched mid-dialog.** Today `caps` derives synchronously from `connected.machine_id` and so
can never disagree with the connected device. An async fetch opens a window where one machine's caps
are shown against another's. The state carries its own key:

```ts
const [capsFor, setCapsFor] = useState<{ machineId: string; caps: Caps } | null>(null);
const caps = connected && capsFor?.machineId === connected.machine_id ? capsFor.caps : ALL_ENABLED;
```

The defect being fixed is capability data drifting from the machine it describes; replacing a
synchronous lookup with an async one without this guard would reintroduce it on a millisecond
timescale.

## Testing

New tests, all in `apps/desktop/src/device.rs`, whose test module already has a `TestDriver` and fake
factory (`:292-330`). `serde_json` is already a dependency of `apps/desktop` (`Cargo.toml:29`) and is
absent from `driver-core`, so the wire-shape test lives desktop-side and adds no dependency — which
matters against the `--locked` gate.

1. `caps_for` on a known machine id returns that driver's caps.
2. `caps_for` on an unknown id is an `IpcError` with code `unknown_machine`.
3. `serde_json::to_value(MachineCaps { .. })` has keys `supportsSpeed`, `supportsForce`,
   `needsOperatorPassConfirm`.

Test 3 guards a silent, user-visible failure: drop the `rename_all` attribute and TypeScript reads
`caps.supportsSpeed` as `undefined`, `!undefined` is `true`, and every field greys out on every
machine. Nothing catches that today — `Actions` and `CutStatus` carry the same attribute with no test
pinning either.

One test-infrastructure edit is required first: `TestFactory::driver_for` (`device.rs:308`) ignores
its argument and returns `Some` for any id, so test 2 cannot fail against it. It gains a `match` on
the id — which is also what the real registry does. Existing tests all use `cameo5` and stay green.

Unchanged deliberately:

- `crates/cutplan` and `apps/desktop/ui/src/cut/viewmodel.ts`, neither of which this change touches.
- `viewmodel.test.ts`'s four `fieldDisabled` cases. The pure function that *applies* the rule was
  never the problem, only the literal that *supplied* it; needing to edit these would mean the seam
  was cut in the wrong place.
- `crates/driver-silhouette/src/encode.rs:136` and `crates/driver-hpgl/src/encode.rs:96`, which
  already assert each driver's caps. They become the coverage for the mapping the UI used to restate
  — it stops needing its own test because it stops being its own copy.

**The e2e fake gets one constant, not a table.** It needs a `machine_caps` handler or the invoke
throws, but a `{cameo5: …, puma: …}` lookup would put the literal just deleted back into the repo one
directory over, leaving the copy count unchanged at two:

```ts
// Deliberately not a per-machine table: that mapping is pinned in Rust by each driver's
// caps test, and restating it here would recreate the copy this change removed.
machine_caps: () => ({ supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false }),
```

The e2e test does not need per-machine fidelity; it needs caps to flow from IPC into the fields at
all. Per-machine correctness is Rust's to prove.

No `MANUAL-CHECKLIST.md` entry: nothing here changes bytes on the wire, so there is nothing only
hardware can confirm.

## Verification

```sh
cargo test --workspace --locked
npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run build     # then commit dist/ — CI gate
npm --prefix apps/desktop/ui run e2e
```
