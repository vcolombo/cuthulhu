<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Cut Host — running cutters from a Raspberry Pi — design

Date: 2026-08-08
Status: approved (brainstorming complete)

## Purpose

Cuthulhu can only cut from the computer the cutter is plugged into. `apps/desktop/src/device.rs:49-50`
holds one `Mutex<Option<Arc<DeviceManager>>>` and one `connected: Mutex<Option<DeviceInfo>>`, so the
desktop drives exactly one cutter for as long as the cut runs, and closing the laptop ends the cut.

Put a Raspberry Pi between the desktop and the cutters. It owns the USB and serial connections and runs
Jobs on them for remote clients, so the desktop is neither physically attached nor obliged to stay
awake.

Two requirements settled during brainstorming, both of which fix the shape of everything below:

- **A cut survives the desktop going away** — lid closed, Wi-Fi dropped, app quit. The Pi owns the cut,
  it does not relay bytes for a desktop that is still driving.
- **Two cutters on one Pi cut at the same time.** Not queued, not one-at-a-time.

## Relationship to open issues

| Issue | Relationship |
| --- | --- |
| #52 Secure local-network job handoff from companion clients | The opposite direction — a tablet sends a job *to* the desktop, which owns the cutter. Shares the security bar, and its "no cloud account or relay is required" criterion is honoured here. Stays open, untouched. |
| #59 Concurrent jobs across multiple connected cutters | Solved **on the Cut Host**, where there is no editor and no active-tab coupling to fight. Stays open for the desktop. |
| #42 Persistent cutter management and connection testing | `hosts.json` is a down payment on the persistent side of it. |
| #70 IPC types mirrored by hand in four places | `DeviceInfo` gains a field, so every mirror moves. |
| #72 Desktop shell needs a module extraction | `device.rs` grows here; the remote arm is a natural first split. |

The referenced [puccilabs SilhouetteServer guide](https://puccilabs.com/guides/SilhouetteServer/) is
not the architecture. It is `scp` plus `ssh pi ./g2g.sh`, with the whole gerber toolchain on the Pi
writing to a hardcoded `/dev/usb/lp0` — one cutter, one overwritten job file, no status returned. It is
useful as evidence that a Pi drives a Cameo over USB from Linux, and as nothing else.

## Naming

With the Pi owning the cut it is not a proxy: it forwards nothing. `CONTEXT.md` gains an entry, since
it is the normative vocabulary and has no word for this yet:

> **Cut Host** — a machine that owns Transports to one or more cutters and runs Jobs on them on behalf
> of remote clients. A Cut Host owns the cut: a client may detach mid-Job and the Job continues.
>
> _Avoid_: proxy, server, relay, bridge — all four name something that forwards, which this does not.

The daemon binary is `cuthulhu-cutd`.

## Scope

In scope: one new crate holding the protocol, the host and the client; a daemon binary; the desktop
changes needed to pair with a host, list its cutters and dispatch to them; `CONTEXT.md`; a Pi install
document.

Out of scope, decided deliberately:

- **Queueing.** Dispatch to a busy device is refused `Busy`. The requirement was two cutters running at
  once, and a spool is a different feature with its own persistence and ordering questions.
- **Desktop-side #59.** The desktop gets a read-only status list covering every cutter on a host, not N
  local `DeviceManager`s. See *Decisions* for why concurrency does not require it.
- **Design-file transfer.** The desktop plans; the host receives Passes. `cutplan::plan_cut` stays the
  one chokepoint, and this is not a headless Cuthulhu.
- **mDNS browse.** Raspberry Pi OS ships avahi, so `cuthulhu-pi.local` resolves without a line of
  discovery code. A browse UI can come later if typing a hostname turns out to hurt.
- **Cloud relay, WAN access, a Pi disk image.** LAN only; a documented systemd unit, not an installer.
- **`cuthulhu cut --host`.** Cheap to add later because the client is a library crate, but the CLI is
  not what makes anyone tethered.

## Decisions

### The wire protocol is the IPC surface that already exists

`apps/desktop/src/device.rs` already exposes `list_devices`, `caps_for`, `status`, `cut`, `cancel`,
`resume` and `confirm_pass_done` plus a `DeviceEvent` stream, and `CutStatus`, `DeviceEvent`,
`DeviceEventKind`, `DeviceInfo` and `MachineCaps` already derive `Serialize` for Tauri. Adding
`Deserialize` turns that surface into a protocol. Nothing new is designed, and the desktop's remote and
local paths cannot drift into two vocabularies for the same cut.

### The Cut Host runs the unmodified `DeviceManager`

Session framing, ENQ polling, the cancel atomic, the per-Pass completion policy and the `pub(crate)`
state machine are reused as they stand. This is the decision that keeps the feature small, and it is
also the one that protects `driver-core`'s central rule: `DeviceState` is private precisely so callers
stop re-deriving "what is legal now", and a Cut Host that re-implemented the worker would be the fourth
caller to do it.

### Concurrency needs almost no code, and none of it on the desktop

`DeviceManager::spawn` already gives each cutter its own worker thread, its own cancel flag and its own
published `CutStatus`. A `DeviceSlot` is therefore `{ info, manager, events }` and failure isolation —
#59's hardest acceptance criterion — is structural rather than implemented.

The desktop needs no equivalent. Because the host owns the cut, a desktop dispatches to cutter A,
detaches, dispatches to cutter B, and both cut while the desktop's single `connected` slot only ever
*watches* one. What the desktop gains is a read-only status list, not a second device registry.

### Preflight runs twice, deliberately

The desktop refuses bad geometry early, where the operator is. The host refuses it again because the
host is the only party that knows what is actually plugged into that port. The desktop planned against
a `MachineProfile` it *believed* was there; that belief is exactly what a network hop makes stale.

### Device connect is the host's business, not a client verb

The host calls `DeviceManager::connect` for each enumerated cutter at startup and holds it, which also
runs the existing identity probe (`manager.rs:381`, `PROBE_TIMEOUT`) against real hardware before any
client can aim at it. So `connect` and `disconnect` never cross the wire. Two clients cannot race over
one cutter's connection state, and a client that dies mid-cut cannot orphan a transport.

The protocol verbs are `ListDevices`, `Snapshot`, `Dispatch`, `Cancel`, `Resume`, `ConfirmPassDone`.

### One client connection covers every cutter on a host

Wire events wrap the existing type with a device id: `Event { device: InstanceId, event: DeviceEvent }`.
No per-device subscribe machinery exists, a multi-cutter view falls out for free, and the desktop's
`connected` slot degrades to "which cutter the cut dialog is aimed at" rather than "which cutter is
reachable".

### `dispatch_id` is client-supplied and deduplicated

The failure it prevents is specific and expensive: the link drops between a client sending `Dispatch`
and receiving the reply, the client retries, and the material is cut twice. A dedupe table keyed on a
client-generated id is a few lines against a failure that ruins work.

### One token, one trust level

Any authorized client may cancel, resume or confirm any Job, not merely the one it dispatched. This is
required rather than lazy. Whoever walks to the cutter to swap material for a Puma's
`needs_operator_pass_confirm` pause (`crates/driver-hpgl/src/encode.rs:19`) is not necessarily sitting
at the laptop that started the Job.

### Sync, thread-per-connection

`DeviceManager::cut()` blocks until the first pause point and the whole device layer is threads and
channels. An async runtime would wrap a blocking API, buy nothing, and cost a large dependency plus an
impedance mismatch at every call. Client count is single digits.

### Rejected alternatives

**Byte-level transport proxy — the Pi as a `Transport` over TCP.** Far smaller: a `TransportKind`
variant, a `RemoteTransport`, and a daemon that pipes bytes. Rejected because it fails the requirement.
The desktop would still be driving — planning, encoding, framing and ENQ-polling across the network for
the whole cut — so a closed lid or a dropped link kills the Job, which is the thing being fixed.

**Off-the-shelf USB/IP or VirtualHere.** No Cuthulhu code at all. Rejected because the desktop is macOS,
where a usable USB/IP client does not exist, serial cutters would need a second mechanism anyway, and
the result would still be a desktop-driven cut with the same lid-closed failure.

**Ride on SSH instead of implementing transport security.** Genuinely attractive — it is what the
puccilabs guide does, and it reuses keys the user already has for administering the Pi. Rejected
because the desktop app would have to embed or shell out to an SSH client to make pairing a GUI step,
which is not obviously less work than pinning a self-signed certificate, and it puts the trust
configuration outside the application entirely.

**Per-Job ownership so only the dispatching client can cancel.** Rejected as actively wrong; see *One
token, one trust level*.

## Architecture

```
Desktop                                     Raspberry Pi
───────                                     ────────────
Document → plan_passes → plan_cut           cuthulhu-cutd
  → CutPlan → cut_passes()                    ├─ preflight, against the driver actually attached
  → Vec<CutPass>                              ├─ DeviceManager ── USB ────► Cameo 5
       │                                      └─ DeviceManager ── serial ─► Puma IV
       ├── TLS ──► Dispatch { dispatch_id, device, machine_id, passes }
       └── TLS ◄── Event { device, DeviceEvent }
                   (the client may detach; the cut continues)
```

Both sides run the same `driver-core` and `driver-registry`. The Cut Host is `DeviceManagerHandle`'s
shape with a socket where the Tauri bridge is, and N devices where there is one.

## Components

One new crate, `crates/cut-host`, holding both sides. The desktop already depends on `driver-registry`,
so the daemon code adds no dependency weight it does not already carry, and splitting the crate in two
to spare the desktop a few unused symbols is work with no payer. No feature gates.

| File | Purpose |
| --- | --- |
| `crates/cut-host/src/protocol.rs` | `Request` / `Response` / `Event`; length-prefixed (`u32`) JSON frames |
| `crates/cut-host/src/client.rs` | `HostClient` — one TLS connection, a reader thread producing `DeviceEvent`s |
| `crates/cut-host/src/host.rs` | `HashMap<InstanceId, DeviceSlot>`, one `DeviceManager` per cutter, event fan-out |
| `crates/cut-host/src/bin/cuthulhu-cutd.rs` | Daemon: config, certificate, bind, thread-per-client |

The one piece concurrency does require: each `DeviceManager` hands over a single
`mpsc::Receiver<DeviceEvent>`, so a pump thread per device fans it out to whichever clients are
attached.

New dependencies: `rustls`, and `rcgen` for the first-run self-signed certificate. `serde_json` is
already present in the workspace.

```rust
// ponytail: JSON frames. Polylines as text floats are bulky — a large cut is
// megabytes. Swap to postcard if dispatch latency ever becomes noticeable.
```

### Desktop

| File | Change |
| --- | --- |
| `crates/driver-core/src/lib.rs` | `Deserialize` on the wire types; `DeviceInfo` gains `host: Option<HostId>` |
| `crates/driver-core/src/{status.rs,manager.rs}` | `Deserialize` on `CutStatus`, `DeviceEvent`, `DeviceEventKind`, `DeviceError` |
| `apps/desktop/src/device.rs` | `enum Cutters { Local { factory, manager }, Remote(HostClient) }` behind the existing mutex; each method becomes a two-arm match |
| `apps/desktop/src/{ipc.rs,state.rs}` | Host add, remove and pair commands |
| `apps/desktop/ui/src/ipc.ts` and its mirrors | The `DeviceInfo` field, per #70; the `e2e/smoke.spec.ts` fake moves with it |
| `CONTEXT.md` | The Cut Host entry |

`DeviceInfo.host` is `None` for a cutter attached to this computer. The alternative — namespacing
`instance_id` strings as `host:name:usb:1:4` — would hide the distinction inside a format nothing
parses.

If `device.rs` (441 lines today) outgrows comfort under the two-arm matches, the remote arm splits into
its own module, which is where #72 points anyway.

Hosts persist at `<config_dir>/cuthulhu/hosts.json`, mode `0600`: address, pinned certificate
fingerprint, token. It sits beside the existing `presets.json` and follows the same contract style.

### Raspberry Pi

`/etc/cuthulhu/cutd.toml` holds the token and bind address. The certificate and key are generated on
first run into `/var/lib/cuthulhu/`. The host is reached as `cuthulhu-pi.local`. Install is a
documented systemd unit in `docs/cut-host.md`.

## Data flow

**Pairing.** The user pastes `cuthulhu-pi.local:PORT` and the token from `cutd.toml`. The desktop shows
the certificate fingerprint, the user accepts once, and both land in `hosts.json`. A changed
fingerprint thereafter is a hard refusal, not a prompt.

**Dispatch.**

```
desktop:  plan_cut → preflight → CutPlan → cut_passes()
       → Dispatch { dispatch_id, device, machine_id, passes }

host:  dedupe on dispatch_id
    → refuse if the device's machine_id differs from the dispatch's
    → preflight again, against the driver actually attached
    → DeviceManager::cut(passes) → job_id
    → DeviceEvents stream to every attached client
```

**Detach and reattach.** A vanished client drops a subscriber and nothing else. On reattach the host
sends `Vec<DeviceSnapshot { info, status, job_id }>` before resuming the stream. `CutStatus` already
carries phase, `ended`, `actions`, Pass position and byte progress, which is precisely why a client
absent for an entire cut renders correctly from a single message. `job_id` rides alongside because
`CutStatus` alone cannot distinguish this client's finished Job from another's.

**Shutdown.** `cuthulhu-cutd` refuses to exit while any device reports `CutStatus::is_active()` — the
same predicate the desktop's window-close guard uses in `apps/desktop/src/main.rs` — except under an
explicit force flag. A host restart mid-cut kills the cut; nothing can prevent that, so the daemon
should at least not do it casually.

## Safety

Transport security is rustls with a first-run self-signed certificate, its fingerprint pinned at
pairing, and a bearer token compared in constant time. The daemon refuses to start bound to a
non-private address without an explicit override, so the LAN-only default is enforced by the daemon
rather than hoped for from a firewall.

Input is bounded, because the Pi has a gigabyte of RAM: a maximum frame size (default 32 MiB,
configurable) is rejected at the framing layer before allocation, frames deserialize into owned types
and validate completely before any device is touched, concurrent connections are capped, and repeated
authentication failures are backed off.

A dispatch can be refused four ways, in this order: **authentication**, **frame**, **machine
mismatch**, **preflight**.

Refusal text keeps one home. The wire carries the typed error, never a rendered sentence, and the
desktop renders it with the `Display` impl added in PR #94. A second copy of refusal prose living on
the Pi is exactly the drift PR #90 removed.

```rust
// ponytail: one token for the whole host. Named per-client tokens if a client
// ever needs revoking without re-pairing the rest.
```

Revocation today is rotate the token and restart.

**What this feature cannot make safe.** Untethering means a blade can start moving in a room with
nobody in it. That is the requested behaviour rather than a defect, and the design does not pretend
otherwise: the daemon logs every dispatch with its source, and cancel stays reachable from any attached
client, since `abort_bytes` and the cooperative cancel atomic already work and only needed a route over
the wire.

## Error handling

Every existing `DeviceError` variant crosses the wire unchanged and is rendered by the desktop exactly
as a local one would be. The failures new to this design are:

| Failure | Behaviour |
| --- | --- |
| Bad token, unknown or changed fingerprint | Connection refused before any frame is read; no device touched |
| Oversized or malformed frame | Connection closed; no device touched |
| `machine_id` mismatch | Dispatch refused, typed error returned, no motion |
| Host-side preflight failure | Dispatch refused, the existing `PlanError` returned and rendered by the desktop's `Display` |
| Repeated `dispatch_id` | Returns the original `job_id`; runs nothing further |
| Client disconnects mid-Job | Subscriber dropped; the Job continues; state recovered on reattach |
| Host restart mid-Job | The Job dies. The daemon refuses a non-forced exit while any device is active |

## Testing

Everything here is testable headless, because `MockTransport` (`crates/driver-core/src/lib.rs:100`)
already exists. A test spins a Cut Host in-process over a `DeviceBackendFactory` that hands out
`MockTransport`s, connects a `HostClient` over loopback TLS, and drives a complete cut. CI is Linux, so
the daemon compiles and its tests run natively there; only the aarch64 link goes unverified by CI.

New tests:

1. Round-trip every `Request`, `Response` and `Event`.
2. A repeated `dispatch_id` runs one Job, not two.
3. A `machine_id` mismatch refuses, and the typed error survives the hop.
4. A host-side preflight failure refuses, and its `PlanError` survives the hop.
5. An oversized frame is rejected before allocation.
6. A bad token and a changed fingerprint are both rejected.
7. A client drops mid-cut, reconnects, and the snapshot reports the correct phase, Pass and `job_id`.
8. **Two mock devices, two Jobs, one failed — the other's Job is untouched.** This is #59's isolation
   criterion as an actual test rather than a hardware hope, and it is the reason the mocks are worth
   the setup.

`apps/desktop/MANUAL-CHECKLIST.md` gains, each with the device and date it was verified on:

- Dispatch a multi-Pass cut, close the laptop, confirm the cut finishes.
- Reopen, reattach, confirm the finished Job reads as `Idle` with `ended: Completed`.
- Two cutters cutting simultaneously; fail one, the other continues.
- A cancel issued from a *second* client.
- A Puma colour swap confirmed from a device other than the one that dispatched.
- A refused connection after the Pi's certificate is regenerated.

## Verification

```sh
cargo test -p cut-host --locked
cargo test --workspace --locked
cross build --target aarch64-unknown-linux-gnu -p cut-host --bin cuthulhu-cutd

npm --prefix apps/desktop/ui run build     # dist/ is committed; CI fails on a stale bundle
npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run e2e
```

`Cargo.lock` is committed in the same change as the new dependencies, since CI runs `--locked`.

## Phasing

The implementation plan should split this three ways, each independently verifiable:

1. `protocol`, `host` and `client`, headless against `MockTransport`. No desktop changes.
2. Desktop wiring: pairing, `hosts.json`, the `Cutters` enum, the UI.
3. Multi-device concurrency and the hardware checklist.
