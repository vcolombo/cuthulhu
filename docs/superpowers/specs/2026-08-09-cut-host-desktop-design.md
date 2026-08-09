<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Cut Host phase 2 — the desktop reaches a Pi — design

Date: 2026-08-09
Status: approved (brainstorming complete)

## Purpose

Phase 1 (#96) built the Cut Host: a Raspberry Pi daemon that owns the USB and serial connections
to one or more cutters and runs Jobs on them for authenticated clients, so a closed laptop cannot
end a cut. It deliberately changed nothing in `apps/desktop` — `git diff main -- apps/` is empty —
so today the daemon exists and nothing can talk to it but a test.

Phase 2 connects the desktop: pair a host, see its cutters beside the local ones, dispatch to one,
and watch it cut.

## What comparable software does, and what we take from it

Four programs solve a version of this. Their agreements are worth copying and their disagreements
are worth knowing about, so the decisions below cite them rather than inventing from scratch.

| | Multiple machines per host | Where you pair | Where you watch it |
| --- | --- | --- | --- |
| **OctoPrint** | No — one printer per instance; several printers means several instances on separate ports, officially unsupported | Its own web UI | Its own web UI |
| **PrusaSlicer** | Yes, via PrusaLink multi-instance on one Pi | "Physical Printer" dialog, with a Test button | PrusaLink's web UI — the slicer sends and forgets |
| **Bambu Studio** | Yes, but explicitly **not** for LAN-only printers, and capped at 6 | The device list, with an access code | In the slicer, pushed over MQTT |
| **Cuthulhu** | Yes, one daemon, structurally isolated | The device list | The desktop (this phase) |

Three things they agree on, which this design adopts without further argument:

- **Connection settings are a named profile, separate from the machine model.** PrusaSlicer calls
  it a Physical Printer; it is what issue #42 asks for, and it is `hosts.json` here.
- **Pairing belongs where you look for a cutter**, not in a settings screen you would have to know
  to visit.
- **A Test action before you trust it.** Prusa has one; #42's acceptance criteria require one.

And one warning. Bambu's most-reported LAN complaint is that Studio forgets printers between
sessions — one operator re-adds 34 machines on every launch. Persistence being boring and correct
is a feature, not plumbing.

## Scope

In scope: `hosts.json` and pairing, per-client tokens, `DeviceInfo.host`, the `Cutters` enum in
`apps/desktop/src/device.rs`, merging remote cutters into the device list, a live status list, and
polling a remote cut's progress.

Out of scope, decided deliberately:

- **A second connection for pushed events.** OctoPrint pushes state twice a second; a 1 Hz poll of
  `Snapshot` carries the same information at a comparable rate, reuses everything phase 1 built,
  and holds no connection open — which matters while issue #97 stands, because an idle held-open
  connection is exactly the failure it describes. Phase 1 left a `ponytail:` note naming the second
  connection as the upgrade path; this is not yet the moment.
- **A status web UI on the Pi.** Two of the four comparables put monitoring on the device, and it
  is worth revisiting — but it is an HTTP server and a frontend, and the desktop's own list answers
  the same question for far less.
- **Desktop-side #59.** The desktop watches; it does not need N local `DeviceManager`s. See below.
- **Design-file transfer, queueing, mDNS browse, cloud relay.** Unchanged from phase 1.

## Decisions

### The desktop needs no second device registry

Because the Cut Host owns the cut, a desktop dispatches to cutter A, detaches, dispatches to
cutter B, and both run while its single `connected` slot only ever *watches* one. What phase 2 adds
is a read-only status list, not a local manager per cutter. #59 stays open for the desktop and is
not needed to cut on two machines at once.

### `DeviceInfo.host: Option<HostId>`, where `None` means "this computer"

Local is the absence of a host, not a mode. The device list merges local hardware with every
configured host's cutters, so a user who never pairs a Pi sees exactly today's list. The
alternative — namespacing `instance_id` strings — would hide the distinction inside a format
nothing parses.

This is the field that hits issue #70: the IPC types are mirrored by hand in four places, and the
e2e fake in `smoke.spec.ts` mirrors them again.

### `enum Cutters { Local { factory, manager }, Remote(HostClient) }`

Behind the existing mutex in `apps/desktop/src/device.rs`, with each method becoming a two-arm
match. `Local` is what exists today; a user who pairs nothing never executes the other arm. If the
file outgrows comfort the remote arm splits into its own module, which is where issue #72 points.

### Pairing lives in the device list, and ends with a Test

An "Add a Cut Host…" row sits at the bottom of the same list that shows local cutters. The user
supplies `host:port` and a token, the desktop shows the certificate fingerprint for confirmation,
and then **runs a Test that lists the host's cutters** before saving anything. A pairing that
saves first and discovers later is how Bambu's users end up re-adding printers.

`hosts.json` lives beside the existing `presets.json` in `<config_dir>/cuthulhu/`, mode `0600`,
holding for each host: a display name, address, pinned fingerprint, and token.

### Per-client named tokens, decided now rather than migrated later

Phase 1 shipped one token for the whole host, marked `// ponytail:` with per-client tokens as the
upgrade path. OctoPrint's own documentation says of exactly that model: *"The old global API key
should no longer be used, as it is a single key granting full admin access to the whole
instance."*

Phase 2 is the first time a desktop asks for credentials, so it is the cheapest moment to get this
right — afterwards it means migrating stored tokens. `cutd.toml` gains a table of named tokens
instead of one scalar; the user pastes the one for that desktop; revoking one leaves the others
alone.

A headless Pi cannot run OctoPrint's approve-in-the-browser flow, so tokens are minted by the
person configuring the daemon, not by the app. That is a smaller mechanism, and it is honest about
what a machine with no screen can do.

### The device list shows every cutter, not only the one you are aimed at

This goes past all four comparables, and it is nearly free: `Request::Snapshot` already returns
every cutter on a host in one call, so the data costs one request and the UI costs a list with a
phase badge. It removes the genuinely surprising behaviour — a cut *you* dispatched to cutter B
being invisible because the dialog is aimed at cutter A.

### Progress comes from polling, at 1 Hz, only while it is being watched

`Snapshot` returns phase, `ended`, `actions`, Pass position and byte progress for every cutter, so
one call refreshes the whole list. Polling stops when no dialog is open. `CutStatus::actions` is
what renders the controls — the desktop must not re-derive legality from a phase, which is the
rule `DeviceState` is `pub(crate)` to enforce.

### Rejected alternatives

**Pairing in a Settings screen.** Cleaner separation, but a user hunting for their Pi looks in the
device list and finds nothing. Bambu puts it in the device list; so do we.

**A reader thread on the existing connection.** One socket, but the request/reply path and the
reader then contend for one stream — a race under exactly the load that makes it matter. If events
are ever pushed, they get their own connection.

**Keeping the single host token and migrating in phase 3.** Cheaper this week, and it means
rewriting stored credentials in every paired desktop later. The mechanism is a table instead of a
scalar.

## Architecture

```
apps/desktop
  device.rs ── Cutters::Local(factory, DeviceManager) ── USB/serial ──► cutter on this computer
            └─ Cutters::Remote(HostClient) ── TLS ──► cuthulhu-cutd ──► cutters on a Pi
                                                          ▲
  hosts.json (address, fingerprint, token) ───────────────┘

  list_devices()  = local hardware  ++  every configured host's cutters
  poll 1 Hz       = Snapshot per host while a cut is being watched
```

## Components

| File | Change |
| --- | --- |
| `crates/driver-core/src/lib.rs` | `DeviceInfo` gains `host: Option<HostId>` |
| `crates/cut-host/src/config.rs` | `token` becomes a named table; `token_matches` takes the set |
| `crates/cut-host/src/client.rs` | `HostClient::test()` — connect, list, disconnect, for pairing |
| `apps/desktop/src/device.rs` | the `Cutters` enum; `list_devices` merges; snapshot polling |
| `apps/desktop/src/hosts.rs` (new) | `hosts.json` load/save, `0600`, atomic write |
| `apps/desktop/src/ipc.rs` | host add / remove / test / list commands |
| `apps/desktop/ui/src/` | the pairing dialog, the device list's host rows and phase badges |
| `apps/desktop/ui/dist/` | rebuilt and committed — CI fails on a stale bundle |
| `docs/cut-host.md` | named tokens replace the single one |

## Error handling

| Failure | Behaviour |
| --- | --- |
| Host unreachable at pairing | Test fails, nothing is saved, the dialog says which host and why |
| Fingerprint differs from the pinned one | Hard refusal, never a prompt; the saved host is left untouched |
| Host unreachable after pairing | Its cutters are listed as unreachable with repair guidance, not hidden (#42) |
| A remote dispatch refused | `Refusal` renders through `PassFault`'s `Display`; the desktop shows it as it shows a local one |
| Token rejected | The host is marked as needing re-pairing; the stored token is kept until replaced |
| Poll fails mid-cut | The row goes stale rather than blank — the cut is still running on the Pi |

## Testing

Phase 1's `cut-host` fixture starts a real Cut Host on a loopback port, so the desktop's remote arm
is testable headless against it — no Pi required.

- `hosts.json` round-trips; a corrupt file does not lose the others; the file is `0600`.
- Pairing: a wrong token, a wrong fingerprint, and an unreachable host each fail without saving.
- `list_devices` merges, and a device's `host` distinguishes local from remote.
- A remote dispatch, cancel, and confirm each reach the host, through `Cutters::Remote`.
- Controls render from `actions`; no test may reach for a phase to decide legality.
- The UI mirror and `e2e/smoke.spec.ts`'s fake move with `DeviceInfo` (#70).

`apps/desktop/MANUAL-CHECKLIST.md` gains: pair a real Pi; dispatch and close the laptop; reopen and
find the Job; watch two cutters in the list with one mid-Job; revoke one desktop's token and
confirm the other keeps working.

## Verification

```sh
cargo test --workspace --locked
npm --prefix apps/desktop/ui run build && npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run e2e
```

## Dependencies on open issues

- **#100 (serial identity) should land first.** `hosts.json` persists an `instance_id`, and until a
  serial cutter's id is stable across a reboot, saving one stores a reference that can silently
  come to mean a different machine.
- **#97 (no read timeout)** is why this phase polls rather than holding a connection open.
- **#70** is unavoidable here: `DeviceInfo` gains a field, and four hand-written mirrors move.
