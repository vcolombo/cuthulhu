// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use cut_host::client::HostClient;
use cutplan::presets::{
    default_presets_path, load_presets, resolve_settings, save_user_presets, MaterialPreset,
    SettingsOverride,
};
use cutplan::{plan_cut, plan_passes, ColorPass, CutError, PassSelection, PlanOptions};
use driver_core::manager::{CutPass, DeviceEvent, DeviceManager};
use driver_core::{CutStatus, DeviceBackendFactory, DeviceInfo, HostId, MachineCaps};
use serde::{Deserialize, Serialize};

use crate::hosts::PairedHost;
use crate::state::AppState;

#[derive(Clone, Debug, Serialize)]
pub struct IpcError { pub code: String, pub message: String }

impl IpcError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        IpcError { code: code.into(), message: message.into() }
    }
}

/// A client failure as a code the UI can branch on, keeping the three that need three different
/// things from the operator apart.
///
/// Collapsed to one code, a rejected token, a changed certificate and a Pi that is merely asleep
/// arrived as the same `host_unreachable` with prose that could not be told apart — so the two
/// that need somebody to *do* something looked exactly like the one that fixes itself (#112).
/// The message stays the client's own, unaltered (#94).
pub(crate) fn host_error(e: &cut_host::client::ClientError) -> IpcError {
    use cut_host::client::ClientError;
    let code = match e {
        // A hard refusal with its own guidance, not a reachability problem: this host is up and is
        // not the one that was paired.
        ClientError::Fingerprint { .. } => "host_fingerprint",
        // The host answered and said no. Re-pairing is the fix; waiting is not.
        ClientError::Unauthorized => "host_unauthorized",
        // The host was reached, understood the request, and refused it.
        ClientError::Refused(_) => "host_refused",
        ClientError::Transport(_) => "host_unreachable",
    };
    IpcError::new(code, e.to_string())
}

#[derive(Deserialize)]
pub struct CutRequest {
    pub device_instance_id: String,
    pub doc_revision: String,
    pub passes: Vec<ConfiguredPassDto>,
}

#[derive(Deserialize)]
pub struct ConfiguredPassDto {
    pub color: Option<u32>,
    pub enabled: bool,
    pub preset_id: Option<String>,
    pub speed: Option<u32>,
    pub force: Option<u32>,
    pub repeat_count: Option<u32>,
}

/// One paired Cut Host: what was saved about it, its connection if it has one, and why it has
/// none if it does not.
///
/// A host that is down keeps its entry so its cutters can be listed as unreachable rather than
/// disappearing — a cutter that vanishes looks like one that was never paired (#42).
pub(crate) struct HostConnection {
    pub paired: PairedHost,
    pub client: Option<HostClient>,
    /// Why this host has no connection, kept as the whole error rather than its prose: the code is
    /// what tells "the token was refused" from "the Pi is asleep", and flattening it here was one
    /// of the two places that distinction used to be thrown away (#112).
    pub last_error: Option<IpcError>,
}

impl HostConnection {
    /// Connect if not already connected, and remember the reason if that fails.
    fn ensure(&mut self) -> Option<&HostClient> {
        self.ensure_within(cut_host::client::CONNECT_TIMEOUT)
    }

    /// Same as `ensure`, but the reconnect attempt is capped at `timeout` rather than always
    /// spending the full `CONNECT_TIMEOUT` — a caller with a short total budget for the whole
    /// call (a status poll behind a lock that must never block for long) must not have that
    /// budget eaten by a reconnect it did not choose the length of.
    fn ensure_within(&mut self, timeout: std::time::Duration) -> Option<&HostClient> {
        if self.client.is_none() {
            match HostClient::connect_within(&self.paired.address, &self.paired.token, &self.paired.fingerprint, timeout) {
                Ok(c) => {
                    self.client = Some(c);
                    self.last_error = None;
                }
                Err(e) => self.last_error = Some(host_error(&e)),
            }
        }
        self.client.as_ref()
    }
}

enum Route {
    Local,
    Host(HostId),
}

/// What a press of Cut did.
///
/// `duplicate` is the Cut Host saying it had already accepted this dispatch id and started
/// nothing — a fact only it holds, and one the operator standing at a cutter that is not moving
/// needs (#121). Always `false` for a local cut, which has no dedupe to be caught by.
#[derive(Clone, Debug, Serialize)]
pub struct CutStarted {
    pub job_id: u64,
    pub duplicate: bool,
}

/// A Cut Host already paired at the address a new pairing names.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingPairing {
    pub id: HostId,
    pub name: String,
    /// Whether the certificate this pairing just probed is the one already pinned. `false` is the
    /// interesting case: the host's identity changed, which is either a reinstall or something
    /// worth worrying about, and only the operator knows which.
    pub same_fingerprint: bool,
}

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

/// The budget handed to *each* of the two legs `with_host_within` can spend on a status poll —
/// reconnect, then body read — not the total. Spent twice in sequence the real cap is **2x this
/// value** (4s); the DNS/mDNS resolve is bounded inside the reconnect leg's own deadline.
///
/// Short because this is the window-close guard's own read and a quit mid-cut must not look hung.
/// It is no longer what keeps the *rest* of the app moving: the poll holds only its own host's
/// connection lock (see `hosts`), so overrunning this budget queues calls aimed at that one host
/// and leaves `cancel`, the device list and every other host alone.
const STATUS_POLL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Separate Tauri managed state from `AppStateHandle` — device commands go
/// through here and never touch the document mutex.
pub struct DeviceManagerHandle {
    local_factory: Arc<dyn DeviceBackendFactory>,
    // ponytail: brief said `Arc<DeviceManager>`; `DeviceManager::shutdown(self)` consumes by
    // value, so the Arc is wrapped in Option to let `shutdown()` take it out and unwrap it.
    local_manager: Mutex<Option<Arc<DeviceManager>>>,
    /// Every paired Cut Host, connected lazily, each behind a lock of its own.
    ///
    /// Two locks rather than one because reaching a host is a network call. Under a single
    /// map-wide lock a wedged Pi held it for the length of whatever timeout that call had, and
    /// every other host verb — `cancel` included, the one that stops a moving blade — queued
    /// behind it; each such call got its own timeout in turn, and `to_socket_addrs` is one that
    /// cannot be given one at all. Per-connection locks confine the wait to calls aimed at the
    /// host actually doing the waiting, which makes an unbounded one survivable instead of
    /// something to keep patching.
    ///
    /// **Lock order, and the only one: map, then connection — never the reverse.** Nothing may
    /// take `hosts` while holding a `HostConnection`. Take the map lock only long enough to clone
    /// the `Arc`s out — `host_conn`/`host_conns` are that step — drop it, and lock the connection
    /// after; no network call may run with the map lock held.
    hosts: Mutex<HashMap<HostId, Arc<Mutex<HostConnection>>>>,
    /// Dispatches this desktop sent and never got an answer to, by the Job they carried.
    ///
    /// The host deduplicates on the dispatch id precisely so a retry after a dropped reply
    /// cannot cut the same material twice — but only the sender knows whether a given press of
    /// Cut *is* that retry, and nothing in the Job says so: a retry and a second sheet of the
    /// same design are byte-identical. What separates them is this entry. It is written before
    /// the request goes out and cleared by any answer, so a Job still listed here is one whose
    /// fate is genuinely unknown, and the next dispatch of it reuses the same id.
    ///
    /// An entry is cleared by an *answer*, and otherwise only once the host itself can no longer
    /// recognise the id. An earlier version aged them out after fifteen minutes, which was the
    /// wrong fix for a dispatch nobody revisits: inside the host's window, time cannot show that
    /// the operator loaded fresh material, so expiring early mints a new id for a Job the host
    /// still remembers under the old one and cuts the design twice. What makes an entry nobody
    /// revisits harmless is that the next Cut is *told* it was read as a retry
    /// (`CutStarted::duplicate`), and that the answer clears the entry, so pressing Cut again
    /// really does cut (#121).
    ///
    /// The one honest expiry is the host's own `ID_RETENTION`: past it the id means nothing to
    /// anyone, so holding it only pretends to a protection that has already lapsed. Nothing on
    /// either side can prevent a re-cut past that point — which is the true limit of deduplicating
    /// by memory, not something a client-side rule can close.
    ///
    /// **Lock order:** taken alone, or last — never with `hosts` or a `HostConnection` acquired
    /// while holding it.
    in_doubt: Mutex<HashMap<JobKey, (cut_host::protocol::DispatchId, std::time::Instant)>>,
    /// The last status each remote cutter actually reported, so something can be said about one
    /// without dialling it.
    last_remote_status: Mutex<HashMap<(HostId, String), CutStatus>>,
    /// Remote cutters this desktop has dispatched to and not since seen free.
    ///
    /// The cache above cannot answer for the window that matters most: a dispatch accepted a
    /// moment ago has not been polled yet, so the newest thing anyone heard is the `Idle` from
    /// before it — and the window-close guard would wave the operator past a cut it just started.
    /// This says "we started something and nothing has told us it ended" without dialling.
    remote_dispatched: Mutex<std::collections::HashSet<(HostId, String)>>,
    /// Bumped by every dispatch, so a poll can tell whether what it learned is still current.
    ///
    /// A poll reads the host's status without holding anything across the network call, so a
    /// dispatch can land while it is in flight — and the `Idle` it then wrote would clear the
    /// entry that dispatch just made. Comparing the count either side of the call is what makes
    /// the write conditional on nothing having happened meanwhile.
    dispatch_epoch: std::sync::atomic::AtomicU64,
    pub connected: Mutex<Option<DeviceInfo>>,
}

/// How many Jobs may be held in doubt at once.
///
/// A backstop against a session that dispatches thousands of distinct designs to hosts that never
/// answer, not a policy anyone should reach: entries clear on any answer, and expire with the
/// host's own memory. The oldest is evicted, which is the one a retry is least likely to name.
const MAX_JOBS_IN_DOUBT: usize = 256;

/// One dispatchable Job as this desktop identifies it: which cutter, on which host, carrying
/// what. Two presses of Cut that agree on all three are the same Job — which is what makes the
/// second one a candidate for being a retry of the first.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct JobKey {
    host: HostId,
    device: String,
    digest: u64,
}

/// Hashes the Job as it will go on the wire. Passes carry `f64` geometry, which has no `Hash`;
/// their serde form is the same bytes `dispatch` sends, so hashing that covers the geometry and
/// the resolved settings together, and cannot drift from what is actually dispatched.
///
/// ponytail: `DefaultHasher` is only guaranteed stable within one process run. That is exactly
/// the lifetime of `in_doubt`, which is the only thing comparing these — see `execute_cut` for
/// what a desktop restart costs.
fn job_digest(machine_id: &str, passes: &[CutPass]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    machine_id.hash(&mut h);
    // Infallible in practice: the writer is a `Vec`, and Preflight has already refused
    // non-finite coordinates by the time a Job reaches dispatch.
    serde_json::to_vec(passes).unwrap_or_default().hash(&mut h);
    h.finish()
}

impl DeviceManagerHandle {
    pub fn new(factory: Arc<dyn DeviceBackendFactory>) -> (Self, std::sync::mpsc::Receiver<DeviceEvent>) {
        let (mgr, events) = DeviceManager::spawn(factory.clone());
        let handle = DeviceManagerHandle {
            local_factory: factory,
            local_manager: Mutex::new(Some(Arc::new(mgr))),
            hosts: Mutex::new(HashMap::new()),
            in_doubt: Mutex::new(HashMap::new()),
            last_remote_status: Mutex::new(HashMap::new()),
            remote_dispatched: Mutex::new(std::collections::HashSet::new()),
            dispatch_epoch: std::sync::atomic::AtomicU64::new(0),
            connected: Mutex::new(None),
        };
        (handle, events)
    }

    fn manager(&self) -> Result<Arc<DeviceManager>, IpcError> {
        self.local_manager.lock().unwrap().clone()
            .ok_or_else(|| IpcError::new("shut_down", "device manager has been shut down"))
    }

    /// The connection for `id`, lifted out from under the map lock so the caller dials with only
    /// that one connection held. See `hosts` for why the ordering is the invariant.
    fn host_conn(&self, id: &HostId) -> Result<Arc<Mutex<HostConnection>>, IpcError> {
        self.hosts
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| IpcError::new("unknown_host", format!("no Cut Host called `{}` is paired", id.0)))
    }

    /// Every connection, lifted out the same way, for the callers that visit all of them.
    fn host_conns(&self) -> Vec<(HostId, Arc<Mutex<HostConnection>>)> {
        self.hosts.lock().unwrap().iter().map(|(id, c)| (id.clone(), c.clone())).collect()
    }

    /// Local hardware plus every paired Cut Host's cutters, in one list.
    ///
    /// A host that cannot be reached contributes nothing here and its reason shows up in
    /// `host_views` — the list is what can be cut on, not what has been configured.
    ///
    /// Deliberately still serial, so N unreachable hosts still cost N connect timeouts *for this
    /// call*. What they no longer cost is every other host verb: the map lock is gone before the
    /// first dial, so only calls aimed at the host being dialled wait behind it.
    pub fn list_devices(&self) -> Vec<DeviceInfo> {
        let mut all = self.local_factory.list_devices();
        for (id, conn) in self.host_conns() {
            let mut guard = conn.lock().unwrap();
            let host = &mut *guard;
            let Some(client) = host.ensure() else { continue };
            match client.devices() {
                Ok(devices) => all.extend(crate::hosts::stamp_host(&id, devices)),
                Err(e) => {
                    // The connection went away between `ensure` and here; drop it so the next
                    // call reconnects rather than reusing a dead one.
                    host.last_error = Some(host_error(&e));
                    host.client = None;
                }
            }
        }
        all
    }

    pub fn add_host(&self, paired: PairedHost) {
        let id = paired.id.clone();
        let conn = HostConnection { paired, client: None, last_error: None };
        self.hosts.lock().unwrap().insert(id, Arc::new(Mutex::new(conn)));
    }

    pub fn remove_host(&self, id: &HostId) {
        self.hosts.lock().unwrap().remove(id);
    }

    /// What `list_hosts` gives the UI: enough to render a row and address it, never the token.
    ///
    /// Reads each connection in turn, so it still waits on a host `list_devices` is dialling. It
    /// walks the same map in the same order, so against a sweep of unreachable hosts it trails one
    /// host behind and the total approaches the sweep's — the same wait as before, just no longer
    /// blocking anything else meanwhile. No safety verb is on this path, which is why that is
    /// tolerable here and would not be on `status` or `cancel`.
    pub(crate) fn host_views(&self) -> Vec<PairedHostView> {
        self.host_conns()
            .into_iter()
            .map(|(_, conn)| {
                let h = conn.lock().unwrap();
                PairedHostView {
                    id: h.paired.id.clone(),
                    name: h.paired.name.clone(),
                    address: h.paired.address.clone(),
                    unreachable: h.last_error.as_ref().map(|e| e.message.clone()),
                }
            })
            .collect()
    }

    /// Every paired host as saved, for `pair`/`forget` to re-derive `hosts.json`'s on-disk
    /// contents and to mint the next id against.
    pub(crate) fn paired_hosts(&self) -> Vec<PairedHost> {
        self.host_conns().into_iter().map(|(_, conn)| conn.lock().unwrap().paired.clone()).collect()
    }

    /// Whether `address` is already paired, and whether `fingerprint` is what was pinned for it.
    ///
    /// Asked by the pairing dialog, before the operator has typed anything they would have to type
    /// twice. `pair` itself cannot refuse a second entry for one address — a changed fingerprint
    /// is a hard refusal on every later connection, so re-pairing is the only recovery there is —
    /// which is exactly how one Pi ended up with two rows, identical but for which of them errors,
    /// and nothing anywhere saying why (#107).
    ///
    /// Matched on the address as typed. Two spellings of one Pi (a name and its IP) read as two
    /// hosts here, which is the direction that fails safe: a warning not shown is the state before
    /// this existed.
    pub(crate) fn existing_pairing(&self, address: &str, fingerprint: &str) -> Option<ExistingPairing> {
        self.paired_hosts().into_iter().find(|h| h.address == address).map(|h| ExistingPairing {
            id: h.id,
            name: h.name,
            same_fingerprint: h.fingerprint == fingerprint,
        })
    }

    /// Pairs a Cut Host: prove it works, mint an id, persist — and only once the save has
    /// actually landed, hold it in memory.
    ///
    /// `hosts::save` is written atomic for exactly this class of failure (full disk, read-only
    /// config dir); ordering the in-memory insert after it means a failed save leaves disk and
    /// memory agreeing (host absent from both) instead of a phantom host live in the device list
    /// while the operator was told pairing failed.
    pub fn pair(
        &self,
        name: String,
        address: String,
        token: String,
        fingerprint: String,
        hosts_path: &Path,
    ) -> Result<HostId, IpcError> {
        // Prove it before writing it down, so a saved host has always worked at least once.
        HostClient::pair_check(&address, &token, &fingerprint).map_err(|e| host_error(&e))?;

        // ponytail: reads paired_hosts() once, mints, saves, then add_hosts — three separate lock
        // acquisitions, so two concurrent pair() calls can mint the same id and one save can
        // overwrite the other's add_host. Fine for a single modal pairing dialog; if that stops
        // being true, hold `hosts` across mint-and-save or mint from a counter that isn't a re-read.
        let mut prospective = self.paired_hosts();
        let id = crate::hosts::next_id(&prospective);
        let paired = PairedHost { id: id.clone(), name, address, fingerprint, token };
        prospective.push(paired.clone());
        crate::hosts::save(hosts_path, &prospective)
            .map_err(|e| IpcError::new("hosts_unwritable", e.to_string()))?;

        self.add_host(paired);
        Ok(id)
    }

    /// Forgets a Cut Host: refuses unless it can confirm the host is idle, since the moment the
    /// host is gone `cancel` routes to `unknown_host` (see `route`) — a blade still moving could
    /// no longer be stopped. Persist-then-mutate for the same reason as `pair`.
    ///
    /// **Silence is not idleness.** This used to let every network error fall through to the
    /// deletion, on the reasoning that a Pi which cannot answer cannot be busy and that a Pi gone
    /// for good must not become unforgettable. The first half is false and is the whole point of
    /// the product: drop Wi-Fi mid-Job and the Pi keeps cutting. Forgetting there discards the
    /// token and the route needed to cancel, resume or confirm that Job once the network comes
    /// back — the one machine that could stop the blade throws away the key. The second half is
    /// still true, which is why the escape hatch is `force` rather than absent.
    ///
    /// The two refusals carry different codes because the operator's next move differs: cancel
    /// the cut, versus decide whether an unreachable Pi is really idle.
    ///
    /// `force` skips nothing but the *unconfirmed* case. A host that answers "busy" is a host
    /// this desktop can still reach, so `cancel` is available and the blade was never orphaned;
    /// there is no case for cutting that route while it works.
    pub fn forget(&self, id: &HostId, hosts_path: &Path, force: bool) -> Result<(), IpcError> {
        // Bounded the same way `status()` is (`STATUS_POLL_TIMEOUT`, not the full
        // `DEFAULT_BODY_TIMEOUT`) — on *both* legs, the reconnect and the body read. Not because
        // anything else waits on it: this holds only this host's connection lock, so a stall here
        // delays calls aimed at this host and nothing else. It is bounded because the operator is
        // usually forgetting a host *precisely* because it has stopped answering, and making them
        // wait 30s before they can even be offered the force is the wrong answer to that.
        match self.with_host_within(id, STATUS_POLL_TIMEOUT, |c| c.snapshots_within(STATUS_POLL_TIMEOUT)) {
            Ok(snapshots) if snapshots.iter().any(|s| s.status.is_active()) => {
                return Err(IpcError::new(
                    "host_busy",
                    "a cut is active on this host; cancel it before forgetting",
                ))
            }
            Ok(_) => {}
            Err(e) if !force => {
                return Err(IpcError::new(
                    "host_unconfirmed",
                    format!(
                        "this Cut Host could not be asked whether it is cutting ({}); if it is, \
                         forgetting it discards the only way to stop it",
                        e.message
                    ),
                ))
            }
            Err(_) => {}
        }

        let prospective: Vec<PairedHost> = self.paired_hosts().into_iter().filter(|h| &h.id != id).collect();
        crate::hosts::save(hosts_path, &prospective)
            .map_err(|e| IpcError::new("hosts_unwritable", e.to_string()))?;

        self.remove_host(id);
        // Otherwise `get_connected_device` keeps answering with a device `list_devices` no
        // longer lists: the aim would survive the host it named.
        let mut connected = self.connected.lock().unwrap();
        if connected.as_ref().and_then(|d| d.host.as_ref()) == Some(id) {
            *connected = None;
        }
        Ok(())
    }

    /// Capability is the Driver's answer, not ours — the Driver that encodes the
    /// bytes is also what declares what they can carry.
    /// `Result`, not `Option`: an id the registry cannot build means the caller
    /// is out of sync with it, which is worth surfacing rather than defaulting.
    pub fn caps_for(&self, machine_id: &str) -> Result<MachineCaps, IpcError> {
        self.local_factory
            .driver_for(machine_id)
            .map(|d| d.caps())
            .ok_or_else(|| IpcError::new("unknown_machine", format!("no driver for `{machine_id}`")))
    }

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

    /// Run `f` against the client for `id`, connecting if needed. Spends the full
    /// `CONNECT_TIMEOUT` on a reconnect: for a Job-carrying call (cancel, resume, confirm,
    /// dispatch) that is rightly owed the same budget a fresh pairing would get.
    fn with_host<T>(
        &self,
        id: &HostId,
        f: impl FnOnce(&HostClient) -> Result<T, cut_host::client::ClientError>,
    ) -> Result<T, IpcError> {
        self.with_host_within(id, cut_host::client::CONNECT_TIMEOUT, f)
    }

    /// Same as `with_host`, but `connect_timeout` bounds the reconnect leg too, not just the
    /// body read inside `f`. `with_host` alone only bounds what `f` does — a dropped connection
    /// still reconnects via the unbounded default, so a caller with a short total budget for the
    /// whole call (the window-close guard reading `status()`) could still block for the full
    /// `CONNECT_TIMEOUT` before `f` ever runs.
    ///
    /// The true wait is still not a hard total: `connect_timeout` twice over (reconnect, then
    /// body), plus the small overshoots the resolver's bounds allow — a grace beat while a
    /// second address family lands, the answer channel's margin. What makes that acceptable
    /// rather than the next thing to patch is *whose* wait it is: the map lock is released
    /// before the connection is locked, so all of it is spent by calls aimed at this host and
    /// by nothing else.
    fn with_host_within<T>(
        &self,
        id: &HostId,
        connect_timeout: std::time::Duration,
        f: impl FnOnce(&HostClient) -> Result<T, cut_host::client::ClientError>,
    ) -> Result<T, IpcError> {
        let conn = self.host_conn(id)?;
        let mut guard = conn.lock().unwrap();
        let host = &mut *guard;
        // Not `let client = host.ensure_within(...).ok_or_else(...)?;` — that binding would keep
        // `host`'s mutable borrow alive through the `None` arm's `host.last_error` read. Matching
        // in place lets the borrow end with the arm that doesn't need it.
        match host.ensure_within(connect_timeout) {
            Some(client) => {
                let out = f(client);
                if let Err(e @ cut_host::client::ClientError::Transport(_)) = &out {
                    // A connection that broke mid-call stays broken, and `ensure` only redials
                    // when there is no client at all — so leaving this one in place fails every
                    // later verb on this host against the same dead socket. `list_devices` has
                    // always dropped it here; the call that most needs the same is the retry
                    // after a lost reply, which is by definition made on a connection that just
                    // failed (see `execute_cut`).
                    host.last_error = Some(host_error(e));
                    host.client = None;
                }
                out.map_err(|e| host_error(&e))
            }
            None => Err(host
                .last_error
                .clone()
                .unwrap_or_else(|| IpcError::new("host_unreachable", "this Cut Host has not answered"))),
        }
    }

    /// Whether the local `DeviceManager` currently has a cut in flight, asked directly rather
    /// than through `status()` at the current aim — the whole point is to catch this *before*
    /// the aim moves away from local, which is what would make `status()` stop seeing it.
    fn local_cut_is_active(&self) -> bool {
        match self.local_manager.lock().unwrap().as_ref() {
            Some(mgr) => mgr.status().is_active(),
            None => false,
        }
    }

    pub fn connect(&self, info: DeviceInfo) -> Result<(), IpcError> {
        let route = self.route(&info)?;
        // A moving blade must not become unstoppable: the window-close guard and `cancel` both
        // read the *current* aim, so moving it to a host mid-cut would deafen the guard (it would
        // see the host's `Idle` instead) and mis-route `cancel` to a machine that was never
        // asked to stop. Mirrors `forget`'s busy refusal.
        if matches!(route, Route::Host(_)) && self.local_cut_is_active() {
            return Err(IpcError::new(
                "device_error",
                "a cut is active on the local cutter; cancel it before switching to another device",
            ));
        }
        match route {
            Route::Local => {
                self.manager()?
                    .connect(info.clone())
                    .map_err(|e| IpcError::new("device_error", format!("{e:?}")))?;
            }
            // A Cut Host connects each cutter itself at startup, so aiming at one is a local
            // bookkeeping act: there is no remote connection to open. But the local manager must
            // still be released — `DeviceManager::connect` refuses anything but
            // `Disconnected`/`Error`, so leaving it `Idle` after aiming away would strand the
            // operator on the Pi for the rest of the session, unable to aim back at their own
            // cutter. Safe unconditionally: the guard above already refused this arm if a local
            // cut is active, so this can only disconnect an idle (or already-disconnected)
            // manager — freeing the USB device for other software as a side effect.
            Route::Host(_) => {
                self.manager()?.disconnect().map_err(|e| IpcError::new("device_error", format!("{e:?}")))?;
            }
        }
        *self.connected.lock().unwrap() = Some(info);
        Ok(())
    }

    /// Refuse a verb that would drop the local cutter's transport while it is still working.
    ///
    /// `Host::reconnect` answers this same question for a remote cutter, and the webview hides
    /// the control (`connectedControl`) — but a guard that lives only in TypeScript, reading a
    /// status documented to lag the worker by one event, must not be the thing standing between a
    /// moving blade and a dropped transport. Mirrors `connect`'s own refusal.
    fn refuse_while_the_local_cutter_is_working(&self, before: &str) -> Result<(), IpcError> {
        if self.local_cut_is_active() {
            return Err(IpcError::new(
                "device_error",
                format!("a cut is active on the local cutter; cancel it before {before}"),
            ));
        }
        Ok(())
    }

    pub fn disconnect(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            // A remote cutter's connection belongs to the Cut Host, not this desktop — the
            // mirror of `connect`'s remote arm, which never opened one here to close. Routing
            // this unconditionally to the local manager used to close the local cutter's
            // transport and discard its parked job while aimed at a Pi, silently.
            None | Some(Route::Local) => {
                self.refuse_while_the_local_cutter_is_working("disconnecting it")?;
                self.manager()?.disconnect().map_err(|e| IpcError::new("device_error", format!("{e:?}")))?;
            }
            Some(Route::Host(_)) => {}
        }
        *self.connected.lock().unwrap() = None;
        Ok(())
    }

    /// Drop the aimed cutter's transport and open it again, re-running the identity probe
    /// against real hardware.
    ///
    /// The way back from a cancel whose stop nothing confirmed — `driver-core` refuses both a
    /// cut and a connect from that state. Locally that is the dialog's Disconnect followed by
    /// Connect, done in one call; on a Cut Host it has to be the host's own verb, since this
    /// desktop never opened that transport and `disconnect` there is bookkeeping only.
    pub fn reconnect(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        let Some(device) = aimed else {
            return Err(IpcError::new("device_error", "no device is connected"));
        };
        match self.route(&device)? {
            Route::Local => {
                self.refuse_while_the_local_cutter_is_working("reconnecting it")?;
                let mgr = self.manager()?;
                mgr.disconnect().map_err(|e| IpcError::new("device_error", format!("{e:?}")))?;
                mgr.connect(device).map_err(|e| IpcError::new("device_error", format!("{e:?}")))
            }
            Route::Host(id) => self.with_host(&id, |c| c.reconnect(&device.instance_id)),
        }
    }

    /// Where the cut has got to. Reads `driver-core`'s published status, which
    /// never blocks on the worker — so the window-close handler and the IPC
    /// command can both call it freely, even mid-transmit.
    pub fn status(&self) -> CutStatus {
        let aimed = self.connected.lock().unwrap().clone();
        let Some(device) = aimed else { return CutStatus::disconnected() };
        match self.route(&device) {
            Ok(Route::Local) => match self.local_manager.lock().unwrap().as_ref() {
                Some(mgr) => mgr.status(),
                None => CutStatus::disconnected(),
            },
            // An unknown host is not a local cutter. Falling through to the local manager here
            // would report its `Idle` — and so `actions.cut` — for a device this desktop cannot
            // reach, offering a cut that `execute_cut` would then refuse.
            Err(_) => CutStatus::disconnected(),
            Ok(Route::Host(id)) => {
                // Read before the network call, compared after: a dispatch landing while this poll
                // is in flight makes what it learns stale, and the stale answer must not be the
                // thing that clears the dispatch's own mark.
                let epoch = self.dispatch_epoch.load(std::sync::atomic::Ordering::SeqCst);
                let polled = self
                    // Bounded well under `DEFAULT_BODY_TIMEOUT` (30s) on both legs, reconnect and
                    // body read: a stale snapshot is fine when the next poll is a second away.
                    // The real total is roughly 2x `STATUS_POLL_TIMEOUT` (4s), not one, and it
                    // starts only once this host's connection lock is in hand — which is why the
                    // window-close guard reads `status_without_dialling` instead of this (#115).
                    // Overrunning it costs a late status for this host and nothing else: only this
                    // host's connection is held here.
                    .with_host_within(&id, STATUS_POLL_TIMEOUT, |c| c.snapshots_within(STATUS_POLL_TIMEOUT))
                    .ok()
                    .and_then(|snaps| {
                        snaps.into_iter().find(|s| s.info.instance_id == device.instance_id).map(|s| s.status)
                    });
                match polled {
                    Some(status) => {
                        if self.dispatch_epoch.load(std::sync::atomic::Ordering::SeqCst) == epoch {
                            let key = (id, device.instance_id.clone());
                            // `actions.cut` is the cutter saying it would take a Job right now,
                            // which is the only authority for "nothing of ours is running there".
                            // Read from `actions`, never the phase — `Idle` is also what a cutter
                            // reports between the accept and the first motion.
                            if status.actions.cut {
                                self.remote_dispatched.lock().unwrap().remove(&key);
                            }
                            self.last_remote_status.lock().unwrap().insert(key, status.clone());
                        }
                        status
                    }
                    // A host that cannot be reached mid-cut is not a finished cut: the Job is
                    // still running on the Pi, and saying `Idle` here would invite a second
                    // dispatch. The last known status is deliberately *not* substituted here —
                    // this is the live read, and a caller asking it wants what is true now.
                    None => CutStatus::disconnected(),
                }
            }
        }
    }

    /// What was last heard about the aimed cutter, without touching the network.
    ///
    /// The window-close guard's read. `status()` cannot be it: its 2s budget starts only once the
    /// host's connection lock is in hand, and `list_devices` holds that same lock across a 30s
    /// call — so a close arriving during a periodic device-list poll waited for the listing, then
    /// its own budget, on a synchronous Tauri callback that runs on the main thread (#115).
    ///
    /// A cached value is the honest answer for what this is asked: whether to warn that a cut is
    /// in progress. Stale-active warns about a Job that may have finished, which costs a dialog.
    /// This mirrors what the local path already does — `driver-core` publishes `CutStatus`
    /// precisely so a status read never blocks.
    pub fn status_without_dialling(&self) -> CutStatus {
        let aimed = self.connected.lock().unwrap().clone();
        let Some(device) = aimed else { return CutStatus::disconnected() };
        match self.route(&device) {
            Ok(Route::Local) => match self.local_manager.lock().unwrap().as_ref() {
                Some(mgr) => mgr.status(),
                None => CutStatus::disconnected(),
            },
            Err(_) => CutStatus::disconnected(),
            Ok(Route::Host(id)) => self
                .last_remote_status
                .lock()
                .unwrap()
                .get(&(id, device.instance_id))
                .cloned()
                .unwrap_or_else(CutStatus::disconnected),
        }
    }

    /// Whether a cut may be running on the aimed cutter, answered without dialling.
    ///
    /// What the window-close guard actually wants to know, and *not* the same question as
    /// `status_without_dialling().is_active()`. A dispatch accepted a second ago has not been
    /// polled yet, so the newest status anyone holds is the `Idle` from before it — and a guard
    /// reading only that waves the operator past the cut they just started. A dispatch this
    /// desktop sent and has not since seen finish counts, whatever the last status said.
    ///
    /// Errs toward warning: a Job that has since finished costs a dialog the operator dismisses,
    /// while the other way round loses the only warning there was.
    pub fn a_cut_may_be_running(&self) -> bool {
        if self.status_without_dialling().is_active() {
            return true;
        }
        let aimed = self.connected.lock().unwrap().clone();
        let Some(device) = aimed else { return false };
        match self.route(&device) {
            Ok(Route::Host(id)) => {
                self.remote_dispatched.lock().unwrap().contains(&(id, device.instance_id))
            }
            _ => false,
        }
    }

    pub fn cancel(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            None | Some(Route::Local) => {
                self.manager()?.cancel();
                Ok(())
            }
            Some(Route::Host(id)) => {
                let device = aimed.expect("a route implies a device").instance_id;
                self.with_host(&id, |c| c.cancel(&device))
            }
        }
    }

    pub fn resume(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            None | Some(Route::Local) => {
                self.manager()?.resume().map_err(|e| IpcError::new("device_error", format!("{e:?}")))
            }
            Some(Route::Host(id)) => {
                let device = aimed.expect("a route implies a device").instance_id;
                self.with_host(&id, |c| c.resume(&device))
            }
        }
    }

    pub fn confirm_pass_done(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            None | Some(Route::Local) => {
                self.manager()?.confirm_pass_done().map_err(|e| IpcError::new("device_error", format!("{e:?}")))
            }
            Some(Route::Host(id)) => {
                let device = aimed.expect("a route implies a device").instance_id;
                self.with_host(&id, |c| c.confirm_pass_done(&device))
            }
        }
    }

    /// Normal-exit lifecycle path: take the sole stored `Arc`, unwrap it, and
    /// consume `DeviceManager::shutdown(self)`. If a call is mid-flight (a
    /// clone is briefly alive), that's a non-fatal race at process exit — log
    /// and move on rather than block or panic.
    pub fn shutdown(&self) {
        let Some(arc) = self.local_manager.lock().unwrap().take() else { return };
        match Arc::try_unwrap(arc) {
            Ok(mgr) => mgr.shutdown(),
            Err(_) => eprintln!("device manager shutdown skipped: a call was still in flight"),
        }
    }

    /// Locks `app`'s document just long enough for `cutplan::plan_cut` to plan,
    /// revalidate and preflight it — returns an owned `Vec<CutPass>` with no
    /// remaining borrow of `app`, so the caller drops the document lock
    /// *before* calling `execute_cut` (which blocks on the worker thread).
    /// Never touches `AppStateHandle`'s mutex beyond that.
    ///
    /// The device Preflight ran against comes back with the Passes: it is what
    /// `execute_cut` compares the aim to, since the two calls do not share a lock.
    ///
    /// What stays here is what `cutplan` cannot know: which device is plugged
    /// in, which driver serves it, and where the presets file lives.
    pub fn prepare_cut(&self, app: &AppState, request: CutRequest) -> Result<(DeviceInfo, Vec<CutPass>), IpcError> {
        let connected = self.connected.lock().unwrap().clone()
            .ok_or_else(|| IpcError::new("not_connected", "no device connected"))?;
        if connected.instance_id != request.device_instance_id {
            return Err(IpcError::new("device_mismatch", "connected device changed since planning"));
        }

        let driver = self.local_factory.driver_for(&connected.machine_id)
            .ok_or_else(|| IpcError::new("unknown_machine", format!("no driver for `{}`", connected.machine_id)))?;
        let profile = driver.profile().clone();
        let caps = driver.caps();

        // Only enabled passes are cut, so only their presets are worth reading.
        let enabled = || request.passes.iter().filter(|p| p.enabled);
        let presets: Vec<MaterialPreset> = if enabled().any(|p| p.preset_id.is_some()) {
            let path = default_presets_path()
                .ok_or_else(|| IpcError::new("no_config_dir", "cannot resolve presets file location"))?;
            load_presets(&path).map_err(|e| IpcError::new("preset_error", format!("{e:?}")))?
        } else {
            Vec::new()
        };

        let passes: Vec<PassSelection> = enabled()
            .map(|dto| {
                let preset = dto.preset_id.as_deref().and_then(|id| presets.iter().find(|p| p.id == id));
                let override_ = SettingsOverride {
                    speed: dto.speed,
                    force: dto.force,
                    repeat_count: dto.repeat_count,
                };
                PassSelection { color: dto.color, settings: resolve_settings(preset, &override_) }
            })
            .collect();

        // The wire carries the revision as a string. One that isn't a u64 was
        // never issued by `doc_revision`, so it cannot be the current plan.
        let Ok(expected) = request.doc_revision.parse::<u64>() else {
            return Err(IpcError::new("stale_plan", "cut request carries an unrecognized plan revision"));
        };
        let opts = PlanOptions { passes, expect_revision: Some(expected), allow_out_of_bounds: false };

        // Planned here, at cut time, against the live document — `expect_revision`
        // is what refuses the cut if that is no longer the document the UI planned.
        let planned = plan_passes(&app.editor.doc)
            .map_err(|e| IpcError::new("plan_error", e.to_string()))?;
        let plan = plan_cut(&planned, &profile, &caps, &opts).map_err(map_cut_error)?;
        Ok((connected, plan.cut_passes()))
    }

    /// Submits already-planned passes to the device manager. Blocks until the
    /// worker reaches its first pause point or completion — call this off the
    /// document lock (see `prepare_cut`) and from an async command so it
    /// doesn't freeze the Tauri main loop.
    ///
    /// `planned_for` is the device `prepare_cut` preflighted against. Nothing holds the
    /// aim still across the two calls — the operator can connect another cutter while
    /// planning runs — and the machine-mismatch check downstream cannot catch the swap,
    /// because it would be handed the *new* device's own `machine_id` and so compare it
    /// against itself. So the aim is re-read and compared here, before anything is sent.
    pub fn execute_cut(&self, planned_for: DeviceInfo, passes: Vec<CutPass>) -> Result<CutStarted, IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        // A cutter is its id *and* its host, never the id alone: fallback ids are assigned by
        // location (`usb:at:1:4`, `serial:at:/dev/ttyUSB0`), so two hosts wired alike hand out
        // the same string for two different machines.
        let same_cutter = aimed
            .as_ref()
            .is_some_and(|d| d.instance_id == planned_for.instance_id && d.host == planned_for.host);
        if !same_cutter {
            return Err(IpcError::new(
                "device_mismatch",
                "the connected device changed while this cut was being planned — nothing was sent; re-plan the cut for the device now connected",
            ));
        }

        // Routed by the device Preflight approved, now that it is known to be the one aimed at.
        match self.route(&planned_for)? {
            Route::Local => self
                .manager()?
                .cut(passes)
                .map(|job_id| CutStarted { job_id, duplicate: false })
                .map_err(|e| IpcError::new("device_error", format!("{e:?}"))),
            Route::Host(id) => {
                let (device, machine_id) = (planned_for.instance_id, planned_for.machine_id);
                let key = JobKey {
                    host: id.clone(),
                    device: device.clone(),
                    digest: job_digest(&machine_id, &passes),
                };
                // The id is the Job's, not the clock's. The host deduplicates on it so a retry
                // after a dropped reply cannot cut twice — which only works if a retry arrives
                // under the id it first went out with, and a timestamp never does.
                //
                // A digest alone would be too strong the other way: an operator who cuts one
                // sheet and loads another is dispatching the identical Job on purpose, and must
                // not be silently swallowed as a duplicate. So the id is the digest plus a
                // once-per-attempt nonce, and `in_doubt` decides which of the two this is —
                // reuse while the previous dispatch of this same Job has no answer, mint fresh
                // once it has one. The desktop is the only party that can tell them apart: it
                // is the one that knows whether the last try was answered.
                //
                // The first Cut after a lost reply is therefore always read as the retry. That
                // is the safe direction, and it self-clears: the retry gets an answer (the
                // host's dedupe makes it a no-op if the Job is already running), which frees the
                // next Cut to be a new Job. A desktop restarted in between loses `in_doubt` and
                // is back to cutting twice — persisting it is the fix if that stops being rare.
                //
                // Chosen and written down in one step, before any network call. Choosing and
                // recording separately let two presses of Cut both find nothing and mint an id
                // each: if the first reached the host and lost its reply, the second's entry
                // replaced the only record of it, and the retry that should have been recognised
                // went out under a name the host had never seen.
                // `first_attempt` is information, not ownership: it says whether an *earlier*
                // dispatch of this Job is still unanswered, which changes what a failure here
                // means. It is deliberately not used to decide whether to undo the reservation.
                let (dispatch_id, first_attempt) = self.reserve_dispatch_id(&key);
                // Marked before the request rather than after the answer: an accepted dispatch
                // whose reply is lost is exactly the case the window-close guard must still warn
                // about, and by then there is nothing to write it from.
                self.remote_dispatched.lock().unwrap().insert((id.clone(), device.clone()));
                // Bumped either side of the call, not just before it. A poll that begins after the
                // mark but before the host has the Job sees a cutter that would still take one,
                // and clearing on that reading throws away the mark the dispatch just made. Only a
                // poll that ran entirely after the dispatch finished sees an unchanged count.
                self.dispatch_epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                let reached_the_host = std::sync::atomic::AtomicBool::new(false);
                let sent = self.with_host(&id, |c| {
                    reached_the_host.store(true, std::sync::atomic::Ordering::SeqCst);
                    let sent = c.dispatch(dispatch_id.clone(), &device, &machine_id, passes);
                    // Cleared by any answer — refusals included, since a host that refused was
                    // reached and its answer is not in doubt. What marks a reply lost is this
                    // entry outliving the call.
                    if !matches!(sent, Err(cut_host::client::ClientError::Transport(_))) {
                        self.in_doubt.lock().unwrap().remove(&key);
                    }
                    // A refusal is the host saying it started nothing, so it is not something the
                    // close guard should hold the window for.
                    if matches!(sent, Err(cut_host::client::ClientError::Refused(_))) {
                        self.remote_dispatched.lock().unwrap().remove(&(id.clone(), device.clone()));
                    }
                    sent
                });
                self.dispatch_epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                if !reached_the_host.load(std::sync::atomic::Ordering::SeqCst) {
                    // Nothing was started, so nothing holds the window.
                    self.remote_dispatched.lock().unwrap().remove(&(id.clone(), device.clone()));
                    // The reservation stays. Removing it needed this call to know it owned the
                    // entry, and it cannot: a concurrent press of Cut on the same Job is *using*
                    // that id, so undoing "its own" reservation could delete the record the other
                    // one still needs and let a later retry go out under a name the host has never
                    // seen. Leaving it costs nothing — a dispatch that never reached a host is one
                    // the host cannot have seen either, so the next Cut reusing this id is cut
                    // normally rather than deduplicated away.
                }

                match sent {
                    // ponytail: a remote dispatch reports job id 0, because `Response::Accepted` carries none —
                    // `DeviceManager::cut` does not return one until the Job reaches a pause point. Nothing reads
                    // this value for a remote cut today; give it the real id when the desktop shows per-Job history.
                    //
                    // `duplicate` is the host's own answer, not a guess: it is the only party that
                    // knows whether it had already accepted this id, and "already accepted" and
                    // "your Job has started" look identical to the operator otherwise — one of
                    // them means the cutter is never going to move (#121).
                    Ok(admitted) => Ok(CutStarted {
                        job_id: 0,
                        duplicate: admitted == cut_host::protocol::Admitted::AlreadyAccepted,
                    }),
                    // Never reached the host, and nothing of this Job was outstanding before it:
                    // there is nothing in doubt, so this stays the plain error it is. Both halves
                    // matter — a first press against an offline Pi must not claim the Job may be
                    // cutting, and a press made while an earlier dispatch is unanswered must.
                    Err(e)
                        if first_attempt
                            && !reached_the_host.load(std::sync::atomic::Ordering::SeqCst) =>
                    {
                        Err(e)
                    }
                    Err(e) => Err(IpcError::new(
                        "dispatch_unconfirmed",
                        format!(
                            "{} — the Job may already be cutting there. Press Cut again to retry \
                             it: the host recognizes the same Job and will not cut it twice.",
                            e.message
                        ),
                    )),
                }
            }
        }
    }

    /// The id this Job goes out under — the one its unanswered dispatch already used, or a fresh
    /// one — recorded as in doubt in the same breath. `true` means this call is what recorded it,
    /// which is what lets a caller that never reached the host undo its own reservation without
    /// discarding an *earlier* dispatch's, the one a retry still needs.
    ///
    /// Get-and-record under one lock, because the gap between them is a Cut pressed twice: both
    /// presses found nothing, minted an id each, and the second overwrote the record of the first
    /// — so if the first had reached the host and lost its reply, the retry that should have been
    /// recognised went out under a name the host had never seen, and cut the material again.
    ///
    /// The nonce is wall-clock rather than a counter because a counter restarts at zero with the
    /// process while the host remembers ids for an hour — a second session would mint ids the host
    /// already knows and have its cuts silently deduplicated away.
    fn reserve_dispatch_id(&self, key: &JobKey) -> (cut_host::protocol::DispatchId, bool) {
        let now = std::time::Instant::now();
        let mut in_doubt = self.in_doubt.lock().unwrap();

        // Entries the host can no longer recognise are not retries any more, whatever they once
        // were: past `ID_RETENTION` it has forgotten the id, so reusing it protects nothing and
        // only hides that fact. This is *not* the fifteen-minute expiry that was removed — that
        // one discarded protection while it still existed. Aligned to the host's own constant so
        // the two sides cannot drift apart about when the window closes.
        in_doubt.retain(|_, (_, written)| {
            now.saturating_duration_since(*written) < cut_host::host::ID_RETENTION
        });
        // Looked up before anything is evicted, so the cap can never discard the very entry this
        // call is here to reuse.
        if let Some((id, _)) = in_doubt.get(key) {
            return (id.clone(), false);
        }

        // A backstop, not a policy: entries are per Job, so a long session cutting many different
        // designs is what grows this, and the oldest is the one a retry is least likely to name.
        // Room is made *before* the insert, so the map settles at the cap rather than one past it.
        while in_doubt.len() >= MAX_JOBS_IN_DOUBT {
            let Some(oldest) =
                in_doubt.iter().min_by_key(|(_, (_, written))| *written).map(|(k, _)| k.clone())
            else {
                break;
            };
            in_doubt.remove(&oldest);
        }
        // The counter is what makes two attempts in the same clock tick distinct; the clock is
        // what keeps this run's ids clear of the previous run's. Neither alone is enough.
        static ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // The cutter's id is truncated because it is the one part with no length of its own: a
        // serial port enumerated by path can be most of a `/dev/serial/by-id/...` string, and the
        // host refuses an id longer than it will remember — which would have refused *every*
        // dispatch to that cutter. The digest already identifies the Job exactly; the text is for
        // whoever is reading a log.
        let device: String = key.device.chars().take(32).collect();
        let id = cut_host::protocol::DispatchId(format!(
            "{}-{:016x}-{}-{}",
            device,
            key.digest,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            ATTEMPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        in_doubt.insert(key.clone(), (id.clone(), now));
        (id, true)
    }

    /// Test convenience: `prepare_cut` + `execute_cut` in one call. Production
    /// callers (`ipc::cut`) keep the two steps separate so the document lock
    /// is dropped before the blocking `execute_cut` call.
    #[cfg(test)]
    fn cut_from_request(&self, app: &AppState, request: CutRequest) -> Result<CutStarted, IpcError> {
        let (planned_for, passes) = self.prepare_cut(app, request)?;
        self.execute_cut(planned_for, passes)
    }
}

/// Every way `plan_cut` can refuse, as an IPC code the UI can branch on.
/// `stale_plan` is the one the frontend actually keys off (CutDialog.tsx).
/// The message is `cutplan`'s — this used to restate all ten here, and the CLI
/// restated them again, differently.
fn map_cut_error(e: CutError) -> IpcError {
    IpcError::new(e.code(), e.to_string())
}

#[derive(Debug, Serialize)]
pub struct PlanCutResponse {
    pub passes: Vec<PlanCutPassSummary>,
    pub skipped_no_stroke: usize,
    pub doc_revision: String,
    pub travel: Vec<[f64; 4]>,
}

#[derive(Debug, Serialize)]
pub struct PlanCutPassSummary {
    pub color: Option<u32>,
    pub shape_count: usize,
    pub node_ids: Vec<document::NodeId>,
    /// Each shape's first world-space point, parallel to `node_ids` — where the blade
    /// actually lands. The preview's order badges anchor here; the UI cannot derive it
    /// from `travel`, which has no move to the first shape and none for a single-shape
    /// plan. `None` is a shape whose outline flattened to nothing.
    pub starts: Vec<Option<[f64; 2]>>,
}

/// Summarizes `plan_passes` output for the UI — not the raw `DocumentPasses`
/// (which carries full flattened polylines the cut dialog doesn't need).
pub fn plan_cut_response(doc: &document::Document) -> Result<PlanCutResponse, IpcError> {
    let planned = plan_passes(doc).map_err(|e| IpcError::new("plan_error", e.to_string()))?;
    let refs: Vec<&ColorPass> = planned.passes.iter().collect();
    let travel = cutplan::travel_moves(&refs);
    Ok(PlanCutResponse {
        passes: planned.passes.iter().map(|p| PlanCutPassSummary {
            color: p.color,
            shape_count: p.shapes.len(),
            node_ids: p.shapes.iter().map(|s| s.node_id).collect(),
            starts: p.shapes.iter().map(|s| {
                s.polylines.first().and_then(|p| p.first()).map(|pt| [pt.x, pt.y])
            }).collect(),
        }).collect(),
        skipped_no_stroke: planned.skipped_no_stroke,
        doc_revision: planned.doc_revision.to_string(),
        travel: travel.into_iter().map(|(a, b)| [a.x, a.y, b.x, b.y]).collect(),
    })
}

/// Travel for the pass order the cut dialog currently shows, replanned against the live
/// document. Reordering is a UI-side list edit — the plan itself is not resent — so this
/// re-asks the planner rather than letting the frontend recompute travel from geometry
/// it does not have. The revision check keeps the stale-plan rule: travel for an order
/// the operator arranged against a document that has since changed is refused, exactly
/// as the cut itself would be.
pub fn travel_for_order(
    doc: &document::Document,
    doc_revision: &str,
    order: &[Option<u32>],
) -> Result<Vec<[f64; 4]>, IpcError> {
    // Same rule as `prepare_cut`: a revision string that isn't a u64 was never
    // issued by `doc_revision`, so it cannot be the current plan.
    let Ok(expected) = doc_revision.parse::<u64>() else {
        return Err(IpcError::new("stale_plan", "travel request carries an unrecognized plan revision"));
    };
    let planned = plan_passes(doc).map_err(|e| IpcError::new("plan_error", e.to_string()))?;
    if planned.doc_revision != expected {
        return Err(map_cut_error(CutError::StalePlan { expected, actual: planned.doc_revision }));
    }
    // With the revision equal, the plan is the one the dialog's rows came from, so the
    // order must name each planned pass exactly once — anything else is a caller bug,
    // and travel for a subset would silently misdraw the machine's motion.
    let mut remaining: Vec<&ColorPass> = planned.passes.iter().collect();
    let mut refs: Vec<&ColorPass> = Vec::with_capacity(order.len());
    for color in order {
        let Some(i) = remaining.iter().position(|p| p.color == *color) else {
            return Err(map_cut_error(CutError::UnknownPassColor(*color)));
        };
        refs.push(remaining.remove(i));
    }
    if !remaining.is_empty() {
        return Err(IpcError::new("plan_mismatch", "the requested pass order does not name every planned pass"));
    }
    Ok(cutplan::travel_moves(&refs).into_iter().map(|(a, b)| [a.x, a.y, b.x, b.y]).collect())
}

/// Re-derives the on-disk *user-only* preset list (builtins always shadow-load
/// with `builtin:false` forced onto user entries — see `cutplan::presets::load_presets`)
/// so `save_preset`/`delete_preset` round-trip through `save_user_presets` correctly
/// without ever writing a builtin back to disk.
fn user_presets_path() -> Result<std::path::PathBuf, IpcError> {
    default_presets_path().ok_or_else(|| IpcError::new("no_config_dir", "cannot resolve presets file location"))
}

pub fn list_presets(machine_id: &str) -> Result<Vec<MaterialPreset>, IpcError> {
    let path = user_presets_path()?;
    let all = load_presets(&path).map_err(|e| IpcError::new("preset_error", format!("{e:?}")))?;
    Ok(all.into_iter().filter(|p| p.machine_id == machine_id).collect())
}

pub fn save_preset(preset: MaterialPreset) -> Result<(), IpcError> {
    let path = user_presets_path()?;
    let mut user: Vec<MaterialPreset> = load_presets(&path)
        .map_err(|e| IpcError::new("preset_error", format!("{e:?}")))?
        .into_iter().filter(|p| !p.builtin).collect();
    user.retain(|p| p.id != preset.id);
    user.push(MaterialPreset { builtin: false, ..preset });
    save_user_presets(&path, &user).map_err(|e| IpcError::new("preset_error", format!("{e:?}")))
}

pub fn delete_preset(id: &str) -> Result<(), IpcError> {
    let path = user_presets_path()?;
    let mut user: Vec<MaterialPreset> = load_presets(&path)
        .map_err(|e| IpcError::new("preset_error", format!("{e:?}")))?
        .into_iter().filter(|p| !p.builtin).collect();
    user.retain(|p| p.id != id);
    save_user_presets(&path, &user).map_err(|e| IpcError::new("preset_error", format!("{e:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cutplan::DocumentPasses;
    use driver_core::{Driver, Job, MachineCaps, MachineProfile, Phase, Transport, TransportError, TransportKind};

    struct TestDriver { profile: MachineProfile, caps: MachineCaps }
    impl Driver for TestDriver {
        fn profile(&self) -> &MachineProfile { &self.profile }
        fn caps(&self) -> MachineCaps { self.caps }
        fn session_begin(&self) -> Vec<u8> { vec![] }
        fn encode_pass(&self, _pass: &Job) -> Result<Vec<u8>, driver_core::DriverError> { Ok(vec![]) }
        fn pass_park(&self) -> Vec<u8> { vec![] }
        fn session_end(&self) -> Vec<u8> { vec![] }
        fn abort_bytes(&self) -> Option<Vec<u8>> { None }
    }

    struct TestFactory;
    impl DeviceBackendFactory for TestFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> { vec![test_instance()] }
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
        fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            Ok(Box::new(driver_core::MockTransport::default()))
        }
    }

    fn test_instance() -> DeviceInfo {
        DeviceInfo {
            instance_id: "usb:1:4".into(),
            machine_id: "cameo5".into(),
            transport: TransportKind::Usb { locator: "1:4".into() },
            candidate: false,
            host: None,
        }
    }

    fn test_device_setup() -> DeviceManagerHandle {
        let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
        dev.connect(test_instance()).unwrap();
        dev
    }

    fn plan_for(app: &AppState) -> DocumentPasses {
        plan_passes(&app.editor.doc).unwrap()
    }

    fn request_from(plan: DocumentPasses) -> CutRequest {
        CutRequest {
            device_instance_id: test_instance().instance_id,
            doc_revision: plan.doc_revision.to_string(),
            passes: plan.passes.iter().map(|p| ConfiguredPassDto {
                color: p.color, enabled: true, preset_id: None, speed: None, force: None, repeat_count: None,
            }).collect(),
        }
    }

    #[test]
    fn cut_request_with_stale_revision_is_rejected() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        app.add_rect(5.0, 5.0);
        let err = dev.cut_from_request(&app, request_from(plan)).unwrap_err();
        assert_eq!(err.code, "stale_plan");
    }

    #[test]
    fn preflight_failures_map_to_ipc_codes() {
        let app = AppState::new();
        let dev = test_device_setup();
        let revision = cutplan::doc_revision(&app.editor.doc);
        let request = CutRequest { device_instance_id: test_instance().instance_id, doc_revision: revision.to_string(), passes: vec![] };
        let err = dev.cut_from_request(&app, request).unwrap_err();
        assert_eq!(err.code, "nothing_to_cut");
    }

    #[test]
    fn unknown_pass_color_is_rejected_not_dropped() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        let mut request = request_from(plan);
        request.passes[0].color = Some(0xDEADBEEF); // doesn't match any planned pass
        let err = dev.cut_from_request(&app, request).unwrap_err();
        assert_eq!(err.code, "unknown_pass_color");
    }

    const RED: u32 = 0xFF0000FF;
    const BLUE: u32 = 0x0000FFFF;

    /// A stroked rect offset along x, so travel direction between shapes is assertable.
    fn add_stroked_rect(app: &mut AppState, stroke: u32, x: f64) {
        use document::{Delta, Node, NodeOp, ShapeKind, Style};
        let id = app.editor.doc.ids.next();
        let mut node = Node::shape(id, ShapeKind::Rect { w: 10.0, h: 10.0 });
        node.style = Style { stroke: Some(stroke), fill: None };
        node.transform = geometry::Affine::translate(x, 0.0);
        app.editor.commit(Delta(vec![NodeOp::Add { parent: app.editor.doc.root, node, index: usize::MAX }]));
    }

    fn two_color_doc() -> (AppState, String) {
        let mut app = AppState::new();
        add_stroked_rect(&mut app, RED, 0.0);
        add_stroked_rect(&mut app, BLUE, 100.0);
        let revision = cutplan::doc_revision(&app.editor.doc).to_string();
        (app, revision)
    }

    #[test]
    fn travel_for_order_follows_the_requested_order() {
        let (app, revision) = two_color_doc();

        // The plan's own first-seen order reproduces exactly what plan_cut_response sent.
        let planned = travel_for_order(&app.editor.doc, &revision, &[Some(RED), Some(BLUE)]).unwrap();
        assert_eq!(planned, plan_cut_response(&app.editor.doc).unwrap().travel);
        assert_eq!(planned.len(), 1);
        assert!(planned[0][2] >= 100.0, "red first: travel lands on the blue rect at x=100, got {planned:?}");

        let reversed = travel_for_order(&app.editor.doc, &revision, &[Some(BLUE), Some(RED)]).unwrap();
        assert_eq!(reversed.len(), 1);
        assert!(reversed[0][0] >= 100.0 && reversed[0][2] <= 10.0,
            "blue first: travel leaves x=100 for the red rect at the origin, got {reversed:?}");
    }

    #[test]
    fn travel_for_order_with_a_stale_revision_is_refused() {
        let (mut app, revision) = two_color_doc();
        app.add_rect(5.0, 5.0);
        let err = travel_for_order(&app.editor.doc, &revision, &[Some(RED), Some(BLUE)]).unwrap_err();
        assert_eq!(err.code, "stale_plan");
    }

    #[test]
    fn travel_for_order_with_an_unknown_color_is_refused() {
        let (app, revision) = two_color_doc();
        let err = travel_for_order(&app.editor.doc, &revision, &[Some(RED), Some(0xDEADBEEF)]).unwrap_err();
        assert_eq!(err.code, "unknown_pass_color");
    }

    #[test]
    fn travel_for_order_missing_a_planned_pass_is_refused() {
        let (app, revision) = two_color_doc();
        let err = travel_for_order(&app.editor.doc, &revision, &[Some(RED)]).unwrap_err();
        assert_eq!(err.code, "plan_mismatch");
    }

    #[test]
    fn plan_cut_response_carries_each_shapes_first_world_point() {
        let (app, _) = two_color_doc();
        let response = plan_cut_response(&app.editor.doc).unwrap();
        for pass in &response.passes {
            assert_eq!(pass.starts.len(), pass.node_ids.len(), "starts is parallel to node_ids");
        }
        // The blue rect's translate must show in its start — a local-space point here
        // would put the badge at the origin instead of on the shape.
        let blue = response.passes.iter().find(|p| p.color == Some(BLUE)).unwrap();
        let start = blue.starts[0].unwrap();
        assert!(start[0] >= 100.0, "world-space start, got {start:?}");
    }

    /// The bridge used to synthesize `Transmitting` from `Progress` because the
    /// worker never re-emitted a state mid-transmit. `CutStatus` carries the
    /// progress in the phase itself, so the synthesis has nothing left to do —
    /// note that nothing here consumes the event channel at all.
    #[test]
    fn status_reports_a_mid_flight_cut_without_the_bridge_synthesizing_it() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        dev.cut_from_request(&app, request_from(plan)).expect("cut");
        let s = dev.status();
        // The exact phase, not merely a mid-flight one: a fixture that let the cut
        // run to completion would leave nothing in flight for the status to report,
        // and this test would then pass without ever exercising the question.
        assert_eq!(s.phase, Phase::AwaitingConfirmation, "the cut should be parked mid-flight");
        assert!(s.is_active(), "a parked cut is what makes the close guard block a quit");
    }

    #[test]
    fn status_without_a_manager_reads_disconnected() {
        let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
        dev.shutdown();
        assert_eq!(dev.status().phase, Phase::Disconnected);
    }

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

        // Not just "nothing is host-tagged" — an empty list satisfies that vacuously, and the
        // regression this guards against is exactly one that returns nothing.
        assert!(!listed.is_empty(), "the local factory's devices must still be listed");
        assert_eq!(listed.len(), TestFactory.list_devices().len());
        assert!(listed.iter().all(|d| d.host.is_none()), "{listed:?}");
    }

    /// A host that cannot be reached keeps its place in the list rather than vanishing — a
    /// cutter that disappears looks like one that was never paired.
    #[test]
    fn an_unreachable_host_is_still_listed_with_its_reason() {
        let dev = test_device_setup();
        // Nothing is listening on this port, so connecting fails.
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

        // `add_host` records the host without dialling it — connections are lazy — so the
        // failure only exists once something asks for its cutters.
        let listed = dev.list_devices();
        assert!(listed.iter().all(|d| d.host.is_none()), "an unreachable host contributes none");

        let views = dev.host_views();
        assert_eq!(views.len(), 1, "the host stays known: {views:?}");
        assert!(views[0].unreachable.is_some(), "and says why it is unreachable");
    }

    /// A cutter with no host is this computer's, and must reach the local DeviceManager — the
    /// path every existing user is on.
    #[test]
    fn a_local_device_still_routes_to_the_local_manager() {
        let dev = test_device_setup();
        assert_eq!(dev.status().phase, driver_core::Phase::Idle);
        assert!(dev.cancel().is_ok(), "a local cancel reaches the local manager");
    }

    /// A moving blade must not become unstoppable: the window-close guard and `cancel` both act
    /// on the *current* aim, so letting `connect` move it to a host mid-cut would deafen the
    /// guard (it would see the host's `Idle`) and mis-route `cancel` to a machine that was never
    /// asked to stop. Mirrors `forget`'s own busy refusal.
    #[test]
    fn connect_refuses_to_move_the_aim_off_an_active_local_cut() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        dev.cut_from_request(&app, request_from(plan)).expect("cut");
        assert!(dev.status().is_active(), "the local cut must be parked mid-flight for this test to mean anything");

        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        let elsewhere = DeviceInfo { host: Some(HostId("host-1".into())), ..test_instance() };
        let err = dev.connect(elsewhere).unwrap_err();
        assert_eq!(err.code, "device_error", "got {err:?}");

        // A refused connect is not a half-connect: the aim must not have moved either.
        assert_eq!(dev.connected.lock().unwrap().as_ref().unwrap().host, None, "aim must stay local");
    }

    /// Aiming at a host used to leave the local `DeviceManager` connected and `Idle` — and
    /// `DeviceManager::connect` refuses anything but `Disconnected`/`Error` — so re-aiming
    /// locally afterward returned `Busy`, stranding the operator's own cutter for the rest of
    /// the session. `connect`'s `Route::Host` arm must release the local manager first.
    #[test]
    fn re_aiming_locally_after_a_host_succeeds() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        let elsewhere = DeviceInfo { host: Some(HostId("host-1".into())), ..test_instance() };
        dev.connect(elsewhere).expect("aiming at a host is bookkeeping only");

        dev.connect(test_instance()).expect("must be able to aim back at the local cutter");
    }

    /// A remote cutter's connection belongs to the Cut Host, not this desktop. Routing
    /// `disconnect` unconditionally to the local manager used to close the local cutter's
    /// transport and discard its parked job while the operator was aimed at a Pi, silently.
    #[test]
    fn disconnect_while_aimed_at_a_host_leaves_the_local_manager_untouched() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        dev.connected.lock().unwrap().replace(DeviceInfo { host: Some(HostId("host-1".into())), ..test_instance() });

        dev.disconnect().expect("disconnecting a remote aim is bookkeeping only");
        assert!(dev.connected.lock().unwrap().is_none(), "the aim itself must still clear");

        // Re-aim locally: if `disconnect` had reached the local manager, this would now read
        // `Disconnected` instead of the `Idle` it started at.
        dev.connected.lock().unwrap().replace(test_instance());
        assert_eq!(dev.status().phase, Phase::Idle, "the local manager's connection must have survived untouched");
    }

    /// A cancel whose stop nothing confirmed refuses another cut, and `DeviceManager::connect`
    /// refuses from that state too — so the disconnect is the whole exit, and the dialog now
    /// offers one. Asserted through `actions`: the phase is `Idle` either side of the reconnect,
    /// which is exactly why it cannot answer this.
    #[test]
    fn disconnecting_and_reconnecting_makes_a_cut_legal_after_an_unconfirmed_cancel() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        dev.cut_from_request(&app, request_from(plan)).expect("cut");

        dev.cancel().expect("cancel");
        // The test Driver parks rather than polls, so nothing can confirm its stop — the Puma's
        // ordinary case, and the one the operator has to be able to get out of.
        let stuck = wait_for_cancelled(&dev);
        assert!(!stuck.actions.cut, "nothing saw the machine stop, so no Job may follow it");

        dev.disconnect().expect("disconnect");
        dev.connect(test_instance()).expect("reconnect");
        assert!(dev.status().actions.cut, "a reconnected cutter accepts a Job again");
    }

    /// The control the dialog offers is hidden mid-Job, but that guard lives in the webview and
    /// reads a status documented to lag the worker by one event. Both verbs drop the local
    /// cutter's transport, so the refusal has to be here too — `Host::reconnect` already answers
    /// the same question for a remote cutter.
    #[test]
    fn dropping_the_local_transport_is_refused_while_a_cut_is_working() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        dev.cut_from_request(&app, request_from(plan)).expect("cut");
        assert!(dev.status().is_active(), "the Job is parked mid-flight, not finished");

        for refused in [dev.disconnect(), dev.reconnect()] {
            let err = refused.expect_err("a working cutter must keep its transport");
            assert_eq!(err.code, "device_error");
            assert!(err.message.contains("cancel it"), "and must say the way through: {}", err.message);
        }
        // Still parked, and still answerable — the refusal changed nothing.
        assert!(dev.status().actions.confirm);
    }

    fn wait_for_cancelled(dev: &DeviceManagerHandle) -> CutStatus {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let s = dev.status();
            if s.ended == Some(driver_core::Ended::Cancelled) {
                return s;
            }
            assert!(std::time::Instant::now() < deadline, "never cancelled, sat at {:?}", s.phase);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
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

    /// A device aimed at a host that was forgotten (or never paired) must report a status the UI
    /// cannot act on. Asserted on `actions`, not `phase` — this project's rule is that a caller
    /// learns what is legal from `actions` and never re-derives it from the phase.
    #[test]
    fn status_for_a_device_on_an_unknown_host_is_not_a_local_idle() {
        let dev = test_device_setup();
        dev.connected.lock().unwrap().replace(DeviceInfo {
            host: Some(HostId("host-does-not-exist".into())),
            ..test_instance()
        });
        assert!(!dev.status().actions.cut, "an unreachable host must not offer a cut it cannot run");
    }

    #[test]
    fn forgetting_a_host_removes_it_and_its_cutters() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        assert_eq!(dev.host_views().len(), 1, "known as soon as it is added");

        dev.remove_host(&HostId("host-1".into()));
        assert!(dev.host_views().is_empty());
        assert!(dev.list_devices().iter().all(|d| d.host.is_none()));
    }

    /// The token must not leave the Rust side. A view type is the guard, and this is what stops
    /// a later refactor from "simplifying" it back to sending `PairedHost`.
    #[test]
    fn a_host_view_carries_no_token() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

        // `unreachable` is `None` here on purpose: nothing has dialled this host yet, and the
        // view reports what is known rather than provoking a connection to find out.
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

    // --- pair()/forget(): network refusals, persist-then-mutate, busy-host refusal ---
    //
    // `crates/cut-host/tests/fixtures/mod.rs` has this exact fixture already, but it lives in a
    // separate integration-test crate and cannot be imported from here — so this is the same
    // loopback host, built from the same public `cut_host` API, kept alive by the same
    // `_dir: TempDir` trick (dropping it deletes the cert directory `serve_on` reads its key from).

    struct LoopbackHost {
        addr: String,
        fingerprint: String,
        _dir: tempfile::TempDir,
    }

    const HOST_TOKEN: &str = "test-token";

    fn start_loopback_host() -> LoopbackHost {
        let dir = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        let config = cut_host::config::Config {
            bind: listener.local_addr().unwrap(),
            tokens: [("test-client".to_string(), HOST_TOKEN.to_string())].into_iter().collect(),
            max_frame: cut_host::frame::DEFAULT_MAX_FRAME,
            cert_dir: dir.path().to_path_buf(),
        };
        // Generated before `Host::start`, so the client has something to pin before the
        // server has accepted anything.
        let fingerprint = cut_host::serve::fingerprint_of_cert_dir(&config.cert_dir).unwrap();
        let host = cut_host::host::Host::start(Arc::new(cut_host::host::testing::TwoCutterFactory));

        std::thread::spawn(move || {
            let _ = cut_host::serve::serve_on(listener, host, config);
        });
        LoopbackHost { addr, fingerprint, _dir: dir }
    }

    /// A regular file where a directory needs to be, so `create_dir_all` — and therefore every
    /// `hosts::save` — fails without touching real filesystem permissions.
    fn unwritable_hosts_path(dir: &std::path::Path) -> std::path::PathBuf {
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"").unwrap();
        blocker.join("hosts.json")
    }

    /// The code, not just the failure. A rejected token, a changed certificate and an offline Pi
    /// call for three different things from the operator, and all three used to arrive as
    /// `host_unreachable` with prose that could not be told apart (#112).
    #[test]
    fn pairing_with_the_wrong_token_is_refused_and_saves_nothing() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let err = dev
            .pair("Pi".into(), host.addr.clone(), "wrong-token".into(), host.fingerprint.clone(), &hosts_path)
            .expect_err("a token this host does not hold must not pair");
        assert_eq!(err.code, "host_unauthorized", "got {err:?}");
        assert!(!hosts_path.exists(), "a pairing that never proved itself must not be written");
    }

    #[test]
    fn pairing_with_the_wrong_fingerprint_is_refused_and_saves_nothing() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let err = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), "wrong:fingerprint".into(), &hosts_path)
            .expect_err("a certificate that is not the pinned one is a hard refusal");
        assert_eq!(err.code, "host_fingerprint", "got {err:?}");
        assert!(!hosts_path.exists());
    }

    #[test]
    fn pairing_an_unreachable_address_is_refused_and_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        // The conventional black-holed address: routable, never answering.
        let err = dev
            .pair("Pi".into(), "10.255.255.1:7878".into(), "token".into(), "aa:bb:cc".into(), &hosts_path)
            .expect_err("nothing answers this address");
        assert_eq!(err.code, "host_unreachable", "got {err:?}");
        assert!(!hosts_path.exists());
    }

    /// Re-pairing an address that is already paired mints a second entry, and must: a changed
    /// fingerprint is refused on every later connection, so pairing again is the only recovery
    /// there is. What was missing is anyone saying so — two rows with the same name and address,
    /// one of them permanently broken, and nothing in the UI explaining which or why (#107).
    #[test]
    fn an_address_that_is_already_paired_is_recognised_before_anything_is_written() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "pi.local:7878"));

        assert!(
            dev.existing_pairing("elsewhere.local:7878", "aa:bb:cc").is_none(),
            "an address nobody has paired is a first pairing"
        );

        let same = dev.existing_pairing("pi.local:7878", "aa:bb:cc").expect("this address is paired");
        assert_eq!(same.id, HostId("host-1".into()));
        assert!(same.same_fingerprint, "the certificate has not changed — this is a re-pair, no more");

        // The case worth saying out loud: the Pi answers with a certificate that is not the one
        // pinned. Either it was reinstalled, or something else is at that address.
        let changed = dev.existing_pairing("pi.local:7878", "ff:ee:dd").expect("this address is paired");
        assert!(!changed.same_fingerprint);
        assert_eq!(changed.name, "Workshop Pi", "and it can be named, so the warning can point at a row");
    }

    #[test]
    fn a_pair_that_fails_to_save_adds_nothing_in_memory() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = unwritable_hosts_path(dir.path());
        let dev = test_device_setup();

        let err = dev.pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path);
        assert!(err.is_err(), "the save must fail against this path");
        assert!(dev.paired_hosts().is_empty(), "a failed save must not leave a phantom host in memory");
    }

    #[test]
    fn a_forget_that_fails_to_save_removes_nothing_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = unwritable_hosts_path(dir.path());
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

        // Forced, so this reaches the save at all: unforced it would stop at the idle check,
        // and the failure under test here is the write, not the network.
        let err = dev.forget(&HostId("host-1".into()), &hosts_path, true);
        assert_eq!(err.unwrap_err().code, "hosts_unwritable");
        assert_eq!(dev.paired_hosts().len(), 1, "a failed save must not remove a host still on disk");
    }

    /// The reversal: a host that cannot be asked is not a host known to be idle. Drop Wi-Fi
    /// mid-Job and the Pi keeps cutting — discarding the token there throws away the only route
    /// left to cancel it. Refused unforced, with its own code, and let through by the force the
    /// operator has to ask for explicitly.
    #[test]
    fn an_unreachable_host_refuses_to_be_forgotten_until_it_is_forced() {
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

        let err = dev.forget(&HostId("host-1".into()), &hosts_path, false).expect_err("silence is not idleness");
        assert_eq!(err.code, "host_unconfirmed", "distinct from the busy refusal: {err:?}");
        assert_eq!(dev.paired_hosts().len(), 1, "refused, so still paired");

        dev.forget(&HostId("host-1".into()), &hosts_path, true).expect("a Pi that is gone for good must stay forgettable");
        assert!(dev.paired_hosts().is_empty());
    }

    #[test]
    fn a_host_with_an_active_cut_refuses_to_be_forgotten() {
        use std::time::{Duration, Instant};

        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("this host answers and the fingerprint matches");

        // `dispatch` returns as soon as the daemon accepts the id; the cut itself runs on a
        // thread there (`Host::dispatch`). Poll until the cutter reports busy rather than
        // asserting on a race.
        let client = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
        client
            .dispatch(
                cut_host::protocol::DispatchId("d-1".into()),
                cut_host::host::testing::CAMEO,
                "cameo5",
                vec![CutPass {
                    job: driver_core::Job {
                        polylines: vec![vec![
                            geometry::Point { x: 0.0, y: 0.0 },
                            geometry::Point { x: 10.0, y: 0.0 },
                            geometry::Point { x: 10.0, y: 10.0 },
                            geometry::Point { x: 0.0, y: 0.0 },
                        ]],
                        settings: driver_core::Settings::default(),
                    },
                }],
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let busy = client
                .snapshots()
                .unwrap()
                .iter()
                .any(|s| s.info.instance_id == cut_host::host::testing::CAMEO && s.status.is_active());
            if busy {
                break;
            }
            assert!(Instant::now() < deadline, "the cutter never went active");
            std::thread::sleep(Duration::from_millis(20));
        }

        let err = dev.forget(&id, &hosts_path, false).expect_err("a moving blade must not become unstoppable");
        assert_eq!(err.code, "host_busy");
        assert_eq!(dev.paired_hosts().len(), 1, "refused, so still paired");

        // Force is for a host that cannot answer, not one that answered "busy": this host is
        // reachable, so `cancel` still works and the blade was never orphaned.
        let forced = dev.forget(&id, &hosts_path, true).expect_err("force must not override a confirmed cut");
        assert_eq!(forced.code, "host_busy");
        assert_eq!(dev.paired_hosts().len(), 1, "still paired");
    }

    /// The two cutters here share an `instance_id` on purpose — the local factory's device and
    /// the loopback host's Cameo are both `usb:1:4`, because a fallback id is assigned by
    /// location and two identically-wired machines really do collide. A guard comparing ids
    /// alone would call these the same cutter and send A's Passes to B.
    #[test]
    fn a_dispatch_whose_aim_moved_after_planning_is_refused() {
        use std::time::{Duration, Instant};

        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);

        let host_id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("this host answers and the fingerprint matches");

        // Preflight runs against the local cutter...
        let (planned_for, passes) = dev.prepare_cut(&app, request_from(plan_for(&app))).unwrap();
        assert_eq!(planned_for.host, None);

        // ...and the operator connects the Pi's Cameo before the dispatch lands.
        *dev.connected.lock().unwrap() = Some(DeviceInfo {
            instance_id: cut_host::host::testing::CAMEO.into(),
            machine_id: "cameo5".into(),
            transport: TransportKind::Usb { locator: "1:4".into() },
            candidate: false,
            host: Some(host_id),
        });

        let err = dev.execute_cut(planned_for, passes).expect_err("Passes approved for A must not go to B");
        assert_eq!(err.code, "device_mismatch");

        // The daemon accepts a dispatch before its cut thread starts, so a single snapshot could
        // read idle on a host that is about to move. Hold the assertion open instead.
        let client = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
        let deadline = Instant::now() + Duration::from_millis(300);
        while Instant::now() < deadline {
            let active = client
                .snapshots()
                .unwrap()
                .iter()
                .any(|s| s.info.instance_id == cut_host::host::testing::CAMEO && s.status.is_active());
            assert!(!active, "the refused Passes reached the host anyway");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn an_idle_host_is_forgotten() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("this host answers and the fingerprint matches");

        dev.forget(&id, &hosts_path, false).expect("it answers, and nothing is running on it");
        assert!(dev.paired_hosts().is_empty());
    }

    /// Otherwise `get_connected_device` keeps answering with a device `list_devices` no longer
    /// lists — the aim would silently outlive the host it named.
    #[test]
    fn forgetting_the_connected_host_clears_the_aim() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("this host answers and the fingerprint matches");
        dev.connected.lock().unwrap().replace(DeviceInfo { host: Some(id.clone()), ..test_instance() });

        dev.forget(&id, &hosts_path, false).expect("it answers, and nothing is running on it");
        assert!(dev.connected.lock().unwrap().is_none(), "the aim must not survive the host it named");
    }

    /// A TCP listener that accepts and then says nothing at all — a Pi that answers the SYN and
    /// then wedges, which is the shape that used to hold the `hosts` map lock for everyone.
    /// Loopback rather than a black-holed address on purpose: how long an unroutable address
    /// takes to fail is the network's business, and a test that needs a real stall must not
    /// depend on it.
    fn start_silent_host() -> (String, std::sync::mpsc::Receiver<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (accepted, saw_accept) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // Held rather than dropped: closing the socket would let the client fail instantly,
            // which is the one thing this fixture must not do.
            let mut open = Vec::new();
            while let Ok((sock, _)) = listener.accept() {
                open.push(sock);
                let _ = accepted.send(());
            }
        });
        (addr, saw_accept)
    }

    /// The property the per-connection locks buy: a call aimed at one host is not delayed by a
    /// call already in flight against another. Before the split this failed — `with_host` held
    /// the whole `hosts` map across its network call, so the healthy host's answer queued behind
    /// the wedged host's timeout, and so did `cancel` and the window-close guard's `status()`.
    ///
    /// Deterministic without sleeping on a guess: the healthy call is only made once the wedged
    /// connection's own lock is *observed* taken, and the assertion is ordering ("the healthy one
    /// answered first"), not a duration.
    #[test]
    fn a_call_to_one_host_is_not_delayed_by_a_wedged_call_to_another() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};

        let (silent_addr, saw_accept) = start_silent_host();
        let healthy = start_loopback_host();
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-wedged", &silent_addr));
        dev.add_host(PairedHost {
            id: HostId("host-healthy".into()),
            name: "Workshop Pi".into(),
            address: healthy.addr.clone(),
            fingerprint: healthy.fingerprint.clone(),
            token: HOST_TOKEN.into(),
        });

        // Taken before anything is in flight, so that observing it later cannot itself be what
        // blocks — under the old shape this very call would have queued on the map lock, and the
        // test would have failed for a reason one step removed from the one it is about.
        let wedged_conn = dev.host_conn(&HostId("host-wedged".into())).unwrap();

        let wedged_done = AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                let _ = dev.with_host(&HostId("host-wedged".into()), |c| c.devices());
                wedged_done.store(true, Ordering::SeqCst);
            });

            // Two anchors, so "in flight" is observed rather than assumed: the fixture has taken
            // the connection, and the connection's lock is held by the call that took it.
            saw_accept.recv_timeout(Duration::from_secs(10)).expect("the wedged host was never dialled");
            let deadline = Instant::now() + Duration::from_secs(10);
            while wedged_conn.try_lock().is_ok() {
                assert!(Instant::now() < deadline, "the wedged call never took its connection lock");
                std::thread::sleep(Duration::from_millis(5));
            }

            dev.with_host(&HostId("host-healthy".into()), |c| c.devices())
                .expect("the healthy host answers regardless of what the wedged one is doing");
            assert!(
                !wedged_done.load(Ordering::SeqCst),
                "the healthy host only answered once the wedged call gave up — the two are still serialized"
            );
        });
    }

    /// The window-close guard's read must not dial. Its 2s budget only starts once the host's
    /// connection lock is in hand, and `list_devices` holds that same lock across a 30s call — so
    /// a close arriving during a device-list poll waited for the listing first, on a synchronous
    /// callback that runs on the main thread (#115).
    ///
    /// Driven by holding the connection lock outright, which is the state a slow call leaves it
    /// in, and asserted on time: the guard has to answer while the lock is held by someone else.
    #[test]
    fn the_close_guards_read_answers_while_another_call_holds_the_hosts_connection() {
        use std::time::{Duration, Instant};

        let (silent_addr, _saw_accept) = start_silent_host();
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-wedged", &silent_addr));
        let aimed = host_cameo(&HostId("host-wedged".into()));
        *dev.connected.lock().unwrap() = Some(aimed);

        let conn = dev.host_conn(&HostId("host-wedged".into())).unwrap();
        let held = conn.lock().unwrap();

        let started = Instant::now();
        let status = dev.status_without_dialling();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "the close guard waited on the host's connection lock: {:?}",
            started.elapsed()
        );
        // Nothing has ever been heard from this host, so there is nothing to warn about — and a
        // guard that cannot ask must not invent an active cut either.
        assert!(!status.is_active());
        drop(held);
    }

    /// And it reports what was actually last heard, so a cut that a poll saw start still holds the
    /// window closed after the poll stops.
    #[test]
    fn the_close_guards_read_remembers_the_last_status_a_poll_managed() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let host_id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&host_id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        assert!(!dev.status_without_dialling().is_active(), "nothing has been heard yet");

        dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");
        // The dialog's own poll, which is what fills the cache.
        wait_for_remote(&dev, |s| s.is_active(), "the cut to show up in a poll");

        assert!(
            dev.status_without_dialling().is_active(),
            "a cut a poll saw start must still hold the window closed"
        );
    }

    /// Forgetting one host must not disturb an aim pointed at a different one.
    #[test]
    fn forgetting_an_unrelated_host_leaves_the_aim_alone() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("this host answers and the fingerprint matches");
        dev.add_host(a_paired_host("host-elsewhere", "127.0.0.1:1"));
        dev.connected.lock().unwrap().replace(DeviceInfo { host: Some(id.clone()), ..test_instance() });

        // Forced: this one is unreachable, and an unreachable host is no longer forgettable
        // without it. What the test is about is the *other* host's aim, which must not move.
        dev.forget(&HostId("host-elsewhere".into()), &hosts_path, true).expect("forced");
        assert_eq!(dev.connected.lock().unwrap().as_ref().unwrap().host, Some(id), "a different host's forget must not move the aim");
    }

    // --- dispatch idempotency ---
    //
    // Two failures, pulling opposite ways, and both are the material: a retry after a lost reply
    // that cuts the sheet twice, and a fix so eager that a second sheet can never be cut at all.
    // Every test below holds one of the two ends down.

    /// A TCP relay in front of a loopback host that can swallow one reply. Everything the client
    /// sends still reaches the host, so the Job runs; once armed, the host's answer is dropped
    /// and the connection taken away instead. Waiting for that answer to *exist* before dropping
    /// it is the point — it means the host had already committed the Job when the sender lost it.
    struct ReplyEater {
        addr: String,
        swallow: Arc<std::sync::atomic::AtomicBool>,
    }

    fn start_reply_eater(upstream: &str) -> ReplyEater {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let swallow = Arc::new(AtomicBool::new(false));
        let (upstream, flag) = (upstream.to_string(), swallow.clone());

        std::thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let Ok(server) = std::net::TcpStream::connect(&upstream) else { continue };
                let (mut from_client, mut to_client) = (client.try_clone().unwrap(), client);
                let (mut from_server, mut to_server) = (server.try_clone().unwrap(), server);

                std::thread::spawn(move || {
                    let mut buf = [0u8; 8192];
                    while let Ok(n) = from_client.read(&mut buf) {
                        if n == 0 || to_server.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                });
                let armed = flag.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 8192];
                    while let Ok(n) = from_server.read(&mut buf) {
                        if n == 0 || armed.load(Ordering::SeqCst) {
                            break;
                        }
                        if to_client.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                    let _ = to_client.shutdown(std::net::Shutdown::Both);
                });
            }
        });
        ReplyEater { addr, swallow }
    }

    fn host_cameo(host: &HostId) -> DeviceInfo {
        DeviceInfo {
            instance_id: cut_host::host::testing::CAMEO.into(),
            machine_id: "cameo5".into(),
            transport: TransportKind::Usb { locator: "1:4".into() },
            candidate: false,
            host: Some(host.clone()),
        }
    }

    fn a_square(side: f64) -> Vec<CutPass> {
        vec![CutPass {
            job: Job {
                polylines: vec![vec![
                    geometry::Point { x: 0.0, y: 0.0 },
                    geometry::Point { x: side, y: 0.0 },
                    geometry::Point { x: side, y: side },
                    geometry::Point { x: 0.0, y: 0.0 },
                ]],
                settings: driver_core::Settings::default(),
            },
        }]
    }

    fn cameo_is_active(client: &cut_host::client::HostClient) -> bool {
        client
            .snapshots()
            .map(|snaps| {
                snaps.iter().any(|s| {
                    s.info.instance_id == cut_host::host::testing::CAMEO && s.status.is_active()
                })
            })
            .unwrap_or(false)
    }

    fn wait_until(mut done: impl FnMut() -> bool, complaint: &str) {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(2);
        while !done() {
            assert!(Instant::now() < deadline, "{complaint}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Holds an assertion open rather than sampling once: the host accepts a dispatch before its
    /// cut thread starts, so a single reading can miss a cut that is about to happen.
    fn stays_false(mut claim: impl FnMut() -> bool, complaint: &str) {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_millis(400);
        while Instant::now() < deadline {
            assert!(!claim(), "{complaint}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The finding's own scenario, end to end: the host takes dispatch `d1` and starts cutting,
    /// the reply is lost, the Job finishes, and the operator presses Cut again.
    /// The remote half of the exit. This desktop never opened the Pi's transport, so its own
    /// `disconnect` there is bookkeeping and would leave the cutter exactly as stuck — only the
    /// host's `Reconnect` re-opens it. Driven over the real loopback host so the verb is proved on
    /// the wire, not just in the router. Asserted through `actions`: the phase reads `Idle` on
    /// both sides of the reconnect.
    #[test]
    fn reconnecting_a_remote_cutter_clears_a_cancel_that_could_not_confirm_the_stop() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let host_id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&host_id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");
        // The host's TestDriver parks rather than polls, so nothing can confirm a stop there.
        wait_for_remote(&dev, |s| s.actions.confirm, "the Job to park");
        dev.cancel().expect("cancel");
        wait_for_remote(&dev, |s| s.ended == Some(driver_core::Ended::Cancelled), "the cancel to land");
        assert!(!dev.status().actions.cut, "nothing saw the machine stop, so no Job may follow it");

        dev.reconnect().expect("the host re-opens its own cutter");
        assert!(dev.status().actions.cut, "a re-opened cutter takes a Job again");
    }

    fn wait_for_remote(dev: &DeviceManagerHandle, want: impl Fn(&CutStatus) -> bool, what: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let s = dev.status();
            if want(&s) {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "waited out {what}, sat at {:?}", s.phase);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn a_retry_after_a_lost_reply_does_not_cut_the_material_twice() {
        use std::sync::atomic::Ordering;

        let host = start_loopback_host();
        let eater = start_reply_eater(&host.addr);
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let host_id = dev
            .pair("Pi".into(), eater.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("the relay forwards the pairing check to a host that answers");
        // Establishes the desktop's own connection through the relay first, so arming it takes
        // away a dispatch reply rather than a piece of the TLS handshake.
        dev.list_devices();

        let aimed = host_cameo(&host_id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        eater.swallow.store(true, Ordering::SeqCst);
        let err = dev
            .execute_cut(aimed.clone(), a_square(10.0))
            .expect_err("the host's answer never came back");
        assert_eq!(err.code, "dispatch_unconfirmed", "{}", err.message);

        // Straight to the host, around the relay: the Job really did land, and really did run.
        let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
        wait_until(|| cameo_is_active(&direct), "the dispatch never reached the host");
        direct.confirm_pass_done(cut_host::host::testing::CAMEO).unwrap();
        wait_until(|| !cameo_is_active(&direct), "the job never finished");

        // Connectivity is back and the operator presses Cut on the same design.
        eater.swallow.store(false, Ordering::SeqCst);
        let retry = dev.execute_cut(aimed, a_square(10.0)).expect("the retry reaches the host");

        stays_false(|| cameo_is_active(&direct), "the same material was cut a second time");
        // And the operator is told which of the two things happened. `Ok` alone said "your Job has
        // started" about a dispatch that started nothing, in front of a cutter that never moved.
        assert!(retry.duplicate, "a Job the host had already accepted must say so");
    }

    /// The other end: the fix must not make a second sheet impossible. Same design, same cutter,
    /// dispatched again after the first one was answered and finished — and the blade moves.
    #[test]
    fn cutting_the_same_design_again_after_an_answered_dispatch_starts_a_new_job() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let host_id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("this host answers and the fingerprint matches");
        let aimed = host_cameo(&host_id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();

        dev.execute_cut(aimed.clone(), a_square(10.0)).expect("first sheet");
        wait_until(|| cameo_is_active(&direct), "the first cut never started");
        direct.confirm_pass_done(cut_host::host::testing::CAMEO).unwrap();
        wait_until(|| !cameo_is_active(&direct), "the first cut never finished");

        // Fresh material, same file: a legitimate second Job, not a retry of anything.
        let second = dev.execute_cut(aimed, a_square(10.0)).expect("second sheet");
        assert!(!second.duplicate, "a deliberate re-cut is not a duplicate");
        wait_until(|| cameo_is_active(&direct), "the second sheet was refused as a duplicate");
    }

    /// The headline verb of the whole feature, driven from the desktop side: the Passes reach the
    /// host intact and the Job it starts is the one that was planned. Every other routed verb had
    /// a test against a loopback host; this one had none (#108).
    ///
    /// The count is asserted through the host's own `pass` position rather than by inspecting what
    /// the mock Driver encoded: a dispatch that arrived with a Pass missing would park at
    /// `1 of 1` here, which is exactly the silent loss worth catching.
    #[test]
    fn a_remote_dispatch_carries_every_pass_and_starts_the_job_it_planned() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let host_id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("this host answers and the fingerprint matches");
        let aimed = host_cameo(&host_id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        let two_passes = [a_square(10.0), a_square(20.0)].concat();
        let started = dev.execute_cut(aimed, two_passes).expect("dispatch");
        assert!(!started.duplicate, "nothing was in doubt, so this is a new Job");

        let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
        wait_until(|| cameo_is_active(&direct), "the dispatch never reached the host");

        let snap = direct
            .snapshots()
            .unwrap()
            .into_iter()
            .find(|s| s.info.instance_id == cut_host::host::testing::CAMEO)
            .expect("the host knows this cutter");
        assert_eq!(
            snap.status.pass.map(|p| p.total),
            Some(2),
            "the host is running a different number of Passes than were sent"
        );
        assert!(snap.job_id.is_some(), "and it registered a Job for them");
    }

    /// Two presses of Cut on one Job cannot mint two ids. Choosing an id and recording it used to
    /// be separate steps, and Start Cut stays enabled until a poll moves the status — so two
    /// presses could both find nothing and mint one each. If the first reached the host and lost
    /// its reply, the second's entry replaced the only record of it, and the retry that should
    /// have been recognised went out under a name the host had never seen.
    #[test]
    fn two_presses_of_cut_cannot_mint_two_ids_for_one_job() {
        let dev = test_device_setup();
        let key = key_for(&a_square(10.0));

        let (first, reserved_first) = dev.reserve_dispatch_id(&key);
        let (second, reserved_second) = dev.reserve_dispatch_id(&key);
        assert_eq!(first, second, "the second press must join the first, not race it");
        assert!(reserved_first, "the first press is what recorded it");
        assert!(!reserved_second, "and the second must know it did not");
    }

    fn key_for(passes: &[CutPass]) -> JobKey {
        JobKey {
            host: HostId("host-1".into()),
            device: cut_host::host::testing::CAMEO.into(),
            digest: job_digest("cameo5", passes),
        }
    }

    #[test]
    fn two_different_jobs_to_one_cutter_get_different_ids() {
        let dev = test_device_setup();
        assert_ne!(
            dev.reserve_dispatch_id(&key_for(&a_square(10.0))).0,
            dev.reserve_dispatch_id(&key_for(&a_square(20.0))).0,
            "different geometry is a different Job"
        );

        // Settings, not only geometry: the same shape cut at a different force is a different
        // Job, and a digest that missed it would let one be swallowed as a retry of the other.
        let mut harder = a_square(10.0);
        harder[0].job.settings.force = Some(driver_core::Settings::default().force.unwrap_or(0) + 1);
        assert_ne!(job_digest("cameo5", &a_square(10.0)), job_digest("cameo5", &harder));

        // And the same Passes aimed at a different machine are not interchangeable either.
        assert_ne!(job_digest("cameo5", &a_square(10.0)), job_digest("puma", &a_square(10.0)));
    }

    /// The whole distinction, in the one place it lives. Flip either half and this fails.
    #[test]
    fn an_id_is_reused_only_while_its_dispatch_has_no_answer() {
        let dev = test_device_setup();
        let key = key_for(&a_square(10.0));

        // One dispatch outstanding: the next press of Cut is that dispatch, tried again.
        let (first, _) = dev.reserve_dispatch_id(&key);
        assert_eq!(
            dev.reserve_dispatch_id(&key).0,
            first,
            "a retry must arrive under the id it went out with"
        );

        // ...and only for that Job. A different one is never mistaken for the retry.
        assert_ne!(dev.reserve_dispatch_id(&key_for(&a_square(20.0))).0, first);

        // The answer arrives, so nothing is in doubt any more — and a deliberate re-cut of the
        // same design is its own Job, not a duplicate of the one that finished.
        dev.in_doubt.lock().unwrap().remove(&key);
        assert_ne!(dev.reserve_dispatch_id(&key).0, first, "a re-cut must not be swallowed");
    }

    /// A cutter enumerated by path can be most of a `/dev/serial/by-id/...` string, and the host
    /// refuses a dispatch id longer than it will remember — which would have refused *every*
    /// dispatch to that cutter, permanently, for a reason nothing in the message points at.
    #[test]
    fn a_dispatch_id_fits_what_a_host_will_accept_even_for_a_long_cutter_id() {
        let dev = test_device_setup();
        let key = JobKey {
            host: HostId("host-1".into()),
            device: "serial:at:/dev/serial/by-id/usb-FTDI_FT232R_USB_UART_A50285BI-if00-port0".into(),
            digest: job_digest("puma", &a_square(10.0)),
        };
        let (id, _) = dev.reserve_dispatch_id(&key);
        assert!(
            id.0.len() <= 128,
            "the host refuses ids over 128 characters; this one is {}: {}",
            id.0.len(),
            id.0
        );
        // Still distinguishing two Jobs on that cutter — the truncation drops the part the digest
        // already covers, not the part that tells Jobs apart.
        let other = JobKey { digest: job_digest("puma", &a_square(20.0)), ..key.clone() };
        assert_ne!(dev.reserve_dispatch_id(&other).0, id);
    }

    /// Entries clear on any answer and expire with the host's memory, so this is a backstop rather
    /// than a policy — but without it a session dispatching thousands of distinct designs at hosts
    /// that never answer grows a map nothing ever prunes.
    #[test]
    fn jobs_held_in_doubt_are_bounded() {
        let dev = test_device_setup();
        for n in 0..MAX_JOBS_IN_DOUBT + 20 {
            dev.reserve_dispatch_id(&JobKey {
                host: HostId("host-1".into()),
                device: cut_host::host::testing::CAMEO.into(),
                digest: n as u64,
            });
        }
        assert!(
            dev.in_doubt.lock().unwrap().len() <= MAX_JOBS_IN_DOUBT,
            "in doubt: {}",
            dev.in_doubt.lock().unwrap().len()
        );
    }

    /// A press that never reached a host keeps its reservation. Undoing it would need this call to
    /// know it owns the entry, and it cannot — a concurrent press of Cut on the same Job is using
    /// that same id. Keeping it costs nothing: a dispatch no host received is one no host can
    /// deduplicate, so the next Cut under that id is cut normally.
    #[test]
    fn a_press_that_never_reached_a_host_keeps_its_id_for_the_next_one() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        let aimed = host_cameo(&HostId("host-1".into()));
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        let err = dev.execute_cut(aimed, a_square(10.0)).expect_err("nothing answers this host");
        // A first press against an offline Pi must not claim the Job may be cutting there.
        assert_eq!(err.code, "host_unreachable", "got {err:?}");

        let key = JobKey {
            host: HostId("host-1".into()),
            device: cut_host::host::testing::CAMEO.into(),
            digest: job_digest("cameo5", &a_square(10.0)),
        };
        assert!(!dev.reserve_dispatch_id(&key).1, "the next press joins the id already reserved");
    }

    /// An abandoned entry is deliberately *kept*, not aged out. Time cannot show that the operator
    /// loaded fresh material, so expiring an unanswered dispatch mints a new id for a Job the host
    /// still remembers under the old one — and cuts the material a second time. What makes it
    /// harmless is that the next press is told it was read as a retry and the answer clears the
    /// entry, so pressing Cut again really does cut (#121).
    #[test]
    fn an_unanswered_dispatch_stays_the_retry_until_something_answers_it() {
        let dev = test_device_setup();
        let key = key_for(&a_square(10.0));
        let (abandoned, _) = dev.reserve_dispatch_id(&key);

        // However long nobody comes back to it.
        assert_eq!(dev.reserve_dispatch_id(&key).0, abandoned);
        assert_eq!(dev.in_doubt.lock().unwrap().len(), 1);
    }

    /// A dispatch that never reached a host drops the reservation *it* made, and only that one:
    /// an earlier unanswered dispatch's entry is what a later retry still needs, and clearing it
    /// here would send that retry out under a name the host has never seen.
    #[test]
    fn a_dispatch_that_never_reached_a_host_leaves_an_earlier_one_in_doubt() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        let aimed = host_cameo(&HostId("host-1".into()));
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        // An earlier dispatch of this Job is outstanding.
        let key = JobKey {
            host: HostId("host-1".into()),
            device: aimed.instance_id.clone(),
            digest: job_digest(&aimed.machine_id, &a_square(10.0)),
        };
        let (earlier, _) = dev.reserve_dispatch_id(&key);

        // Nothing is listening on this address, so the dispatch never gets a connection.
        let err = dev.execute_cut(aimed, a_square(10.0)).expect_err("nothing answers this host");
        // Still `dispatch_unconfirmed` rather than a plain refusal, and rightly: the earlier
        // dispatch of this Job was never answered, so it may be cutting there right now.
        assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
        assert_eq!(
            dev.in_doubt.lock().unwrap().get(&key).map(|(id, _)| id.clone()),
            Some(earlier),
            "a call that never reached the host discarded a previous dispatch's id"
        );
    }

    /// The window-close guard has to see a dispatch that has not been polled yet. The newest
    /// status anyone holds at that moment is the `Idle` from before the cut started, so a guard
    /// reading only the cache waves the operator past the cut they just pressed Cut for.
    #[test]
    fn a_dispatch_nothing_has_polled_yet_still_holds_the_window() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let host_id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&host_id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        // A poll before the cut, which is what fills the cache with `Idle`.
        assert!(!dev.status().is_active());
        assert!(!dev.a_cut_may_be_running(), "nothing has been dispatched yet");

        dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");
        // Deliberately no poll in between: this is the window the cache cannot answer for.
        assert!(
            dev.a_cut_may_be_running(),
            "the guard waved through a cut this desktop had just started"
        );

        // And it stops holding the window once the cutter says it would take another Job.
        let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
        wait_until(|| cameo_is_active(&direct), "the cut never started");
        direct.confirm_pass_done(cut_host::host::testing::CAMEO).unwrap();
        wait_until(|| !cameo_is_active(&direct), "the cut never finished");
        wait_until(
            || {
                dev.status();
                !dev.a_cut_may_be_running()
            },
            "a finished cut still held the window closed",
        );
    }
}
