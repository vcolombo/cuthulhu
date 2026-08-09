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

`HostId` is an opaque string minted at pairing and never derived from the address, the display
name, or the certificate. All three change in ordinary use: a Pi gets a static DHCP lease, an
operator renames it, a certificate is regenerated. An id derived from any of them would make a
saved cutter reference point at nothing the first time one did. `hosts.json` maps the id to the
address, so moving a host is an edit rather than a re-pairing.

Together with #100's `usb:sn:…` / `serial:sn:…` ids, a saved reference is then stable in both
halves: which host, and which cutter on it.

This is the field that hits issue #70: the IPC types are mirrored by hand in four places, and the
e2e fake in `smoke.spec.ts` mirrors them again.

### The desktop holds every paired host at once, and routes per call

Phase 1's design sketched `enum Cutters { Local { .. }, Remote(HostClient) }` behind the existing
mutex, as though the desktop were connected to one thing at a time. That is wrong for what this
phase actually does: `list_devices` has to ask *every* configured host, and the status list shows
every host's cutters together. An either/or enum cannot express that.

So the desktop holds both, always:

```rust
struct Cutters {
    local: LocalCutters,                     // the factory and DeviceManager it has today
    hosts: HashMap<HostId, HostConnection>,  // one per paired host, lazily connected
}
```

and the enum becomes about *routing a single call*, derived from the `DeviceInfo.host` of the
device being addressed — `None` routes to `local`, `Some(id)` routes to that host. A user who
pairs nothing has an empty map and never executes the remote path.

`HostConnection` owns the `HostClient` plus its last known reachability, so a host that is down is
listed with its cutters marked unreachable rather than vanishing (#42).

### Connections are held open, which makes #97 a prerequisite

Polling every host once a second means either holding a TLS connection per host or completing a
handshake per poll. Handshaking at 1 Hz is waste, so the connections are held.

That has a consequence the phase 1 design's "holds no connection open" line got wrong: **issue #97
(no read timeout in `read_frame`) must be fixed before this phase, not after.** `HostClient::call`
holds its stream mutex across a blocking read with no deadline, so a Pi that hangs — or a Wi-Fi
drop that never sends a RST — freezes the poll, the mutex, and whatever UI thread is behind it.
That is tolerable when nothing consumes `HostClient`. It is a frozen application once the desktop
polls on a timer.

Polling still beats a pushed-event connection here, for the reason given above — but not because
it avoids holding a socket. It avoids a *second* socket and a demultiplexing reader.

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

`cutd.toml` moves from `token = "…"` to a `[tokens]` table keyed by a name the operator chooses
(`workshop-laptop`, `office-desktop`). The old scalar is **refused with a message naming the new
form**, rather than silently accepted as an unnamed token — a daemon that quietly kept working
would leave the operator believing they had per-client revocation when they had one shared key.
Nothing is deployed yet, so this costs a line in `docs/cut-host.md` and no migration.

The daemon logs which token name authenticated each connection, so revoking the right one does not
require guessing.

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
| `crates/cut-host/src/config.rs` | `[tokens]` table replaces the scalar; the old form is refused by name |
| `crates/cut-host/src/client.rs` | `HostClient::test()` — connect, list, disconnect, for pairing |
| `apps/desktop/src/device.rs` | `Cutters { local, hosts }`; `list_devices` merges; snapshot polling |
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
- A remote dispatch, cancel, and confirm each route to the right host by `DeviceInfo.host`.
- Two paired hosts are listed together, and one being unreachable does not hide the other's cutters.
- A `cutd.toml` carrying the old scalar `token` is refused with a message naming `[tokens]`.
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

Two must land before this phase starts. Both are small, and both become expensive to retrofit once
a desktop is storing references or polling on a timer.

- **#100 — stable device identity. Fixed in PR #101.** `hosts.json` persists a cutter reference,
  and until a cutter's id survives a reboot, saving one stores a reference that can silently come
  to mean a different machine. With #101 the id is `usb:sn:…` / `serial:sn:…`, and a device that
  can only be named by socket says so — which the pairing UI should surface, because a saved
  reference to an `at:` device is exactly the one that can go wrong.
- **#97 — no read timeout in `read_frame`. Prerequisite, not a follow-up.** See *Connections are
  held open* above. Polling on a timer over a held connection turns a hung Pi into a frozen
  application; today it costs only a stuck test.

And one that is unavoidable rather than blocking:

- **#70** — `DeviceInfo` gains a field, so four hand-written IPC mirrors and the e2e fake move
  with it. Worth considering whether this phase is the moment to generate them instead, since it
  is the second time in two phases that this type has changed.
