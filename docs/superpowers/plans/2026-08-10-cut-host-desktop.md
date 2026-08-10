<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Cut Host phase 2, part B: the desktop reaches a Pi — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The desktop pairs with a Cut Host, lists its cutters beside the local ones, and dispatches to either — with no user who never pairs a Pi seeing any change at all.

**Architecture:** `DeviceInfo` gains `host: Option<HostId>`, where `None` means "plugged into this computer". `DeviceManagerHandle` holds the local factory and every paired host together, and routes a single call by the addressed device's `host` field. Paired hosts persist in `hosts.json` beside the existing `presets.json`.

**Tech Stack:** Rust workspace, `cargo test --workspace --locked`. One new path dependency (`apps/desktop` on `cut-host`); no third-party crate. **This plan is Rust only** — it stops at the IPC boundary.

Spec: `docs/superpowers/specs/2026-08-09-cut-host-desktop-design.md`.

## Why this is Rust-only, and what part C does

Phase 2's spec covers two subsystems with a clean seam at the Tauri IPC boundary:

- **This plan (part B):** `driver-core`, `crates/cut-host`'s client, and `apps/desktop/src`. Every deliverable is testable headless against the `cut-host` loopback fixture — no Pi, no browser.
- **Part C, a separate plan:** `apps/desktop/ui` — `ipc.ts` and the four hand-written mirrors it feeds (`cut/viewmodel.ts`, `cut/viewmodel.test.ts`, `cut/CutDialog.tsx`, `e2e/smoke.spec.ts`), the pairing dialog, and the device list's host rows and phase badges.

Part C cannot start until this lands, because it consumes the IPC commands defined here. This plan can land and be reviewed on its own: it produces a desktop that can pair, list and dispatch, driven by tests, with no UI to click yet.

**A note on the TypeScript mirrors.** `DeviceInfo` gains a field in Task 1. TypeScript is structurally typed, so a mirror that omits `host` keeps compiling and the extra field is ignored at runtime — nothing breaks. Do not update the mirrors here; that is part C's job, and issue #70 tracks the underlying problem that they are hand-written at all.

## Prerequisites, all landed

- **#100 / PR #101** — a cutter's id is now `usb:sn:…` / `serial:sn:…`, derived from what the hardware says about itself. `hosts.json` persists cutter references, so an id that changed meaning across a reboot would have poisoned saved state.
- **PR #104** — `read_frame` bounds every frame that is owed, and `cutd.toml` has per-client tokens. Without the first, polling a host on a timer would freeze the app behind a hung Pi; without the second, every desktop would share one key.

Two gaps those left open, both tracked and neither blocking: **#102** (the write half is still unbounded) and **#103** (a silent client holds a slot forever).

## Global Constraints

- **SPDX header on every new file**: `// SPDX-License-Identifier: GPL-3.0-or-later`.
- **`cargo test --workspace --locked`** is what CI runs; `--locked` is mandatory. This plan **does** add one dependency — `apps/desktop` gains `cut-host = { path = "../../crates/cut-host" }`, which it does not have today — so `Cargo.lock` is committed in the same commit as that change (Task 3). No third-party crate is added; everything else is already in the workspace.
- **`CONTEXT.md` is normative vocabulary.** Use **Pass**, **Job**, **Driver**, **Transport**, **Preflight**, **Cut Host**. Never "proxy", "server", "relay" or "bridge" for the Cut Host.
- **A caller is told about a cut through one value.** Render from `CutStatus::actions`; never reconstruct what is legal from a phase. `DeviceState` is `pub(crate)` in `driver-core` and must stay unreachable.
- **Comments explain why, not what.** Where a step carries a comment, that comment is part of the deliverable. Do not add restating ones.
- **`// ponytail:` marks a deliberate simplification** with its ceiling and upgrade path.
- **Commit subjects are imperative with the reason attached.** Keep the repo's `Co-Authored-By:` trailer. Prose carries no process narration: no "as requested", no "per the plan", no agent names.
- **Do not touch `apps/desktop/ui/`.** `git diff --stat -- apps/desktop/ui/` must be empty for this plan.
- The workspace builds warning-free. Verify with `cargo clean -p driver-core -p cut-host -p desktop` then a rebuild before each commit.

---

### Task 1: `HostId`, and a `DeviceInfo` that says where it lives

**Files:**
- Modify: `crates/driver-core/src/lib.rs` — add `HostId`, add `DeviceInfo.host`
- Modify: `crates/driver-registry/src/lib.rs` — the two enumerators and `device_at_port` set `host: None`
- Modify: `crates/cut-host/src/host.rs` — the `testing` fixture's `DeviceInfo` literals
- Modify: `apps/desktop/src/device.rs` — its test fixture's `DeviceInfo` literal

**Interfaces:**
- Consumes: `DeviceInfo`, `TransportKind` as they stand.
- Produces:
  - `pub struct HostId(pub String)` deriving `Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize`
  - `DeviceInfo.host: Option<HostId>` — `None` means "attached to this computer"

**Why the field and not a namespaced id.** The alternative is encoding the host into `instance_id` as `host:name:usb:sn:…`. That hides the distinction inside a string nothing parses, and it would collide with #100's work, which made `instance_id` mean "what the hardware says about itself" — a value the desktop must be able to compare across hosts.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/driver-core/src/lib.rs`:

```rust
    /// `None` is not a mode — it is the absence of a host, which is what "plugged into this
    /// computer" is. A user who never pairs a Pi has every device in this state and sees no
    /// difference from before.
    #[test]
    fn a_device_with_no_host_is_local() {
        let local = DeviceInfo {
            instance_id: "usb:sn:CAMEO-A".into(),
            machine_id: "cameo5".into(),
            transport: TransportKind::Usb { locator: "sn:CAMEO-A".into() },
            candidate: false,
            host: None,
        };
        assert!(local.host.is_none());
    }

    /// The same cutter reached through two different hosts is two different devices, even though
    /// #100 makes its `instance_id` identical — the id says which machine, the host says which
    /// computer owns it.
    #[test]
    fn the_same_cutter_on_two_hosts_is_two_devices() {
        let on_a = DeviceInfo {
            instance_id: "usb:sn:CAMEO-A".into(),
            machine_id: "cameo5".into(),
            transport: TransportKind::Usb { locator: "sn:CAMEO-A".into() },
            candidate: false,
            host: Some(HostId("host-1".into())),
        };
        let on_b = DeviceInfo { host: Some(HostId("host-2".into())), ..on_a.clone() };
        assert_ne!(on_a, on_b);
        assert_eq!(on_a.instance_id, on_b.instance_id, "the cutter's own id is unchanged");
    }

    #[test]
    fn a_host_id_round_trips_through_serde() {
        let id = HostId("host-1".into());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<HostId>(&json).unwrap(), id);
    }
```

`serde_json` is not currently a dependency of `driver-core`. Rather than add one, drop the third test and assert the round trip inside `cut-host`'s protocol tests, which already have `serde_json`. Say in your report which you did.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p driver-core --locked host`

Expected: FAIL to compile — `struct DeviceInfo has no field named host`, and `cannot find type HostId`.

- [ ] **Step 3: Add the type and the field**

In `crates/driver-core/src/lib.rs`, beside `DeviceInfo`:

```rust
/// Which Cut Host a device is attached to. Opaque, and minted at pairing rather than derived
/// from the host's address, its display name, or its certificate — all three change in ordinary
/// use (a static lease, a rename, a regenerated certificate), and an id built on any of them
/// would make every saved cutter reference point at nothing the first time one did.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HostId(pub String);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub instance_id: String,
    pub machine_id: String,
    pub transport: TransportKind,
    pub candidate: bool,
    /// The Cut Host this device is attached to, or `None` for one plugged into this computer.
    /// Local is the absence of a host, not a mode.
    pub host: Option<HostId>,
}
```

- [ ] **Step 4: Give every existing constructor `host: None`**

`grep -rn "DeviceInfo {" --include=*.rs crates apps` finds them all. Every one is local hardware or a fixture standing in for it, so every one takes `host: None`:

- `crates/driver-registry/src/lib.rs` — `cameo5_devices`, `puma_devices`, `device_at_port`
- `crates/cut-host/src/host.rs` — the two literals in `mod testing`
- `apps/desktop/src/device.rs` — the `test_instance` fixture
- any others the grep turns up

Add the comment once, in `driver-registry`, where the real hardware is enumerated:

```rust
            // Enumerated here, so it is on this computer. A Cut Host's cutters get their id
            // stamped on by whoever fetched them, because the daemon does not know its own.
            host: None,
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. Note this changes a serialized shape the UI reads — `DeviceInfo` gains a `"host": null` field. TypeScript ignores unknown fields, so `apps/desktop/ui` keeps working untouched; do not edit it.

- [ ] **Step 6: Commit**

```bash
git add crates/driver-core/src/lib.rs crates/driver-registry/src/lib.rs \
        crates/cut-host/src/host.rs apps/desktop/src/device.rs
git commit -m "Let a device say which computer owns it, since a cutter's own id cannot"
```

---

### Task 2: `hosts.json`, which has to be boring and correct

**Files:**
- Create: `apps/desktop/src/hosts.rs`
- Modify: `apps/desktop/src/lib.rs` — add `pub mod hosts;`

**Interfaces:**
- Consumes: `driver_core::HostId`.
- Produces:
  - `pub struct PairedHost { pub id: HostId, pub name: String, pub address: String, pub fingerprint: String, pub token: String }`
  - `pub enum HostsError { Unreadable(String), Malformed(String), Unwritable(String) }` with `Display`
  - `pub fn load(path: &Path) -> Result<Vec<PairedHost>, HostsError>` — a missing file is an empty list, not an error
  - `pub fn save(path: &Path, hosts: &[PairedHost]) -> Result<(), HostsError>` — atomic, mode `0600`
  - `pub fn default_hosts_path() -> Option<PathBuf>`
  - `pub fn next_id(existing: &[PairedHost]) -> HostId`

**Why this task exists at all.** Bambu Studio's most-reported LAN complaint is that it forgets printers between sessions — one operator re-adds 34 machines on every launch. Persistence being boring and correct is the feature.

**Copy the pattern that already works.** `crates/cutplan/src/presets.rs` writes user presets atomically: a `NamedTempFile` in the destination directory, then `persist`. Read it (`grep -n "fn save_user_presets" -A 30 crates/cutplan/src/presets.rs`) and follow its shape, including the comment explaining why the temp file must be in the same directory.

- [ ] **Step 1: Write the failing test**

Create `apps/desktop/src/hosts.rs` with only the test module:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests {
    use super::*;

    fn a_host(id: &str, name: &str) -> PairedHost {
        PairedHost {
            id: HostId(id.into()),
            name: name.into(),
            address: "cuthulhu-pi.local:7878".into(),
            fingerprint: "aa:bb:cc".into(),
            token: "s3cret".into(),
        }
    }

    #[test]
    fn hosts_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        let written = vec![a_host("host-1", "Workshop Pi"), a_host("host-2", "Spare")];

        save(&path, &written).unwrap();
        assert_eq!(load(&path).unwrap(), written);
    }

    /// A user who has never paired anything is not an error state.
    #[test]
    fn a_missing_file_is_no_hosts_rather_than_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&dir.path().join("nothing-here.json")).unwrap(), Vec::new());
    }

    /// The file holds every client's token, so it must not be world-readable. Checked because a
    /// mode set at creation is easy to lose to a later rewrite.
    #[cfg(unix)]
    #[test]
    fn the_file_is_not_readable_by_anyone_else() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        save(&path, &[a_host("host-1", "Workshop Pi")]).unwrap();
        save(&path, &[a_host("host-1", "Renamed")]).unwrap(); // a rewrite must not widen it

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "hosts.json holds tokens: {mode:o}");
    }

    /// A corrupt file must not be silently read as "no hosts paired" — that would look exactly
    /// like a fresh install and invite the user to re-pair over the top of it.
    #[test]
    fn a_corrupt_file_is_an_error_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(load(&path), Err(HostsError::Malformed(_))));
    }

    #[test]
    fn a_new_id_never_collides_with_one_already_paired() {
        assert_eq!(next_id(&[]), HostId("host-1".into()));
        assert_eq!(next_id(&[a_host("host-1", "a")]), HostId("host-2".into()));
        // A gap left by a forgotten host must not be reused: a stale reference to `host-2`
        // would silently start meaning a different Pi.
        assert_eq!(next_id(&[a_host("host-1", "a"), a_host("host-3", "c")]), HostId("host-4".into()));
    }

    /// An id nobody minted must not derail minting the next one.
    #[test]
    fn an_unrecognised_id_shape_does_not_stop_a_new_one() {
        assert_eq!(next_id(&[a_host("imported-by-hand", "a")]), HostId("host-1".into()));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p desktop --locked hosts`

Expected: FAIL to compile — `cannot find type PairedHost`, `cannot find function save`.

- [ ] **Step 3: Write the implementation**

Add to the top of `apps/desktop/src/hosts.rs`, above the test module:

```rust
//! The Cut Hosts this desktop has paired with.
//!
//! Holds a token per host, so the file is `0600` and written atomically. Losing it means
//! re-pairing every Pi by hand, which is the single most-reported complaint about the
//! comparable feature in Bambu Studio — persistence being dull and correct is the point.

use std::path::{Path, PathBuf};

use driver_core::HostId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedHost {
    pub id: HostId,
    /// What the operator calls it. Free to change without invalidating anything, which is why
    /// the id is not derived from it.
    pub name: String,
    /// `host:port`. Also free to change — a Pi can get a new lease.
    pub address: String,
    /// The certificate fingerprint accepted at pairing. A change is a refusal, never a prompt.
    pub fingerprint: String,
    pub token: String,
}

#[derive(Debug)]
pub enum HostsError {
    Unreadable(String),
    Malformed(String),
    Unwritable(String),
}

impl std::fmt::Display for HostsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostsError::Unreadable(m) => write!(f, "the paired hosts could not be read ({m})"),
            HostsError::Malformed(m) => write!(f, "the paired hosts file could not be understood ({m})"),
            HostsError::Unwritable(m) => write!(f, "the paired hosts could not be saved ({m})"),
        }
    }
}
impl std::error::Error for HostsError {}

/// Every paired host, or an empty list when none has been.
///
/// A missing file is an empty list; a corrupt one is an error. The distinction matters: read as
/// empty, a corrupt file looks exactly like a fresh install, and the operator would pair over the
/// top of hosts they still have.
pub fn load(path: &Path) -> Result<Vec<PairedHost>, HostsError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(HostsError::Unreadable(e.to_string())),
    };
    serde_json::from_str(&text).map_err(|e| HostsError::Malformed(e.to_string()))
}

/// Write atomically, so an interrupted save leaves the previous list intact rather than a
/// truncated one. The temp file goes in the destination directory because a rename across
/// filesystems is not atomic.
pub fn save(path: &Path, hosts: &[PairedHost]) -> Result<(), HostsError> {
    let dir = path.parent().ok_or_else(|| HostsError::Unwritable("no parent directory".into()))?;
    std::fs::create_dir_all(dir).map_err(|e| HostsError::Unwritable(e.to_string()))?;

    let json = serde_json::to_string_pretty(hosts).map_err(|e| HostsError::Unwritable(e.to_string()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| HostsError::Unwritable(e.to_string()))?;
    std::io::Write::write_all(&mut tmp, json.as_bytes())
        .map_err(|e| HostsError::Unwritable(e.to_string()))?;

    // Set on the temp file before the rename, so the tokens are never briefly world-readable
    // at the destination — and re-applied on every save, since a rewrite would otherwise
    // inherit whatever the temp file's default mode happened to be.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o600))
            .map_err(|e| HostsError::Unwritable(e.to_string()))?;
    }

    tmp.persist(path).map_err(|e| HostsError::Unwritable(e.to_string()))?;
    Ok(())
}

/// Beside `presets.json`, which is where this application already keeps user state.
pub fn default_hosts_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("cuthulhu").join("hosts.json"))
}

/// The next unused id.
///
/// Counts past the highest ever used rather than filling gaps: a saved reference to a host the
/// operator forgot must not quietly start meaning a different Pi.
///
// ponytail: ids are `host-N` because a handful of hosts on one desktop cannot exhaust them and
// they read well in a log. If hosts ever sync between machines this needs to be random instead,
// since two desktops would both mint `host-1`.
pub fn next_id(existing: &[PairedHost]) -> HostId {
    let highest = existing
        .iter()
        .filter_map(|h| h.id.0.strip_prefix("host-"))
        .filter_map(|n| n.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    HostId(format!("host-{}", highest + 1))
}
```

Add to `apps/desktop/src/lib.rs`:

```rust
pub mod hosts;
```

- [ ] **Step 4: Check the dependencies are already there**

`apps/desktop/Cargo.toml` needs `serde_json`, `dirs` and `tempfile`. Run:

```sh
grep -nE "^(serde_json|dirs|tempfile)" apps/desktop/Cargo.toml
```

`serde_json` is already a dependency. If `dirs` or `tempfile` is missing, add it — `dirs = "6"` and `tempfile = "3"`, matching the versions `crates/cutplan/Cargo.toml` already pins — and **commit `Cargo.lock` in the same commit**, since CI runs `--locked`. Note in your report whether the lock changed; the plan's global constraint assumes it does not, and this is the one task that might.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p desktop --locked hosts` then `cargo test --workspace --locked`

Expected: PASS, six new tests.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/hosts.rs apps/desktop/src/lib.rs apps/desktop/Cargo.toml Cargo.lock
git commit -m "Remember the Cut Hosts a desktop has paired with, atomically and unreadable to others"
```

---

### Task 3: A client that proves a host before it is saved, and labels what it fetches

**Files:**
- Modify: `crates/cut-host/src/client.rs` — add `HostClient::pair_check`
- Modify: `apps/desktop/src/hosts.rs` — add `stamp_host`

**Interfaces:**
- Consumes: `HostClient::connect`, `HostClient::devices` from phase 1; `HostId`, `PairedHost` from Tasks 1-2.
- Produces:
  - `pub fn HostClient::pair_check(addr: &str, token: &str, fingerprint: &str) -> Result<Vec<DeviceInfo>, ClientError>` — connect, list, drop
  - `pub fn stamp_host(id: &HostId, devices: Vec<DeviceInfo>) -> Vec<DeviceInfo>`

**Why `stamp_host` exists, and why it is easy to forget.** A Cut Host does not know its own `HostId` — the desktop mints it, and the daemon has never heard of it. So every `DeviceInfo` a host returns arrives with `host: None`, which is the value that means "plugged into this computer". Left unstamped, every remote cutter would be routed to the local `DeviceManager`. Whoever fetches must stamp.

**Why `pair_check` and not just `connect`.** Pairing must prove the host before anything is written, or the operator ends up with a saved entry that has never worked — which is how Bambu's users end up re-adding printers. `connect` alone proves the fingerprint and the token; listing devices proves the daemon is actually serving.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `apps/desktop/src/hosts.rs`:

```rust
    use driver_core::{DeviceInfo, TransportKind};

    fn a_device(instance: &str) -> DeviceInfo {
        DeviceInfo {
            instance_id: instance.into(),
            machine_id: "cameo5".into(),
            transport: TransportKind::Usb { locator: "sn:CAMEO-A".into() },
            candidate: false,
            host: None,
        }
    }

    /// A Cut Host does not know its own id, so everything it returns says `host: None` — which
    /// is the value that means "plugged into this computer". Unstamped, every remote cutter
    /// would be routed to the local DeviceManager.
    #[test]
    fn fetched_devices_are_stamped_with_the_host_they_came_from() {
        let id = HostId("host-1".into());
        let stamped = stamp_host(&id, vec![a_device("usb:sn:A"), a_device("serial:sn:B")]);

        assert_eq!(stamped.len(), 2);
        assert!(stamped.iter().all(|d| d.host.as_ref() == Some(&id)), "{stamped:?}");
        assert_eq!(stamped[0].instance_id, "usb:sn:A", "the cutter's own id is untouched");
    }

    #[test]
    fn stamping_nothing_yields_nothing() {
        assert!(stamp_host(&HostId("host-1".into()), Vec::new()).is_empty());
    }
```

And add an integration test at `crates/cut-host/tests/end_to_end.rs`:

```rust
#[test]
fn a_pair_check_lists_the_cutters_and_leaves_no_connection_behind() {
    let host = start_test_host();
    let devices = HostClient::pair_check(&host.addr, TOKEN, &host.fingerprint).unwrap();
    assert_eq!(devices.len(), 2, "the test host has two cutters");
    assert!(devices.iter().all(|d| d.host.is_none()), "a daemon does not know its own id");
}

#[test]
fn a_pair_check_with_the_wrong_token_fails_before_anything_is_saved() {
    let host = start_test_host();
    assert!(HostClient::pair_check(&host.addr, "not-the-token", &host.fingerprint).is_err());
}

#[test]
fn a_pair_check_against_a_different_certificate_is_refused() {
    let host = start_test_host();
    let wrong = "00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
                 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff";
    match HostClient::pair_check(&host.addr, TOKEN, wrong) {
        Err(ClientError::Fingerprint { .. }) => {}
        Err(e) => panic!("expected a fingerprint refusal, got {e:?}"),
        Ok(_) => panic!("a certificate that was not the pinned one was accepted"),
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p desktop --locked stamp` then `cargo test -p cut-host --locked --test end_to_end pair_check`

Expected: FAIL to compile — `cannot find function stamp_host`, and `no function or associated item named pair_check`.

- [ ] **Step 3: Write both**

In `crates/cut-host/src/client.rs`, beside `connect`:

```rust
    /// Prove a host before anything about it is written down: connect, list its cutters, and
    /// drop the connection.
    ///
    /// Pairing that saves first and discovers later is how an operator ends up with an entry
    /// that has never worked. `connect` alone proves the fingerprint and the token; listing
    /// proves the daemon is actually serving.
    pub fn pair_check(
        addr: &str,
        token: &str,
        pinned_fingerprint: &str,
    ) -> Result<Vec<DeviceInfo>, ClientError> {
        HostClient::connect(addr, token, pinned_fingerprint)?.devices()
    }
```

In `apps/desktop/src/hosts.rs`:

```rust
/// Mark every device as belonging to `id`.
///
/// A Cut Host does not know its own `HostId` — the desktop mints it at pairing and the daemon
/// has never heard of it — so everything a host returns arrives saying `host: None`, which is
/// the value that means "plugged into this computer". Whoever fetches must stamp, or every
/// remote cutter routes to the local Transport.
pub fn stamp_host(id: &HostId, devices: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
    devices
        .into_iter()
        .map(|d| DeviceInfo { host: Some(id.clone()), ..d })
        .collect()
}
```

Add `use driver_core::{DeviceInfo, HostId};` to `hosts.rs`'s imports.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. The three end-to-end tests run against a real Cut Host on a loopback port, so they also prove `pair_check` closes its connection — the fixture's `MAX_CLIENTS` would otherwise be exhausted by repeated runs.

- [ ] **Step 5: Commit**

```bash
git add crates/cut-host/src/client.rs apps/desktop/src/hosts.rs crates/cut-host/tests/end_to_end.rs
git commit -m "Prove a Cut Host before saving it, and stamp what it returns with the id it does not know"
```

---

### Task 4: The desktop holds every paired host at once

**Files:**
- Modify: `apps/desktop/src/device.rs` — `Cutters`, `HostConnection`, `list_devices`

**Interfaces:**
- Consumes: `PairedHost`, `stamp_host` from Tasks 2-3; `HostClient` from phase 1.
- Produces:
  - `pub(crate) struct HostConnection { pub paired: PairedHost, pub client: Option<HostClient>, pub last_error: Option<String> }`
  - `DeviceManagerHandle` gains `local_factory`, `local_manager` and `hosts: Mutex<HashMap<HostId, HostConnection>>`
  - `DeviceManagerHandle::list_devices(&self) -> Vec<DeviceInfo>` — now merges local and remote
  - `DeviceManagerHandle::add_host(&self, paired: PairedHost)` and `remove_host(&self, id: &HostId)`

**Note on the spec's `Cutters`.** The spec describes this as a `Cutters { local, hosts }` struct. Realise it as fields on the existing `DeviceManagerHandle` instead: a wrapper whose only user is that one handle is an indirection with nothing on the other side of it, and the handle already owns the mutexes the fields need. The property the spec cared about — both kinds held at once, rather than either/or — is what matters, and it holds either way.

**The correction this task encodes.** Phase 1's design sketched `enum Cutters { Local, Remote(HostClient) }` behind one mutex, as though the desktop talked to one thing at a time. That cannot express what this phase does: `list_devices` asks *every* paired host, and the device list shows them together. The enum became a struct holding both, and routing moved to the call site.

**A host that is down stays listed.** Issue #42 asks that unavailable cutters remain visible with repair guidance rather than vanishing. `HostConnection::last_error` is where that lives — a host whose connection failed keeps its `PairedHost` and reports why.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `apps/desktop/src/device.rs`:

```rust
    use crate::hosts::PairedHost;
    use driver_core::HostId;

    fn a_paired_host(id: &str, addr: &str) -> PairedHost {
        PairedHost {
            id: HostId(id.into()),
            name: "Workshop Pi".into(),
            address: addr.into(),
            fingerprint: "aa:bb:cc".into(),
            token: "s3cret".into(),
        }
    }

    /// A user who never pairs a Pi must see exactly what they see today. This is the test that
    /// says the feature is optional by construction rather than by intention.
    #[test]
    fn with_no_host_paired_the_device_list_is_the_local_one() {
        let dev = test_device_setup();
        let listed = dev.list_devices();
        assert!(listed.iter().all(|d| d.host.is_none()), "{listed:?}");
    }

    /// A host that cannot be reached keeps its place in the list rather than vanishing — a
    /// cutter that disappears looks like one that was never paired.
    #[test]
    fn an_unreachable_host_is_still_listed_with_its_reason() {
        let dev = test_device_setup();
        // Nothing is listening on this port, so connecting fails.
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

        let reasons = dev.host_errors();
        assert_eq!(reasons.len(), 1, "the host stays known: {reasons:?}");
        assert!(reasons[0].1.is_some(), "and says why it is unreachable");
    }

    #[test]
    fn forgetting_a_host_removes_it_and_its_cutters() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        assert_eq!(dev.host_errors().len(), 1);

        dev.remove_host(&HostId("host-1".into()));
        assert!(dev.host_errors().is_empty());
        assert!(dev.list_devices().iter().all(|d| d.host.is_none()));
    }
```

**`host_errors` is a test observation point.** Add it as `pub(crate) fn host_errors(&self) -> Vec<(HostId, Option<String>)>` — Task 6 turns it into an IPC command, so it is not test-only scaffolding.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p desktop --locked host`

Expected: FAIL to compile — `no method named add_host`.

- [ ] **Step 3: Restructure the handle**

In `apps/desktop/src/device.rs`, replace the `DeviceManagerHandle` fields. The existing `factory` and `manager` become the `local_*` pair, and a map of hosts joins them:

```rust
/// One paired Cut Host: what was saved about it, its connection if it has one, and why it has
/// none if it does not.
///
/// A host that is down keeps its entry so its cutters can be listed as unreachable rather than
/// disappearing — a cutter that vanishes looks like one that was never paired (#42).
pub(crate) struct HostConnection {
    pub paired: PairedHost,
    pub client: Option<HostClient>,
    pub last_error: Option<String>,
}

impl HostConnection {
    /// Connect if not already connected, and remember the reason if that fails.
    fn ensure(&mut self) -> Option<&HostClient> {
        if self.client.is_none() {
            match HostClient::connect(&self.paired.address, &self.paired.token, &self.paired.fingerprint) {
                Ok(c) => {
                    self.client = Some(c);
                    self.last_error = None;
                }
                Err(e) => self.last_error = Some(e.to_string()),
            }
        }
        self.client.as_ref()
    }
}
```

Then hold both kinds at once, rather than either/or:

```rust
pub struct DeviceManagerHandle {
    local_factory: Arc<dyn DeviceBackendFactory>,
    // ponytail: brief said `Arc<DeviceManager>`; `DeviceManager::shutdown(self)` consumes by
    // value, so the Arc is wrapped in Option to let `shutdown()` take it out and unwrap it.
    local_manager: Mutex<Option<Arc<DeviceManager>>>,
    /// Every paired Cut Host, connected lazily. Held together rather than one-at-a-time
    /// because `list_devices` asks all of them and the device list shows them together.
    hosts: Mutex<HashMap<HostId, HostConnection>>,
    pub connected: Mutex<Option<DeviceInfo>>,
}
```

Rename the existing `factory`/`manager` uses to `local_factory`/`local_manager` throughout the file — `grep -n "self.factory\|self.manager" apps/desktop/src/device.rs` finds them.

- [ ] **Step 4: Merge the device list**

Replace `list_devices`:

```rust
    /// Local hardware plus every paired Cut Host's cutters, in one list.
    ///
    /// A host that cannot be reached contributes nothing here and its reason shows up in
    /// `host_errors` — the list is what can be cut on, not what has been configured.
    pub fn list_devices(&self) -> Vec<DeviceInfo> {
        let mut all = self.local_factory.list_devices();
        let mut hosts = self.hosts.lock().unwrap();
        for (id, host) in hosts.iter_mut() {
            let Some(client) = host.ensure() else { continue };
            match client.devices() {
                Ok(devices) => all.extend(crate::hosts::stamp_host(id, devices)),
                Err(e) => {
                    // The connection went away between `ensure` and here; drop it so the next
                    // call reconnects rather than reusing a dead one.
                    host.last_error = Some(e.to_string());
                    host.client = None;
                }
            }
        }
        all
    }

    pub fn add_host(&self, paired: PairedHost) {
        let id = paired.id.clone();
        self.hosts
            .lock()
            .unwrap()
            .insert(id, HostConnection { paired, client: None, last_error: None });
    }

    pub fn remove_host(&self, id: &HostId) {
        self.hosts.lock().unwrap().remove(id);
    }

    /// Every paired host and why it is unreachable, or `None` if it is not.
    pub(crate) fn host_errors(&self) -> Vec<(HostId, Option<String>)> {
        self.hosts
            .lock()
            .unwrap()
            .iter()
            .map(|(id, h)| (id.clone(), h.last_error.clone()))
            .collect()
    }
```

Note `ensure` takes `&mut self`, so `hosts` is locked mutably for the whole loop. That is acceptable here — the lock is not held across a *cut*, only across connect-and-list, and no other call path contends for it during a dispatch. If a later task finds itself wanting this lock while a cut is running, that is the signal to split it.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS. `an_unreachable_host_is_still_listed_with_its_reason` connects to `127.0.0.1:1`, which fails fast — if it hangs, `HostClient::connect` is missing the socket timeout PR #104 added, and that is a real finding rather than a test to loosen.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/device.rs
git commit -m "Hold every paired Cut Host at once, since listing cutters has to ask them all"
```

---

### Task 5: Route a call by the device it names

**Files:**
- Modify: `apps/desktop/src/device.rs` — `connect`, `cancel`, `resume`, `confirm_pass_done`, `execute_cut`

**Interfaces:**
- Consumes: `Cutters` from Task 4.
- Produces: no new public signatures. Each existing method routes on the addressed device's `host`.

**The rule.** `None` routes to the local `DeviceManager`; `Some(id)` routes to that host's `HostClient`. The desktop's `connected: Mutex<Option<DeviceInfo>>` says which cutter the dialog is aimed at, and its `host` field is what decides where each call goes.

**What does not change.** The desktop still watches one cutter at a time. Dispatching to a second host's cutter means aiming at it — and because a Cut Host owns its cut, the first one keeps cutting. That is why issue #59 stays open for the desktop and is not needed here.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `apps/desktop/src/device.rs`:

```rust
    /// A cutter with no host is this computer's, and must reach the local DeviceManager — the
    /// path every existing user is on.
    #[test]
    fn a_local_device_still_routes_to_the_local_manager() {
        let dev = test_device_setup();
        assert_eq!(dev.status().phase, driver_core::Phase::Idle);
        assert!(dev.cancel().is_ok(), "a local cancel reaches the local manager");
    }

    /// Naming a host that was forgotten (or never paired) must be refused rather than falling
    /// back to the local cutter — a Job aimed at a Pi must never be cut on the desk.
    #[test]
    fn a_device_naming_an_unknown_host_is_refused_not_run_locally() {
        let dev = test_device_setup();
        let elsewhere = DeviceInfo {
            host: Some(HostId("host-does-not-exist".into())),
            ..test_instance()
        };
        let err = dev.connect(elsewhere).unwrap_err();
        assert_eq!(err.code, "unknown_host", "got {err:?}");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p desktop --locked routes`

Expected: FAIL — `connect` accepts the device and reaches the local manager, so no `unknown_host` error is produced.

- [ ] **Step 3: Add the router**

In `apps/desktop/src/device.rs`:

```rust
    /// Where a call about `device` has to go. `None` is this computer; `Some(id)` is that host.
    ///
    /// An id nobody has paired is refused rather than falling back to local hardware: a Job
    /// aimed at a Pi must never be cut on the machine sitting on the desk.
    fn route(&self, device: &DeviceInfo) -> Result<Route, IpcError> {
        match &device.host {
            None => Ok(Route::Local),
            Some(id) if self.hosts.lock().unwrap().contains_key(id) => Ok(Route::Host(id.clone())),
            Some(id) => Err(IpcError::new("unknown_host", format!("no Cut Host called `{}` is paired", id.0))),
        }
    }

    /// Run `f` against the client for `id`, connecting if needed.
    fn with_host<T>(
        &self,
        id: &HostId,
        f: impl FnOnce(&HostClient) -> Result<T, cut_host::client::ClientError>,
    ) -> Result<T, IpcError> {
        let mut hosts = self.hosts.lock().unwrap();
        let host = hosts
            .get_mut(id)
            .ok_or_else(|| IpcError::new("unknown_host", format!("no Cut Host called `{}` is paired", id.0)))?;
        let client = host
            .ensure()
            .ok_or_else(|| IpcError::new("host_unreachable", host.last_error.clone().unwrap_or_default()))?;
        f(client).map_err(|e| IpcError::new("host_error", e.to_string()))
    }
```

with

```rust
enum Route {
    Local,
    Host(HostId),
}
```

Then give each verb its two arms. `connect` records what is aimed at without touching a remote host — a Cut Host connects its own cutters at startup, so there is nothing for the desktop to connect:

```rust
    pub fn connect(&self, info: DeviceInfo) -> Result<(), IpcError> {
        match self.route(&info)? {
            Route::Local => {
                self.local_manager()?
                    .connect(info.clone())
                    .map_err(|e| IpcError::new("device_error", format!("{e:?}")))?;
            }
            // A Cut Host connects each cutter itself at startup, so aiming at one is a local
            // bookkeeping act: there is no remote connection to open.
            Route::Host(_) => {}
        }
        *self.connected.lock().unwrap() = Some(info);
        Ok(())
    }
```

`cancel`, `resume` and `confirm_pass_done` each read `connected` and route:

```rust
    pub fn cancel(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            None | Some(Route::Local) => {
                self.local_manager()?.cancel();
                Ok(())
            }
            Some(Route::Host(id)) => {
                let device = aimed.expect("a route implies a device").instance_id;
                self.with_host(&id, |c| c.cancel(&device))
            }
        }
    }
```

Write `resume` and `confirm_pass_done` the same way, calling `c.resume(&device)` and `c.confirm_pass_done(&device)`.

`execute_cut` routes its dispatch. A remote dispatch needs a `DispatchId`; mint one per call:

```rust
            Some(Route::Host(id)) => {
                // `execute_cut` takes only the Passes, so both the device and the machine it is
                // for come from what the dialog is aimed at — which is also what `route` just
                // resolved, so the two cannot disagree.
                let aimed = aimed.expect("a route implies a device");
                let (device, machine_id) = (aimed.instance_id, aimed.machine_id);
                // A fresh id per attempt: this is a new Job, not a retry of a dropped reply,
                // and reusing one would make the host treat it as already accepted.
                let dispatch_id = cut_host::protocol::DispatchId(format!(
                    "{}-{}",
                    device,
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0)
                ));
                self.with_host(&id, |c| c.dispatch(dispatch_id, &device, &machine_id, passes))?;
                Ok(0)
            }
```

`execute_cut` returns a `job_id` locally. A remote dispatch has none yet — the host assigns it and the client learns it from the event stream — so it returns `0`, and the caller reads real progress from `status`. Add:

```rust
// ponytail: a remote dispatch reports job id 0, because `Response::Accepted` carries none —
// `DeviceManager::cut` does not return one until the Job reaches a pause point. Nothing reads
// this value for a remote cut today; give it the real id when the desktop shows per-Job history.
```

- [ ] **Step 4: Route `status` too**

`status` currently reads the local manager. For a remote cutter it must read that host's snapshot for the aimed-at device:

```rust
    pub fn status(&self) -> CutStatus {
        let aimed = self.connected.lock().unwrap().clone();
        let Some(device) = aimed else { return CutStatus::disconnected() };
        match self.route(&device) {
            Ok(Route::Local) | Err(_) => match self.local_manager.lock().unwrap().as_ref() {
                Some(mgr) => mgr.status(),
                None => CutStatus::disconnected(),
            },
            Ok(Route::Host(id)) => self
                .with_host(&id, |c| c.snapshots())
                .ok()
                .and_then(|snaps| {
                    snaps.into_iter().find(|s| s.info.instance_id == device.instance_id).map(|s| s.status)
                })
                // A host that cannot be reached mid-cut is not a finished cut: the Job is still
                // running on the Pi, and saying `Idle` here would invite a second dispatch.
                .unwrap_or_else(|| CutStatus::disconnected()),
        }
    }
```

`status` is called by the window-close guard and must never block for long — PR #104's read deadline is what makes that true over a network.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --workspace --locked`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/device.rs
git commit -m "Send each call where its device lives, and refuse one naming a host nobody paired"
```

---

### Task 6: The IPC commands the UI will need

**Files:**
- Modify: `apps/desktop/src/ipc.rs` — four new commands
- Modify: `apps/desktop/src/main.rs` — register them, and load `hosts.json` at startup

**Interfaces:**
- Consumes: everything above.
- Produces, all `#[tauri::command]`:
  - `list_hosts(dev) -> Result<Vec<PairedHostView>, IpcError>`
  - `test_host(address: String, token: String, fingerprint: String) -> Result<Vec<DeviceInfo>, IpcError>`
  - `pair_host(dev, name: String, address: String, token: String, fingerprint: String) -> Result<PairedHostView, IpcError>`
  - `forget_host(dev, id: HostId) -> Result<(), IpcError>`
  - `pub struct PairedHostView { pub id: HostId, pub name: String, pub address: String, pub unreachable: Option<String> }`

**Why a view type rather than `PairedHost`.** `PairedHost` holds the token. Sending it to the webview would put every host's secret into the frontend, where a stray `console.log` or a devtools session exposes it — the same reasoning that made `Config`'s `Debug` redact in PR #104. `PairedHostView` carries what the UI needs to render a row and nothing more.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `apps/desktop/src/device.rs` (the handle is what these commands delegate to):

```rust
    /// The token must not leave the Rust side. A view type is the guard, and this is what stops
    /// a later refactor from "simplifying" it back to sending `PairedHost`.
    #[test]
    fn a_host_view_carries_no_token() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

        let views = dev.host_views();
        assert_eq!(views.len(), 1);
        let json = serde_json::to_string(&views[0]).unwrap();
        assert!(!json.contains("s3cret"), "a token reached the view: {json}");
        assert!(json.contains("host-1"), "the id is what the UI addresses: {json}");
    }

    #[test]
    fn pairing_mints_an_id_that_does_not_collide() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        let next = crate::hosts::next_id(&dev.paired_hosts());
        assert_eq!(next, HostId("host-2".into()));
    }
```

Add the two accessors this needs alongside `host_errors`:

```rust
    pub(crate) fn host_views(&self) -> Vec<PairedHostView>
    pub(crate) fn paired_hosts(&self) -> Vec<PairedHost>
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p desktop --locked host_view`

Expected: FAIL to compile — `cannot find type PairedHostView`.

- [ ] **Step 3: Add the view and the accessors**

In `apps/desktop/src/device.rs`:

```rust
/// What the UI is told about a paired Cut Host.
///
/// Deliberately not `PairedHost`: that holds the token, and anything sent to the webview can
/// reach a `console.log` or a devtools session. The UI needs to render a row and address the
/// host by id; it does not need the secret.
#[derive(Clone, Debug, Serialize)]
pub struct PairedHostView {
    pub id: HostId,
    pub name: String,
    pub address: String,
    /// Why this host cannot be reached, or `None` when it can.
    pub unreachable: Option<String>,
}

impl DeviceManagerHandle {
    pub(crate) fn host_views(&self) -> Vec<PairedHostView> {
        self.hosts
            .lock()
            .unwrap()
            .values()
            .map(|h| PairedHostView {
                id: h.paired.id.clone(),
                name: h.paired.name.clone(),
                address: h.paired.address.clone(),
                unreachable: h.last_error.clone(),
            })
            .collect()
    }

    pub(crate) fn paired_hosts(&self) -> Vec<PairedHost> {
        self.hosts.lock().unwrap().values().map(|h| h.paired.clone()).collect()
    }
}
```

- [ ] **Step 4: Add the commands**

In `apps/desktop/src/ipc.rs`:

```rust
#[tauri::command]
pub fn list_hosts(dev: tauri::State<DeviceManagerHandle>) -> Result<Vec<PairedHostView>, IpcError> {
    Ok(dev.host_views())
}

/// Prove a host without saving it. The pairing dialog calls this before `pair_host`, so an
/// entry that has never worked is never written.
#[tauri::command]
pub fn test_host(address: String, token: String, fingerprint: String) -> Result<Vec<DeviceInfo>, IpcError> {
    cut_host::client::HostClient::pair_check(&address, &token, &fingerprint)
        .map_err(|e| IpcError::new("host_unreachable", e.to_string()))
}

#[tauri::command]
pub fn pair_host(
    dev: tauri::State<DeviceManagerHandle>,
    name: String,
    address: String,
    token: String,
    fingerprint: String,
) -> Result<PairedHostView, IpcError> {
    // Prove it before writing it down, so a saved host has always worked at least once.
    cut_host::client::HostClient::pair_check(&address, &token, &fingerprint)
        .map_err(|e| IpcError::new("host_unreachable", e.to_string()))?;

    let paired = crate::hosts::PairedHost {
        id: crate::hosts::next_id(&dev.paired_hosts()),
        name,
        address,
        fingerprint,
        token,
    };
    dev.add_host(paired.clone());
    save_hosts(&dev)?;
    Ok(PairedHostView { id: paired.id, name: paired.name, address: paired.address, unreachable: None })
}

#[tauri::command]
pub fn forget_host(dev: tauri::State<DeviceManagerHandle>, id: HostId) -> Result<(), IpcError> {
    dev.remove_host(&id);
    save_hosts(&dev)
}

fn save_hosts(dev: &DeviceManagerHandle) -> Result<(), IpcError> {
    let path = crate::hosts::default_hosts_path()
        .ok_or_else(|| IpcError::new("no_config_dir", "this system has no configuration directory"))?;
    crate::hosts::save(&path, &dev.paired_hosts()).map_err(|e| IpcError::new("hosts_unwritable", e.to_string()))
}
```

- [ ] **Step 5: Register them and load at startup**

In `apps/desktop/src/main.rs`, add to `tauri::generate_handler![...]`:

```rust
            ipc::list_hosts,
            ipc::test_host,
            ipc::pair_host,
            ipc::forget_host,
```

and load the saved hosts once, after `DeviceManagerHandle` is built:

```rust
    // A host that fails to load is not a reason to refuse to start — the desktop still cuts on
    // local hardware, and the operator can re-pair. Say so once rather than failing silently.
    match hosts::default_hosts_path().map(|p| hosts::load(&p)) {
        Some(Ok(paired)) => {
            for host in paired {
                device.add_host(host);
            }
        }
        Some(Err(e)) => eprintln!("cuthulhu: paired hosts could not be loaded: {e}"),
        None => {}
    }
```

- [ ] **Step 6: Run the whole suite**

Run: `cargo test --workspace --locked` then `cargo build -p desktop --locked`

Expected: PASS and a clean build. Confirm `git diff --stat -- apps/desktop/ui/` is empty.

- [ ] **Step 7: Commit**

```bash
git add apps/desktop/src/ipc.rs apps/desktop/src/main.rs apps/desktop/src/device.rs
git commit -m "Give the UI commands to pair, list and forget a Cut Host, without handing it a token"
```

---

## Done when

- `cargo test --workspace --locked` passes.
- `cargo build -p desktop --locked` succeeds and a clean rebuild is warning-free.
- `git diff --stat -- apps/desktop/ui/` is empty — this plan touches no TypeScript.
- `grep -rn "\.host" apps/desktop/src/device.rs` shows every verb routing on it, and none falling back to local when a host is named.
- A `PairedHostView` serialized to JSON contains no token, pinned by `a_host_view_carries_no_token`.

## What part C inherits

`list_hosts`, `test_host`, `pair_host` and `forget_host` as Tauri commands; `DeviceInfo.host` distinguishing local from remote in the existing `list_devices`; a `get_device_state` that already routes to the right host; and `PairedHostView` as the shape a host row renders from.

Part C adds the pairing dialog, the device list's host grouping and phase badges, the `ipc.ts` type updates, the four hand-written mirrors (#70), and the `e2e/smoke.spec.ts` fake.

**Part C also owns the polling.** The spec calls for refreshing a remote cut's progress at 1 Hz while it is being watched, and nothing here does that — this plan makes `status` answer correctly for a remote cutter, but a *timer* that calls it belongs with the dialog that stops polling when it closes. Building it here would mean a loop with no way to know whether anyone is looking.
