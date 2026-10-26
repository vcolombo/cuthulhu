// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use cut_host::client::HostClient;
use cutplan::presets::{
    default_presets_path, load_presets, resolve_settings, save_user_presets, MaterialPreset,
    SettingsOverride,
};
use cutplan::{plan_cut, plan_passes_for, DocumentPass, CutError, Grouping, PassKey, PassSelection, PlanOptions};
use cutplan::weed::WeedOptions;
use driver_core::manager::{CutPass, DeviceEvent, DeviceManager};
use driver_core::{CutStatus, DeviceBackendFactory, DeviceInfo, HostId, MachineCaps};
use serde::{Deserialize, Serialize};

use crate::hosts::PairedHost;
use crate::state::AppState;

mod close_gate;
mod ledger;
mod window_events;

pub use window_events::{on_close_requested, on_page_load};

use close_gate::{CloseGate, SendPermit};
use ledger::Ledger;
use ledger::JobKey;
#[cfg(test)]
use ledger::{AttemptId, MAX_JOBS_IN_DOUBT};

#[derive(Clone, Debug, Serialize)]
pub struct IpcError {
    pub code: String,
    pub message: String,
    /// Set only for `host_protocol`. Skipped on the wire: a thrown error is still
    /// `{ code, message }`. The host row carries this, because that is what the chip
    /// and the update link branch on.
    #[serde(skip)]
    pub protocol_side: Option<cut_host::client::ProtocolSide>,
}

impl IpcError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        IpcError { code: code.into(), message: message.into(), protocol_side: None }
    }
}

/// A device fault as the UI's own currency, in one place instead of at every call site.
///
/// Eight sites wrote `IpcError::new("device_error", format!("{e:?}"))`, so every variant arrived
/// as one code with the discriminant surviving only inside a `Debug` string — a cable pull and a
/// verb issued at the wrong moment were the same failure to anything reading the code, and
/// distinguishable to a human only by the Rust value in the message. The conversion lives here
/// so the call sites are a bare `?`, which is what makes a ninth hand-written one impossible to
/// add by accident (#73).
///
/// The code and the sentence are both `driver-core`'s: the CLI, the Cut Host and this desktop
/// name the same fault the same way, and adding a variant means editing one match, there.
impl From<driver_core::manager::DeviceError> for IpcError {
    fn from(e: driver_core::manager::DeviceError) -> Self {
        IpcError::new(e.code(), e.to_string())
    }
}

/// A presets-file refusal as the UI's own currency, in one place instead of at every call site.
///
/// Five sites wrote `IpcError::new("preset_error", format!("{e:?}"))`, so a missing header on the
/// file told the operator `Corrupt("missing or invalid version field")` — the sentence the code
/// wrote, wrapped in a struct literal, in quotes — and a permission problem told them
/// `Io("Permission denied (os error 13)")`. One code covered all of it, so nothing could tell a
/// file this build is too old to read from one that is damaged without parsing the Rust value in
/// the message (#278).
///
/// The code and the sentence are both `cutplan`'s, declared beside `load_presets` and
/// `save_user_presets` which raise them, so adding a variant means editing one match, there.
impl From<cutplan::presets::PresetError> for IpcError {
    fn from(e: cutplan::presets::PresetError) -> Self {
        IpcError::new(e.code(), e.to_string())
    }
}

/// A preferences-file refusal as the UI's own currency. The code and the sentence are
/// `preferences`'s, so a version this build cannot read and a damaged file stay distinct
/// (#243) instead of arriving as one `Debug` string.
impl From<crate::preferences::PreferencesError> for IpcError {
    fn from(e: crate::preferences::PreferencesError) -> Self {
        IpcError::new(e.code(), e.to_string())
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
        // A refusal that is the *cutter's* is reported as that fault, not as the host's: the
        // same jam, cable pull or wrong-moment verb must not read differently depending on
        // whether the cutter hangs off this laptop or off a Pi. The other refusals are the
        // host's own and keep its code.
        ClientError::Refused(cut_host::protocol::Refusal::Device(fault)) => fault.code(),
        // The host was reached, understood the request, and refused it.
        ClientError::Refused(_) => "host_refused",
        // The host answered, so its reachability is settled — the answer is the proof. What it
        // answered was a reply this request cannot use, which nothing about the network or the
        // pairing will change, and `host_unreachable` sent the operator to check both (#283).
        //
        // One code for both, because a code is what the operator must *do* about it and both ask
        // the same: the peer on that port does not follow this protocol, so neither the network nor
        // the pairing is the thing to change. What the two prove differs, and the sentence carries
        // that difference; a fifth code would sort them for a UI that has never branched on
        // either (#285).
        ClientError::WrongReply { .. } | ClientError::WrongDispatch => "host_wrong_reply",
        // The code is the action: update one of the two programs, which neither the network
        // nor a re-pair will do. Which program is `protocol_side`, recorded here from the
        // same comparison the sentence uses, so the row does not have to parse the sentence.
        ClientError::Protocol { .. } => "host_protocol",
        ClientError::Transport(_) => "host_unreachable",
    };
    let mut err = IpcError::new(code, e.to_string());
    err.protocol_side = e.protocol_side();
    err
}

/// Whether this failure left the socket unsafe to reuse.
///
/// A transport break, a reply this request cannot use, and an `Accepted` naming some other
/// dispatch can all leave a frame on the connection that the next verb would read as its own
/// answer. `on_host`'s callers drop the client on these and redial. A refusal does not: the host
/// answered about this request, and the framing survived it.
fn connection_died(e: &cut_host::client::ClientError) -> bool {
    matches!(
        e,
        cut_host::client::ClientError::Transport(_)
            | cut_host::client::ClientError::WrongReply { .. }
            | cut_host::client::ClientError::WrongDispatch
            // A version is agreed once, at connect, which never stores a client. The arm is
            // here so a mismatch that did surface mid-call cannot be retried on the same socket.
            | cut_host::client::ClientError::Protocol { .. }
    )
}

/// What an operator is told when a dispatch failed with something of this Job still outstanding.
///
/// Not a promise that pressing Cut again cannot cut twice, which is what this said until #286.
/// Nothing on this side can know that: a Cut Host forgets an accepted id past its retention and
/// past its capacity cap, and `in_doubt` is process-local, so a desktop restarted between the two
/// presses has no id left to reuse at all. What is true is narrower, and is the whole of the
/// protection — the retry goes out under the id the first attempt used, and a host that still
/// remembers that id reads it as this Job rather than starting a second one. Which of the two
/// happened is a fact about the cutter, so the cutter is where it is answered.
///
/// The bounds are not enumerated. They are why the promise cannot be made, not something an
/// operator standing over a cutter can act on.
///
/// `retry_holds_the_id` is asked rather than assumed, because the failure being reported is not
/// what decides it. Two ways this side has nothing left to retry under while something is still
/// outstanding: an answer that settled *this* attempt clears the entry and leaves an earlier one
/// unaccounted for (#288), and an entry this call was reusing can age past the point where reusing
/// it protects anything while the call is still in flight. Either way the next press is a new Job,
/// and saying otherwise is the promise this stopped making — see `retry_would_reuse`.
fn unconfirmed(failure: &IpcError, retry_holds_the_id: bool) -> IpcError {
    IpcError::new(
        "dispatch_unconfirmed",
        format!(
            "{} — the Job may already be cutting there, and only the cutter can tell you. {}",
            failure.message,
            if retry_holds_the_id {
                "Press Cut again to send it under the same id: a host that still remembers that id \
                 reads it as this Job rather than starting a second one."
            } else {
                "Pressing Cut again dispatches it as a new Job rather than a retry, so check the \
                 cutter first."
            }
        ),
    )
}

/// How long "quit anyway" waits for a press that is already sending.
///
/// Long enough to cover a request in flight to a Pi on the same network, short enough that the
/// answer to a prompt is not a spinner: past it the send may land, which the prompt already says.
const CLOSE_GATE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// What one press did once it held the host's connection.
///
/// One value, because the alternative was a flag for "the closure ran" and another for "the answer
/// settled", read back beside the bool that said whether this call wrote the in-doubt entry. Those
/// three could be edited apart, and the empty arm that used to sit between the send and this match
/// was the result. `NotSent` is not a failure of the host or the network: a close was committed
/// before this press reached the host, and a Job started now would keep cutting with nothing left
/// to watch it (#158). `Failed` is the operator-facing error, already plain or unconfirmed.
enum Sent {
    Admitted { duplicate: bool },
    NotSent,
    Failed(IpcError),
}

/// What a Cut Host's snapshot says about one cutter, in the four states this desktop treats
/// differently.
///
/// One value rather than a pile of predicates, because the differences are decisions: which states
/// let a poll clear a dispatch mark, which refuse a forget, and which of those refusals `force` may
/// pass. Spelled as guards instead, the second predicate's meaning came from the arm above it and
/// the third was a double negative — and the third's refusal reached the webview under a code that
/// hid the escape it named.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CutterState {
    /// Nothing claims it and it says it would take another Job. The only state a mark clears on.
    Free,
    /// A Job in flight, or a dispatch admitted and on its way to the manager. `claimed` is the
    /// half of this that `actions` cannot describe (#158).
    Claimed,
    /// Stopped where nothing saw it stop. The host's own `Reconnect` clears it, so a refusal here
    /// has a verb one click away and is not something to force past: the blade may still be moving
    /// and the token is the only thing that could stop it.
    Stalled,
    /// Disconnected or faulted. The host's `Reconnect` may still fix it — `Command::Connect`
    /// accepts an errored cutter — but it may equally be hardware that is gone, which answers every
    /// reconnect with the same fault and has no cut to cancel. Nothing in a snapshot tells those
    /// two apart, so this is the one refusal `force` has to be able to pass: otherwise the host row
    /// could never be removed.
    Unreachable,
}

fn classify(snapshot: &cut_host::protocol::DeviceSnapshot) -> CutterState {
    if snapshot.claimed {
        CutterState::Claimed
    } else if snapshot.status.actions.cut {
        CutterState::Free
    } else if snapshot.status.ended == Some(driver_core::Ended::Cancelled) {
        CutterState::Stalled
    } else {
        CutterState::Unreachable
    }
}

/// The state that decides for the whole host: the worst any of its cutters is in.
///
/// The order is `severity`'s match rather than a list here, so a state added later cannot
/// aggregate as `Free` — which would make a host `forget` must refuse silently forgettable, with
/// the mark that raises the quit prompt discarded on the way out.
fn worst_state(snapshots: &[cut_host::protocol::DeviceSnapshot]) -> CutterState {
    snapshots.iter().map(classify).max_by_key(severity).unwrap_or(CutterState::Free)
}

fn severity(state: &CutterState) -> u8 {
    match state {
        CutterState::Free => 0,
        CutterState::Unreachable => 1,
        CutterState::Stalled => 2,
        CutterState::Claimed => 3,
    }
}

/// Whether a poll may clear this cutter's dispatch marks: only when nothing claims it and its own
/// status says it would take a Job. Either half alone clears the mark of a blade that may be moving.
fn snapshot_is_free(snapshot: &cut_host::protocol::DeviceSnapshot) -> bool {
    classify(snapshot) == CutterState::Free
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// What a press is told when the window closed before it could start anything.
const CLOSED_MID_PRESS: &str =
    "the app is closing, so this cut was not sent — start it again once it is open";

/// What a request to leave the app comes to.
#[derive(Debug, PartialEq, Eq)]
pub enum CloseVerdict {
    /// The app may go: either nothing is outstanding or the guard could not warn, and the gate
    /// is committed so no press starts anything new.
    Proceed,
    /// A cut may be running and the operator has been asked; the app stays open until the page
    /// answers through `force_quit`.
    Warn,
}


#[derive(Deserialize)]
pub struct CutRequest {
    pub device_instance_id: String,
    pub doc_revision: String,
    /// How the dialog grouped the passes it is naming. Sent rather than remembered: the plan,
    /// the travel and the cut are three round trips, and a mode kept in `AppState` could be
    /// changed between them while the stale-plan check only guards the document.
    pub grouping: Grouping,
    /// The weed options the dialog planned with, sent for the same reason as `grouping`: rows
    /// previewed with one border and cut with another would send geometry nobody saw.
    pub weed: Option<WeedOptions>,
    pub passes: Vec<ConfiguredPassDto>,
}

#[derive(Deserialize)]
pub struct ConfiguredPassDto {
    pub key: PassKey,
    pub enabled: bool,
    pub preset_id: Option<String>,
    pub speed: Option<u32>,
    pub force: Option<u32>,
    pub repeat_count: Option<u32>,
}

/// Whether a lifted host connection may still serve a call.
///
/// `Forgetting` refuses new calls while `hosts.json` is written without holding the host-control
/// mutex across filesystem I/O. A failed save returns it to `Active`; a successful one advances it
/// to `Forgotten` before the map entry is removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostLifecycle {
    Active,
    Forgetting,
    Forgotten,
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
    /// Unix millis of the last snapshot this host actually returned.
    ///
    /// Left alone when a dial fails, so an offline card can say when the cutters were last real
    /// rather than when the failure was noticed.
    pub last_seen: Option<u64>,
    /// The state a call lifted out of the paired-host map must still check.
    ///
    /// A press lifts this connection out before it queues on the mutex, so removing the map entry
    /// cannot stop one already waiting: it would acquire a live `Arc`, dispatch, and start a Job on
    /// a host whose token has just been discarded — with no cancel route, and a mark no poll can
    /// clear because that poll is a call to the host that was removed. The mutex is the one thing
    /// both sides pass through, so the answer lives here.
    lifecycle: HostLifecycle,
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
    /// The last thing that went wrong with this host, or `None` while nothing has. The name is the
    /// serialized one this field has always had. The row prints it. The chip and the update
    /// link do not read it: which program to replace is `protocol_side`, because a rewording
    /// of this sentence must not drop them.
    pub unreachable: Option<String>,
    /// Which program a protocol mismatch names. `None` for every other failure, including a
    /// host that timed out.
    pub protocol_side: Option<cut_host::client::ProtocolSide>,
    /// Unix millis of the last successful snapshot, or `None` until one has landed.
    /// A failed dial leaves the previous value, so the row keeps when the host was last heard.
    pub last_seen: Option<u64>,
}

/// One remote cutter's last reported status, read from the cache without dialling.
///
/// Fields stay snake_case like `DeviceInfo`. The status inside keeps `CutStatus`'s camelCase.
#[derive(Clone, Debug, Serialize)]
pub struct CutterStatusView {
    pub host: HostId,
    pub instance_id: String,
    pub status: CutStatus,
}

/// Which cutter a command names when the caller does not want the aim.
enum CutterAddress {
    /// Both arguments omitted: the connected cutter, including "no aim" for a local cancel.
    Aim,
    /// An explicit cutter on this computer. Local multi-cutter is out of scope (#59), so this
    /// has to be the one already aimed.
    Local { instance_id: String },
    /// A cutter on a paired Cut Host, whether or not it is the aim (#307).
    Remote { host: HostId, instance_id: String },
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
    /// Serializes a `pair` or `forget` rewriting `hosts.json` from a snapshot.
    ///
    /// Each of those verbs writes the whole file. A forget publishes `Forgetting` and drops the
    /// connection before the write, so a snapshot taken in that window still finds the host in
    /// the map and can put the row back, or land on top of a pairing that arrived during the
    /// write. Held only around that snapshot and the save, and never under a `HostConnection`:
    /// `cancel` and a dispatch do not take it.
    hosts_persist: Mutex<()>,
    /// Every HostId minted or loaded in this process, including hosts since forgotten.
    ///
    /// A forgotten host can still have a close-guard mark. `forget` refuses rather than discard
    /// the token when a press lands after the idle answer, but `remove_host` and a retraction
    /// that names only some presses can still leave one. Reusing that HostId for a new Pi wired
    /// at the same socket would give both the same `(HostId, instance_id)` key, and an idle poll
    /// of the new Pi would erase the old one's warning. Ids therefore remain claimed for the
    /// process lifetime, which is also the lifetime of those marks.
    claimed_host_ids: Mutex<HashSet<HostId>>,
    /// In-doubt ids, the presses using them, and the marks that hold the window.
    /// The only code that locks those maps lives on `Ledger`.
    ledger: Ledger,
    /// The last status each remote cutter actually reported, so something can be said about one
    /// without dialling it.
    last_remote_status: Mutex<HashMap<(HostId, String), CutStatus>>,
    /// Whether the webview has installed the one listener that makes a prevented close escapable.
    ///
    /// `emit` succeeding says only that Tauri serialized the event; it also returns success with
    /// zero listeners. The listener sets this after registration and clears it before teardown, so
    /// the native handler never refuses a close on the strength of an event nobody can receive.
    close_guard_ready: std::sync::atomic::AtomicBool,
    /// The one barrier between a press starting a cut and a window that has been let go.
    close_gate: CloseGate,
    /// How many callers are between asking for a host connection and holding it. Test-only: it is
    /// the difference between a press inside `on_host` at that lock and one merely registered.
    #[cfg(test)]
    on_host_waiting: std::sync::atomic::AtomicUsize,
    pub connected: Mutex<Option<DeviceInfo>>,
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
            hosts_persist: Mutex::new(()),
            claimed_host_ids: Mutex::new(HashSet::new()),
            ledger: Ledger::default(),
            last_remote_status: Mutex::new(HashMap::new()),
            close_gate: CloseGate::default(),
            #[cfg(test)]
            on_host_waiting: std::sync::atomic::AtomicUsize::new(0),
            close_guard_ready: std::sync::atomic::AtomicBool::new(false),
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
    ///
    /// Asks for snapshots rather than the bare device list, because this is the only call that
    /// hears from *every* cutter this desktop can reach: `status` polls the aimed one, and the
    /// close guard asks about all of them (#158). Without this a Job dispatched to a Pi the
    /// operator then aimed away from held the window for the rest of the session, with no verb that
    /// could ever release it. The cut dialog runs this once a second while a host is paired, which
    /// is where an operator watching a remote cut finish already is.
    pub fn list_devices(&self) -> Vec<DeviceInfo> {
        let mut all = self.local_factory.list_devices();
        for (id, conn) in self.host_conns() {
            let mut guard = conn.lock().unwrap();
            let host = &mut *guard;
            // `forget` has published `Forgetting` and released this lock so the save does not pin
            // it. Dialling anyway would reopen a host the operator is removing; the `Arc` this
            // call already holds outlives the map entry until the call drops it.
            if host.lifecycle != HostLifecycle::Active {
                continue;
            }
            let Some(client) = host.ensure() else { continue };
            // Same-host dispatches cannot overlap this call: both hold this connection. A mark is
            // therefore written after this snapshot or the snapshot reports after the request.
            match client.snapshots() {
                Ok(snapshots) => {
                    // Both facts are the cutter's authority for "nothing of ours is running":
                    // status says it would take a Job, and `claimed` closes the admitted-
                    // dispatch-before-worker-publication gap status cannot represent.
                    // Every cutter is kept, not only the aimed one: a second machine on the
                    // same Pi used to vanish here (#307).
                    self.remember_snapshots(&id, &snapshots);
                    host.last_seen = Some(unix_millis());
                    let listed = snapshots.into_iter().map(|s| s.info).collect();
                    all.extend(crate::hosts::stamp_host(&id, listed));
                }
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
        let conn = HostConnection {
            paired,
            client: None,
            last_error: None,
            last_seen: None,
            lifecycle: HostLifecycle::Active,
        };
        self.claimed_host_ids.lock().unwrap().insert(id.clone());
        self.hosts.lock().unwrap().insert(id, Arc::new(Mutex::new(conn)));
    }

    /// Mint a HostId that no host or surviving mark in this process has ever meant.
    fn claim_next_host_id(&self) -> Result<HostId, IpcError> {
        let mut claimed = self.claimed_host_ids.lock().unwrap();
        let highest = claimed
            .iter()
            .filter_map(|id| id.0.strip_prefix("host-"))
            .filter_map(|n| n.parse::<u128>().ok())
            .max()
            .unwrap_or(0);
        let next = highest.checked_add(1).ok_or_else(|| {
            IpcError::new("host_id_exhausted", "no numeric Cut Host id remains available")
        })?;
        let id = HostId(format!("host-{next}"));
        claimed.insert(id.clone());
        Ok(id)
    }

    /// Drops the host and the status its cutters last reported.
    ///
    /// The cache goes because nothing can ever read it again: `claimed_host_ids` only grows, so a
    /// forgotten id is never minted a second time in this process, and the cache is keyed by it.
    /// Keeping it would be a map that only grows across a session of pairing and forgetting.
    ///
    /// The dispatch marks are *not* dropped here, because which of them are stale is not something
    /// this verb knows: `forget` asks the host and retracts the marks that answer covered, which
    /// leaves a dispatch accepted after the answer still holding the window (see `retract_marks`).
    /// The HostId staying claimed is what keeps that mark's `(HostId, instance_id)` key its own.
    /// In-doubt ids stay too, and not because anything can name them again — nothing can. They are
    /// keyed by `JobKey` and bounded by expiry and the capacity cap rather than by host, so
    /// filtering them here would be a third pruning rule for entries that are already dead. Their
    /// only cost is a cap slot until `ID_RETENTION`.
    pub fn remove_host(&self, id: &HostId) {
        self.hosts.lock().unwrap().remove(id);
        self.last_remote_status.lock().unwrap().retain(|(host, _), _| host != id);
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
                    protocol_side: h.last_error.as_ref().and_then(|e| e.protocol_side),
                    last_seen: h.last_seen,
                }
            })
            .collect()
    }

    /// Every paired host as saved, for `pair`/`forget` to re-derive `hosts.json`'s on-disk
    /// contents and to mint the next id against.
    pub(crate) fn paired_hosts(&self) -> Vec<PairedHost> {
        self.host_conns()
            .into_iter()
            .filter_map(|(_, conn)| {
                let h = conn.lock().unwrap();
                // A host mid-forget is still in the map so queued calls can see `Forgetting` and
                // refuse. It is not a row to write back: its own save has already dropped it, and
                // a concurrent `pair` or `forget` rewriting the file would resurrect it.
                (h.lifecycle == HostLifecycle::Active).then(|| h.paired.clone())
            })
            .collect()
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

        // Claimed separately from the paired map because a forgotten host can leave a close-guard
        // mark behind. Reusing its id would let an idle cutter on a new Pi erase that warning.
        // The snapshot and the save are one critical section with `forget`: each rewrites the
        // whole file, and interleaving the two puts a row back or drops one that just landed.
        let _persist = self.hosts_persist.lock().unwrap();
        let mut prospective = self.paired_hosts();
        let id = self.claim_next_host_id()?;
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
    /// Four refusals over three codes, because the operator's next move differs: cancel the cut,
    /// reconnect the cutter, forget past a cutter nothing can recover, or decide whether a host
    /// that cannot be asked is really idle. The last two carry codes of their own because the
    /// dialog offers the force from the code.
    ///
    /// `force` skips those two and nothing else. A host that answers "busy" — cutting, or stopped
    /// where nothing saw it stop — is reachable, so `cancel` or the host's own `Reconnect` still
    /// works there, and there is no case for cutting a route while it works.
    pub fn forget(&self, id: &HostId, hosts_path: &Path, force: bool) -> Result<(), IpcError> {
        self.forget_with_save(id, force, |prospective| crate::hosts::save(hosts_path, prospective))
    }

    /// The same transition with the persistence boundary injected, so its lock/state ordering can
    /// be asserted without making a real filesystem write stall on demand.
    fn forget_with_save(
        &self,
        id: &HostId,
        force: bool,
        save: impl FnOnce(&[PairedHost]) -> Result<(), crate::hosts::HostsError>,
    ) -> Result<(), IpcError> {
        // Bounded the same way `status()` is (`STATUS_POLL_TIMEOUT`, not the full
        // `DEFAULT_BODY_TIMEOUT`) — on *both* legs, the reconnect and the body read. Not because
        // anything else waits on it: this holds only this host's connection lock, so a stall here
        // delays calls aimed at this host and nothing else. It is bounded because the operator is
        // usually forgetting a host *precisely* because it has stopped answering, and making them
        // wait 30s before they can even be offered the force is the wrong answer to that.
        //
        // The marks are read *inside* that connection, immediately before the question. A press
        // writes its mark under the same connection, so one captured here is one this answer is
        // about. A press that gets the connection after the answer is not: the re-check below
        // refuses the forget, and the token stays. Discarding it would drop the only route to a
        // Job this answer never saw.
        //
        // A press that is already inside its dispatch cannot be stopped by anything here: it owns
        // the connection, and the `forgotten` flag below is only read on the way in. So the one
        // press this verb cannot answer for is refused rather than raced — the operator's next
        // click gets the real answer a moment later, and the alternative is a Job started on a host
        // whose token this call discarded.
        if self.ledger.a_press_is_dispatching_to(id) {
            return Err(IpcError::new(
                "host_busy",
                "a cut is being sent to this host right now; forget it once that press has finished",
            ));
        }
        // Read once more before the question for the forced path below, which has no answer to
        // scope anything by: a press that marks *while* the question is in flight is not covered by
        // a silence that cannot speak for it, and re-reading afterwards would retract the only
        // warning that press left behind.
        let before_asking = self.ledger.marks_for(id);
        let asked = self.with_host_within(id, STATUS_POLL_TIMEOUT, |c| {
            let marks = self.ledger.marks_for(id);
            Ok((marks, c.snapshots_within(STATUS_POLL_TIMEOUT)?))
        });
        let marks = match asked {
            // One match on what the host said about its worst cutter, so each refusal carries its
            // own code: the webview decides from the code whether to offer the force, and a refusal
            // that names a control the operator cannot see is a dead end dressed as a choice.
            Ok((marks, snapshots)) => match worst_state(&snapshots) {
                // `force` has never passed a claimed cutter and still does not: the host is
                // answering, so `cancel` reaches the blade and it was never orphaned.
                CutterState::Claimed => {
                    return Err(IpcError::new(
                        "host_busy",
                        "a cut is active or starting on this host; cancel it before forgetting",
                    ))
                }
                // Recoverable, so the refusal names the verb that recovers it rather than a force.
                CutterState::Stalled => {
                    return Err(IpcError::new(
                        "host_busy",
                        "a cutter on this host stopped where nothing saw it stop; reconnect it before forgetting",
                    ))
                }
                // A reconnect may still fix this, or the hardware may be gone; a snapshot cannot
                // say which. So the refusal names the verb to try and stays passable — refusing
                // outright would leave a host row that could never be removed. Its own code, which
                // is what makes the force reachable.
                CutterState::Unreachable if !force => {
                    return Err(IpcError::new(
                        "host_cutter_unreachable",
                        "a cutter on this host is offline or faulted; reconnect it, or forget the \
                         host anyway if it is gone for good",
                    ))
                }
                CutterState::Unreachable | CutterState::Free => marks,
            },
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
            // Forced past a host that said nothing. Silence covers only what this desktop already
            // held when it asked: those marks can never be cleared by a poll, since the poll is a
            // call to the host being removed. A dispatch that lands after that read is outside the
            // silence, and the re-check below refuses so the token is not discarded past it.
            Err(_) => before_asking,
        };

        // Held across the snapshot and the save, same as `pair`. The connection guard is taken
        // after the snapshot: `paired_hosts` locks every connection in turn, including this one,
        // so gathering it under the guard would deadlock this verb against itself.
        let _persist = self.hosts_persist.lock().unwrap();
        let prospective: Vec<PairedHost> =
            self.paired_hosts().into_iter().filter(|h| &h.id != id).collect();
        let conn = self.host_conn(id)?;
        let Ok(mut guard) = conn.try_lock() else {
            return Err(IpcError::new(
                "host_busy",
                "this Cut Host is mid-call right now; forget it again in a moment",
            ));
        };
        if guard.lifecycle != HostLifecycle::Active {
            return Err(IpcError::new(
                "host_busy",
                "this Cut Host is already being forgotten",
            ));
        }
        // The question released this connection before `hosts_persist` was taken. A press can
        // begin, mark and finish in that gap: the check before the question already passed, and
        // a finished press is no longer dispatching when this lock is free. Both are asked again
        // here, still holding the connection, before `Forgetting` is published. A new mark is a Job
        // the answer did not cover; a press still dispatching has not marked yet. Either one
        // refuses, and the token stays.
        if self.ledger.a_press_is_dispatching_to(id)
            || self.ledger.marks_for(id).iter().any(|mark| !marks.contains(mark))
        {
            return Err(IpcError::new(
                "host_busy",
                "a cut is being sent to this host right now; forget it once that press has finished",
            ));
        }
        // Publish the transition before filesystem I/O, then release host control. Calls queued on
        // this connection wake, see `Forgetting`, and refuse immediately; no cancel, poll or
        // dispatch is held behind a slow configuration filesystem.
        guard.lifecycle = HostLifecycle::Forgetting;
        drop(guard);

        if let Err(e) = save(&prospective) {
            // The file still names the host, so this process must answer for it again too.
            conn.lock().unwrap().lifecycle = HostLifecycle::Active;
            return Err(IpcError::new("hosts_unwritable", e.to_string()));
        }

        // Save succeeded: no call can cross this transition, because every lifted `Arc` checks the
        // same state under the same mutex before it reaches the client.
        conn.lock().unwrap().lifecycle = HostLifecycle::Forgotten;
        self.remove_host(id);
        // A host that reported every cutter free has just said those marks are stale; a host forced
        // past unreachable had the operator told outright what forgetting discards. Either way the
        // marks go, because the poll that would otherwise clear them is a call to the host this
        // just removed — a mark left behind is a quit prompt nothing can ever stop raising.
        self.ledger.retract_marks(&marks);
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
        self.on_host(id, connect_timeout, |host| {
            let out = f(host.client.as_ref().expect("on_host calls only once connected"));
            if let Err(e) = &out {
                if connection_died(e) {
                    // A connection that broke mid-call stays broken, and `ensure` only redials
                    // when there is no client at all — so leaving this one in place fails every
                    // later verb on this host against the same dead socket. `list_devices` has
                    // always dropped it here; the call that most needs the same is the retry
                    // after a lost reply, which is by definition made on a connection that just
                    // failed (see `send_this_press`).
                    //
                    // `WrongReply` is dropped with it, and not only because it was a `Transport`
                    // until #283. The framing survives a wrong reply — one request written, frames
                    // read until the reply — but nothing correlates a reply with its request, so a
                    // peer that answered outside the protocol once may also have volunteered a
                    // second `Response` this call has not read, and the next verb on this socket
                    // would take that one for its own answer. Redialling costs a handshake; reading
                    // a stale reply as the answer to a cut verb does not stop at costing that.
                    //
                    // `WrongDispatch` joins them from the other end: it is that hypothesis
                    // observed rather than feared. An `Accepted` naming a dispatch this call did
                    // not send is a frame that belongs to somebody else's request, so whatever is
                    // still on this socket is not this client's to read either (#285).
                    host.last_error = Some(host_error(e));
                    host.client = None;
                }
            }
            out.map_err(|e| host_error(&e))
        })
    }

    /// Run `f` with `id`'s connection held, once it is connected. `Err` means `f` was not called.
    ///
    /// That is the whole of "never reached": an unknown host, one mid-forget, one already
    /// forgotten, or one that did not connect. A failure *inside* `f` is `f`'s own `Ok` or `Err`
    /// and is not this one — `send_this_press` relies on the difference, which a flag set inside
    /// the closure used to stand in for.
    fn on_host<T>(
        &self,
        id: &HostId,
        connect_timeout: std::time::Duration,
        f: impl FnOnce(&mut HostConnection) -> Result<T, IpcError>,
    ) -> Result<T, IpcError> {
        let conn = self.host_conn(id)?;
        // Counted across the lock acquisition so a test can tell a press *inside `on_host`*,
        // between asking for this connection and holding it, from one that has merely registered
        // its dispatch (#326 review).
        #[cfg(test)]
        self.on_host_waiting.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut guard = conn.lock().unwrap();
        #[cfg(test)]
        self.on_host_waiting.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        let host = &mut *guard;
        // Asked under the connection lock, which is where `forget` transitions it: a press that
        // queued here before the map entry went would otherwise dispatch onto a discarded token.
        match host.lifecycle {
            HostLifecycle::Active => {}
            HostLifecycle::Forgetting => {
                return Err(IpcError::new(
                    "host_busy",
                    "this Cut Host is being forgotten; try the call again if the save fails",
                ))
            }
            HostLifecycle::Forgotten => {
                return Err(IpcError::new(
                    "unknown_host",
                    "this Cut Host was forgotten while this call was waiting for it; pair it again",
                ))
            }
        }
        // Not `let client = host.ensure_within(...).ok_or_else(...)?;` — that binding would keep
        // `host`'s mutable borrow alive through the `None` arm's `host.last_error` read. The
        // temporary ends at the semicolon, and `f` then borrows `host` on its own.
        if host.ensure_within(connect_timeout).is_none() {
            return Err(host.last_error.clone().unwrap_or_else(|| {
                IpcError::new("host_unreachable", "this Cut Host has not answered")
            }));
        }
        f(host)
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
                    .connect(info.clone())?;
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
                self.manager()?.disconnect()?;
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
                self.manager()?.disconnect()?;
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
        self.reconnect_addressed(None, None)
    }

    /// One match for every verb that names a cutter.
    ///
    /// Aim, and a local cutter that is the one already aimed, share the helper that
    /// already knew that verb. A remote cutter is the same verb on that host. Five
    /// copies of this match are how the local check would drift from the one beside it.
    fn addressed<T>(
        &self,
        host: Option<HostId>,
        instance_id: Option<String>,
        aimed: impl FnOnce(&Self) -> Result<T, IpcError>,
        remote: impl FnOnce(&Self, &HostId, &str) -> Result<T, IpcError>,
    ) -> Result<T, IpcError> {
        match self.resolve_cutter(host, instance_id)? {
            CutterAddress::Aim => aimed(self),
            CutterAddress::Local { instance_id } => {
                self.require_aimed_local(&instance_id)?;
                aimed(self)
            }
            CutterAddress::Remote { host, instance_id } => remote(self, &host, &instance_id),
        }
    }

    /// Re-open the aimed cutter, or the cutter `(host, instance_id)` names.
    pub fn reconnect_addressed(
        &self,
        host: Option<HostId>,
        instance_id: Option<String>,
    ) -> Result<(), IpcError> {
        self.addressed(host, instance_id, Self::reconnect_aimed, |this, host, instance_id| {
            this.with_host(host, |c| c.reconnect(instance_id))
        })
    }

    fn reconnect_aimed(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        let Some(device) = aimed else {
            return Err(IpcError::new("device_error", "no device is connected"));
        };
        match self.route(&device)? {
            Route::Local => {
                self.refuse_while_the_local_cutter_is_working("reconnecting it")?;
                let mgr = self.manager()?;
                mgr.disconnect()?;
                Ok(mgr.connect(device)?)
            }
            Route::Host(id) => self.with_host(&id, |c| c.reconnect(&device.instance_id)),
        }
    }

    /// Where the cut has got to. Reads `driver-core`'s published status, which
    /// never blocks on the worker — so the window-close handler and the IPC
    /// command can both call it freely, even mid-transmit.
    pub fn status(&self) -> CutStatus {
        self.status_of(None, None).unwrap_or_else(|_| CutStatus::disconnected())
    }

    /// The aimed cutter when both arguments are absent, or the cutter they name.
    ///
    /// A live host read that fails is `Disconnected`, the same answer `status` has always given:
    /// the last cached value is not a live read. A target this desktop cannot address is an error.
    pub fn status_of(
        &self,
        host: Option<HostId>,
        instance_id: Option<String>,
    ) -> Result<CutStatus, IpcError> {
        // A live host read that fails stays `Disconnected`. That is not `with_host`'s error:
        // the last cached value is not a live read, and `Idle` would offer a second cut.
        self.addressed(
            host,
            instance_id,
            |this| Ok(this.status_aimed()),
            |this, host, instance_id| {
                Ok(this
                    .remote_status(host, instance_id)
                    .unwrap_or_else(|_| CutStatus::disconnected()))
            },
        )
    }

    fn status_aimed(&self) -> CutStatus {
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
            Ok(Route::Host(id)) => self
                .remote_status(&id, &device.instance_id)
                .unwrap_or_else(|_| CutStatus::disconnected()),
        }
    }

    /// Every remote cutter's last snapshot, without dialling.
    pub fn cutter_statuses(&self) -> Vec<CutterStatusView> {
        self.last_remote_status
            .lock()
            .unwrap()
            .iter()
            .map(|((host, instance_id), status)| CutterStatusView {
                host: host.clone(),
                instance_id: instance_id.clone(),
                status: status.clone(),
            })
            .collect()
    }

    /// Record every cutter a snapshot named, and clear dispatch marks only for the free ones.
    ///
    /// `status` used to keep the aimed cutter and drop the rest, so a second machine on the same
    /// Pi had no status to show and no command could name it (#307). A mark still clears only
    /// when that cutter itself says it would take a Job.
    fn remember_snapshots(&self, id: &HostId, snapshots: &[cut_host::protocol::DeviceSnapshot]) {
        self.ledger.clear_cutters(
            id,
            snapshots
                .iter()
                .filter(|s| snapshot_is_free(s))
                .map(|s| s.info.instance_id.as_str()),
        );
        let mut cache = self.last_remote_status.lock().unwrap();
        // A failed dial never gets here, so an unreachable host keeps the rows it had.
        // A host that answered and no longer names a cutter has unplugged it. This map is
        // what `cutter_statuses` publishes, and the cutting dot reads `actions.cancel` off it.
        cache.retain(|(host, instance), _| {
            host != id || snapshots.iter().any(|s| s.info.instance_id == *instance)
        });
        for snapshot in snapshots {
            cache.insert((id.clone(), snapshot.info.instance_id.clone()), snapshot.status.clone());
        }
    }

    /// Snapshot every cutter on `id`, remember them, and note that the host answered.
    ///
    /// Bookkeeping stays inside the connection closure: returning the snapshots first would
    /// release the lock and let a dispatch write a mark before this poll acted on its older
    /// free answer. A dead socket is dropped here, the same way `with_host_within` does,
    /// because `ensure` only redials when there is no client at all.
    fn poll_host(&self, id: &HostId) -> Result<Vec<cut_host::protocol::DeviceSnapshot>, IpcError> {
        self.on_host(id, STATUS_POLL_TIMEOUT, |host| {
            let out = host
                .client
                .as_ref()
                .expect("on_host calls only once connected")
                .snapshots_within(STATUS_POLL_TIMEOUT);
            if let Err(e) = &out {
                if connection_died(e) {
                    host.last_error = Some(host_error(e));
                    host.client = None;
                }
            }
            let snapshots = out.map_err(|e| host_error(&e))?;
            host.last_seen = Some(unix_millis());
            self.remember_snapshots(id, &snapshots);
            Ok(snapshots)
        })
    }

    fn remote_status(&self, id: &HostId, instance_id: &str) -> Result<CutStatus, IpcError> {
        let snapshots = self.poll_host(id)?;
        Ok(snapshots
            .into_iter()
            .find(|s| s.info.instance_id == instance_id)
            .map(|s| s.status)
            // A snapshot that omits this cutter is not a finished cut. Callers map a failed
            // dial to `Disconnected` the same way: the last cached value is not a live read,
            // and `Idle` would invite a second dispatch onto a Job still running on the Pi.
            .unwrap_or_else(CutStatus::disconnected))
    }

    /// `None` for both arguments is the aim. Naming a cutter opts out of the aim (#307).
    /// A host without a cutter is not a target: the aim is the only place a host id alone means
    /// "whichever cutter is connected".
    fn resolve_cutter(
        &self,
        host: Option<HostId>,
        instance_id: Option<String>,
    ) -> Result<CutterAddress, IpcError> {
        match (host, instance_id) {
            (None, None) => Ok(CutterAddress::Aim),
            (None, Some(instance_id)) => Ok(CutterAddress::Local { instance_id }),
            (Some(host), Some(instance_id)) => {
                if !self.hosts.lock().unwrap().contains_key(&host) {
                    return Err(IpcError::new(
                        "unknown_host",
                        format!("no Cut Host called `{}` is paired", host.0),
                    ));
                }
                Ok(CutterAddress::Remote { host, instance_id })
            }
            (Some(_), None) => Err(IpcError::new(
                "device_mismatch",
                "a cutter on a Cut Host is named by the host and the cutter together",
            )),
        }
    }

    /// Local multi-cutter is #59. An explicit local target that is not the aimed cutter is refused
    /// rather than guessed: two USB cutters on this computer are not addressable yet.
    fn require_aimed_local(&self, instance_id: &str) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed {
            Some(device) if device.host.is_none() && device.instance_id == instance_id => Ok(()),
            _ => Err(IpcError::new(
                "device_mismatch",
                "this computer addresses one cutter at a time; aim at it first",
            )),
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

    /// Whether any cut this desktop started may still be running, answered without dialling.
    ///
    /// What the window-close guard actually wants to know, and *not* the same question as
    /// `status_without_dialling().is_active()`. A dispatch accepted a second ago has not been
    /// polled yet, so the newest status anyone holds is the `Idle` from before it — and a guard
    /// reading only that waves the operator past the cut they just started. A dispatch this
    /// desktop sent and has not since seen finish counts, whatever the last status said.
    ///
    /// Not scoped to the aim, which is the whole of #158: aiming elsewhere and disconnecting are
    /// both offered mid-Job and both left a cut this desktop had dispatched running behind a guard
    /// that had stopped looking at it — `disconnect` clearing the aim outright, so the guard
    /// answered `false` however many dispatches were outstanding. Forgetting the host reached the
    /// same place from the other side, by refusing until the Job was over and then dropping the
    /// aim. The aim is where a Job is *sent*, never a statement about what is running.
    ///
    /// Errs toward warning: a Job that has since finished costs a dialog the operator dismisses,
    /// while the other way round loses the only warning there was.
    pub fn a_cut_may_be_running(&self) -> bool {
        // Asked of the manager rather than through the aim, so a local cut is still seen while the
        // operator is looking at a Pi. `connect` refuses to aim at a host mid-local-cut, which
        // makes that unreachable today; the guard does not need that to stay true to be right.
        if self.local_cut_is_active() || self.status_without_dialling().is_active() {
            return true;
        }
        // One mark is enough, and a cutter with no marks left has no entry: `unmark_dispatched`
        // removes the set with its last id, so an entry existing is the whole of the question.
        if self.ledger.any_mark() {
            return true;
        }
        // A press that has begun and not returned counts even before it writes a mark: between
        // `begin_dispatch` and the host connection it is queued, and the Job it is about to start
        // is exactly what the operator must not be waved past (#158).
        self.ledger.a_press_is_dispatching()
    }

    /// Permission for one press to start a cut, or the refusal if the window has gone.
    ///
    /// Both routes ask, because both start something a closing process cannot look after: a remote
    /// Job outlives the desktop on its host, and a local Job dies with the transport this process
    /// owns. The permit must be held across whatever starts the cut — `CloseGate::commit_if` is the
    /// writer, so a press either holds it when the close is decided, and the close is refused, or
    /// asks afterwards and is told no.
    fn permit_for_a_press(&self) -> Result<SendPermit<'_>, IpcError> {
        self.close_gate.permit().ok_or_else(|| IpcError::new("closing", CLOSED_MID_PRESS))
    }

    /// Whether the window may close now, committing to it when the answer is yes.
    pub fn commit_close(&self) -> bool {
        self.close_gate.commit_if(|| !self.a_cut_may_be_running())
    }

    /// Commit to a close that is happening whatever is outstanding — the stand-downs, where the
    /// window goes because the webview could not be told.
    pub fn commit_close_regardless(&self) {
        self.close_gate.commit_regardless();
    }

    /// Decide a request to leave the app, from either door: the window's close button or the
    /// Quit menu item. `warn` shows the operator the prompt.
    ///
    /// Never blocking, local or remote. The window callback is synchronous and runs on the main
    /// thread, and `status()` aimed at a host dials: its 2s budget starts only once that host's
    /// connection lock is in hand, and `list_devices` holds the same lock across a 30s call — so a
    /// close arriving during a device-list poll froze the window for the listing plus the poll plus
    /// an unbounded resolve (#115). What this asks is whether to warn, which a status alone cannot
    /// answer: a dispatch accepted a second ago has not been polled yet, so the newest status
    /// anyone holds is the `Idle` from before it. `a_cut_may_be_running` counts every dispatch this
    /// desktop sent and has not seen finish, on any cutter and whatever is aimed at now (#158); a
    /// warning about a Job that has since ended costs a dialog the operator dismisses.
    ///
    /// `commit_close` answers and commits in one step, so a press queued behind a poll for a host's
    /// connection cannot cross into its dispatch after the app has been let go: the Job it would
    /// start outlives the process that started it.
    ///
    /// Readiness is the delivery fact: `emit` only proves serialization and returns success even
    /// with nobody listening, so once the webview has acknowledged its listener a successful `warn`
    /// and the refusal are one act. Without that acknowledgement the guard fails open by design — a
    /// refusal with no listener has no escape — but not silently: the log line is the only evidence
    /// a startup or listener failure disabled it when an operator later reports a missing warning.
    /// Both fail-open arms commit, because the app goes either way and nothing new may start.
    pub fn decide_close<E: std::fmt::Display>(
        &self,
        warn: impl FnOnce() -> Result<(), E>,
    ) -> CloseVerdict {
        if self.commit_close() {
            return CloseVerdict::Proceed;
        }
        if self.close_guard_ready() {
            match warn() {
                Ok(()) => return CloseVerdict::Warn,
                Err(e) => {
                    self.commit_close_regardless();
                    eprintln!("cuthulhu: a cut may be running and the warning could not be shown: {e}");
                }
            }
        } else {
            self.commit_close_regardless();
            eprintln!("cuthulhu: a cut may be running but the warning listener is not ready; closing");
        }
        CloseVerdict::Proceed
    }

    /// Commit once the operator has answered the prompt, and stop the local cutter before waiting
    /// out a send already in flight.
    ///
    /// The flag, the stop and the wait are one function because each swap is a different bug:
    /// stopping before the flag lets a press start during the cancel, and waiting before the stop
    /// lets the local worker keep transmitting for up to `CLOSE_GATE_WAIT`.
    pub fn commit_close_after_confirmation(&self) {
        self.close_gate.commit_regardless();
        self.stop_local_motion();
        self.close_gate.wait_for_an_inflight_send();
    }

    pub fn set_close_guard_ready(&self, ready: bool) {
        self.close_guard_ready.store(ready, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn close_guard_ready(&self) -> bool {
        self.close_guard_ready.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn cancel(&self) -> Result<(), IpcError> {
        self.cancel_addressed(None, None)
    }

    /// Stop the aimed cutter, or the cutter `(host, instance_id)` names.
    ///
    /// The aim is where a Job is sent, not which blade a Cancel reaches. Two cutters on one Pi
    /// used to share that aim, so stopping one stopped whichever happened to be connected (#307).
    pub fn cancel_addressed(
        &self,
        host: Option<HostId>,
        instance_id: Option<String>,
    ) -> Result<(), IpcError> {
        self.addressed(host, instance_id, Self::cancel_aimed, |this, host, instance_id| {
            this.with_host(host, |c| c.cancel(instance_id))
        })
    }

    fn cancel_aimed(&self) -> Result<(), IpcError> {
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

    /// Stop what this process is driving, and leave a Cut Host's Jobs to the host.
    ///
    /// What "quit anyway" does with the cut it just warned about, and deliberately not `cancel`:
    /// that routes by the aim, so it cancelled a host-owned Job when the operator happened to be
    /// looking at it and left an identical one running when they were not — a distinction the aim
    /// does not make (#158). The split it is replaced with is the one the product already has: the
    /// local cutter's transport dies with this process, so a Job left mid-motion there could never
    /// be stopped or resumed by anything, while a Cut Host owns its Jobs and keeps cutting whether
    /// this desktop is running or not — which is why `disconnect` never cancelled a remote cutter
    /// either. Cancelling a remote Job stays an addressed act with an acknowledgement to wait for,
    /// not a side effect of closing a window
    /// (`docs/adr/0002-the-close-guard-answers-for-every-cut-this-desktop-started.md`).
    ///
    /// `shutdown` would cancel the local Job too — it sets the flag and sends `Cancel` before it
    /// joins — so this is not the only thing stopping the blade. It is here because the decision is
    /// worth saying out loud in the code that makes it: what quitting stops and what it leaves are
    /// two different answers, and one of them is a `shutdown` side effect nobody reading
    /// `force_quit` would see.
    pub fn stop_local_motion(&self) {
        if let Ok(mgr) = self.manager() {
            mgr.cancel();
        }
    }

    pub fn resume(&self) -> Result<(), IpcError> {
        self.resume_addressed(None, None)
    }

    pub fn resume_addressed(
        &self,
        host: Option<HostId>,
        instance_id: Option<String>,
    ) -> Result<(), IpcError> {
        self.addressed(host, instance_id, Self::resume_aimed, |this, host, instance_id| {
            this.with_host(host, |c| c.resume(instance_id))
        })
    }

    fn resume_aimed(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            None | Some(Route::Local) => Ok(self.manager()?.resume()?),
            Some(Route::Host(id)) => {
                let device = aimed.expect("a route implies a device").instance_id;
                self.with_host(&id, |c| c.resume(&device))
            }
        }
    }

    pub fn confirm_pass_done(&self) -> Result<(), IpcError> {
        self.confirm_addressed(None, None)
    }

    pub fn confirm_addressed(
        &self,
        host: Option<HostId>,
        instance_id: Option<String>,
    ) -> Result<(), IpcError> {
        self.addressed(host, instance_id, Self::confirm_aimed, |this, host, instance_id| {
            this.with_host(host, |c| c.confirm_pass_done(instance_id))
        })
    }

    fn confirm_aimed(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            None | Some(Route::Local) => Ok(self.manager()?.confirm_pass_done()?),
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

        // Only enabled passes are cut, so only their presets are worth reading. Filtered to the
        // connected machine here rather than at the lookup below: `load_presets` returns every
        // machine's entries (that is what `list_presets` filters for display), and a preset is
        // machine-scoped — its speed and force mean nothing on another cutter. Builtin ids are
        // machine-prefixed so they could not collide, but a *user* preset's id is the
        // operator's own string, so `my-vinyl` can exist for both a Puma and a Cameo.
        let enabled = || request.passes.iter().filter(|p| p.enabled);
        let presets: Vec<MaterialPreset> = if enabled().any(|p| p.preset_id.is_some()) {
            let path = default_presets_path()
                .ok_or_else(|| IpcError::new("no_config_dir", "cannot resolve presets file location"))?;
            load_presets(&path)?
                .into_iter()
                .filter(|p| p.machine_id == connected.machine_id)
                .collect()
        } else {
            Vec::new()
        };

        let passes: Vec<PassSelection> = enabled()
            .map(|dto| {
                let preset = match dto.preset_id.as_deref() {
                    // A named preset that the file no longer resolves is refused, not silently
                    // replaced by defaults. The operator asked for that material's speed and
                    // force; falling back would cut real material with settings unrelated to
                    // what the pass is named for, and a machine-scoped preset disappears for
                    // ordinary reasons — the project was converted, or the entry was deleted.
                    // The planner still keys such a pass (a document may name anything); it is
                    // this boundary, which is where the preset file is actually read, that has
                    // to fail closed.
                    Some(id) => match presets.iter().find(|p| p.id == id) {
                        Some(found) => Some(found),
                        None => return Err(IpcError::new("unknown_preset",
                            format!("this cut uses the material preset `{id}`, which is not available for this machine; pick another for that pass"))),
                    },
                    None => None,
                };
                let override_ = SettingsOverride {
                    speed: dto.speed,
                    force: dto.force,
                    repeat_count: dto.repeat_count,
                };
                Ok(PassSelection { key: dto.key.clone(), settings: resolve_settings(preset, &override_) })
            })
            .collect::<Result<_, IpcError>>()?;

        // The wire carries the revision as a string. One that isn't a u64 was
        // never issued by `doc_revision`, so it cannot be the current plan.
        let Ok(expected) = request.doc_revision.parse::<u64>() else {
            return Err(IpcError::new("stale_plan", "cut request carries an unrecognized plan revision"));
        };
        let opts = PlanOptions { passes, expect_revision: Some(expected), allow_out_of_bounds: false };

        // Planned here, at cut time, against the live document — `expect_revision`
        // is what refuses the cut if that is no longer the document the UI planned.
        let planned = plan_passes_for(&app.editor.doc, request.grouping, request.weed.as_ref())
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
            // The same gate the remote arm takes, for the same reason: a press that has passed the
            // aim check but not yet reached the worker is invisible to `local_cut_is_active`, and a
            // blade that starts as this process exits loses the transport that drives it.
            Route::Local => {
                let _permit = self.permit_for_a_press()?;
                Ok(CutStarted { job_id: self.manager()?.cut(passes)?, duplicate: false })
            }
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
                // reuse while this Job still has an entry here, mint fresh once nothing holds one.
                // The desktop is the only party that can tell them apart: it is the
                // one that knows what the last try was told.
                //
                // The first Cut after a lost reply is therefore always read as the retry. That
                // is the safe direction, and a settling answer is what ends it: the retry is either
                // accepted — the host's dedupe making it a no-op if the Job is already running — or
                // refused, and either frees the next Cut to be a new Job. A desktop restarted in
                // between loses `in_doubt` and is back to cutting twice — persisting it is the fix
                // if that stops being rare.
                //
                // Chosen and written down in one step, before any network call, inside `send_this_press`.
                // Choosing and recording separately let two presses of Cut both find nothing and mint
                // an id each: if the first reached the host and lost its reply, the second's entry
                // replaced the only record of it, and the retry that should have been recognised
                // went out under a name the host had never seen.
                match self.send_this_press(&id, &device, &machine_id, &key, passes) {
                    // ponytail: a remote dispatch reports job id 0, because `Response::Accepted` carries none —
                    // `DeviceManager::cut` does not return one until the Job reaches a pause point. The sheet
                    // reads 0 as an accept that named no id (`useJob.ts`'s `remember`) and takes the real id
                    // off the worker's later status. Put the id on `Accepted` when the host can return it
                    // before that pause.
                    //
                    // `duplicate` is the host's own answer, not a guess: it is the only party that
                    // knows whether it had already accepted this id, and "already accepted" and
                    // "your Job has started" look identical to the operator otherwise — one of
                    // them means the cutter is never going to move (#121).
                    Ok(Sent::Admitted { duplicate }) => Ok(CutStarted { job_id: 0, duplicate }),
                    // A close was committed before this press reached the host. Nothing was sent
                    // and nothing was marked, which is the point: a Job started now would outlive
                    // the process the operator has just left.
                    Ok(Sent::NotSent) => Err(IpcError::new("closing", CLOSED_MID_PRESS)),
                    // The host was asked, and the error is already plain or unconfirmed.
                    Ok(Sent::Failed(error)) => Err(error),
                    // `on_host` returned before calling the closure: nothing was marked, and the
                    // reservation stays. The error is already plain or unconfirmed.
                    Err(error) => Err(error),
                }
            }
        }
    }

    /// Send one press to `id` and return the one outcome of that press.
    ///
    /// `Err` is `on_host` not calling the closure — the host was never asked. Every other outcome
    /// is `Ok`, including a dispatch the host refused, so a failure inside the closure cannot be
    /// mistaken for one that never left this machine. The reservation is not retracted on either
    /// failure: a host that was never reached cannot have seen the id, and a reply that may have
    /// been lost is exactly the id the next press must reuse.
    fn send_this_press(
        &self,
        id: &HostId,
        device: &str,
        machine_id: &str,
        key: &JobKey,
        passes: Vec<CutPass>,
    ) -> Result<Sent, IpcError> {
        // `recorded_the_entry` says only that no entry existed for this Job before this call. It
        // decides what a failure means, and nothing else: it is not used to undo the reservation.
        // It stays in this function. The press's outcome leaves as `Sent` or as this `Err`.
        let (attempt, dispatch_id, recorded_the_entry) = self.ledger.begin_dispatch(key);
        let attempt_id = attempt.attempt;
        // What the closure did, before the operator-facing wording. The wording asks `in_doubt`
        // and is applied after this returns, so it does not run under the connection lock.
        enum Asked {
            Admitted { duplicate: bool },
            NotSent,
            Failed { error: IpcError, settled: bool },
        }
        let asked = match self.on_host(id, cut_host::client::CONNECT_TIMEOUT, |host| {
            // Held across the check and the send, and taken inside the connection so it never
            // spans the queue wait: a close cannot commit while a permit is out, and a press that
            // asks after one committed is told no and starts nothing.
            let Some(_permit) = self.close_gate.permit() else {
                return Ok(Asked::NotSent);
            };
            // Marked immediately before the request, under the HostConnection lock. A poll that
            // already owns that lock therefore runs before the mark exists; a poll that sees the
            // mark runs after the dispatch has released the lock.
            self.ledger.mark_dispatched(id, device, attempt_id);
            let sent = {
                let client = host.client.as_ref().expect("on_host calls only once connected");
                client.dispatch(&dispatch_id, device, machine_id, passes)
            };
            // Cleared by an answer that settles what the host did — a refusal included, since a
            // host that refused says it started nothing. A reply this request cannot use settles
            // nothing: it arrived, so nothing was lost, but it does not say whether the Job began,
            // so the entry stays.
            //
            // Keeping it is the only thing that leaves host-side deduplication a chance: a fresh
            // id matches nothing the host has seen, so nothing there can read the next press as
            // this dispatch again. The same id may still be read as a new Job — a host forgets ids
            // past `ID_RETENTION` and past `MAX_REMEMBERED_IDS`, and a peer answering outside the
            // protocol may dedupe nothing at all — which is why that failure is `unconfirmed`
            // rather than an all-clear. Clearing the entry would remove the chance rather than
            // improve on it.
            //
            // A reply about another dispatch stays for the same reason and with less known, not
            // more: a wrong variant at least came from a peer answering this request badly, while
            // an `Accepted` naming another Job says nothing about this one at all (#285).
            let settled = !matches!(&sent, Err(e) if connection_died(e));
            if settled {
                self.ledger.settle_dispatch(key, &dispatch_id, attempt_id);
            }
            // A refusal is the host saying it started nothing, so it is not something the close
            // guard should hold the window for — this press's mark, and no other. A sibling press
            // of the same Job may have been accepted and be cutting right now, and this answer is
            // not about that one (#290).
            if matches!(sent, Err(cut_host::client::ClientError::Refused(_))) {
                self.ledger.unmark_dispatched(id, device, attempt_id);
            }
            if let Err(ref e) = sent {
                if connection_died(e) {
                    // Same drop `with_host_within` makes. This closure returns `Ok`, so that path's
                    // match on `Err` would leave the dead socket in place for the retry.
                    host.last_error = Some(host_error(e));
                    host.client = None;
                }
            }
            Ok(match sent {
                Ok(admitted) => Asked::Admitted {
                    duplicate: admitted == cut_host::protocol::Admitted::AlreadyAccepted,
                },
                Err(e) => Asked::Failed { error: host_error(&e), settled },
            })
        }) {
            Ok(asked) => asked,
            // The closure was not called, so no mark was written. Nothing on the host can have
            // started, which is the `settled` this report treats as "this press is the whole story"
            // when it also recorded the entry.
            Err(missed) => {
                return Err(self.report_press_failure(key, recorded_the_entry, true, missed));
            }
        };
        Ok(match asked {
            Asked::Admitted { duplicate } => Sent::Admitted { duplicate },
            Asked::NotSent => Sent::NotSent,
            Asked::Failed { error, settled } => {
                Sent::Failed(self.report_press_failure(key, recorded_the_entry, settled, error))
            }
        })
    }

    /// Plain error when this press is the whole of what happened; unconfirmed when it is not.
    ///
    /// `recorded_the_entry` is "no entry stood before this call". `settled` is "this attempt's
    /// outcome is known" — a refusal, an accept, or a call that never reached a host. Both are
    /// required, and a refusal is why the second is not enough on its own: it settles this attempt
    /// while saying nothing about an earlier one, so it is plain only when there was no earlier one
    /// (#288). A first press against an offline Pi must not claim the Job may be cutting; a press
    /// made while an earlier dispatch is unsettled must.
    ///
    /// What the next press can do about an unconfirmed one is asked of the record, as the *next*
    /// press will answer it: an entry this call is reusing can age out of reuse while the call is
    /// still in flight.
    fn report_press_failure(
        &self,
        key: &JobKey,
        recorded_the_entry: bool,
        settled: bool,
        failure: IpcError,
    ) -> IpcError {
        if recorded_the_entry && settled {
            failure
        } else {
            unconfirmed(&failure, self.ledger.retry_would_reuse(key, std::time::Instant::now()))
        }
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
/// `stale_plan` is the one the frontend actually keys off (`useCutDispatch.ts`).
/// The message is `cutplan`'s — this used to restate all ten here, and the CLI
/// restated them again, differently.
fn map_cut_error(e: CutError) -> IpcError {
    IpcError::new(e.code(), e.to_string())
}

#[derive(Debug, Serialize)]
pub struct PlanCutResponse {
    pub passes: Vec<PlanCutPassSummary>,
    pub skipped_not_cut: usize,
    pub doc_revision: String,
    pub travel: Vec<[f64; 4]>,
}

#[derive(Debug, Serialize)]
pub struct PlanCutPassSummary {
    /// The pass's key, as the canonical string the dialog keys its rows on and sends back. A
    /// string rather than a tagged object so the CLI, this DTO and the dialog hold one
    /// spelling.
    pub key: PassKey,
    pub shape_count: usize,
    pub node_ids: Vec<document::NodeId>,
    /// Each shape's first world-space point, parallel to `node_ids` — where the blade
    /// actually lands. The preview's order badges anchor here; the UI cannot derive it
    /// from `travel`, which has no move to the first shape and none for a single-shape
    /// plan. `None` is a shape whose outline flattened to nothing.
    pub starts: Vec<Option<[f64; 2]>>,
    /// The pass's weed lines then its border, in world mm, for the preview to draw. Empty
    /// without weed options.
    pub weed: Vec<Vec<[f64; 2]>>,
}

/// Summarizes `plan_passes_for` output for the UI — not the raw `DocumentPasses`
/// (which carries full flattened polylines the cut dialog doesn't need).
///
/// Takes the grouping rather than defaulting it: unlike `cutplan::plan_passes`, this has no
/// caller that means "whatever the default is" — the dialog always has a mode selected.
pub fn plan_cut_response(doc: &document::Document, grouping: Grouping, weed: Option<&WeedOptions>)
    -> Result<PlanCutResponse, IpcError> {
    let planned = plan_passes_for(doc, grouping, weed)
        .map_err(|e| IpcError::new("plan_error", e.to_string()))?;
    let refs: Vec<&DocumentPass> = planned.passes.iter().collect();
    let travel = cutplan::travel_moves(&refs);
    Ok(PlanCutResponse {
        passes: planned.passes.iter().map(|p| PlanCutPassSummary {
            key: p.key.clone(),
            shape_count: p.shapes.len(),
            node_ids: p.shapes.iter().map(|s| s.node_id).collect(),
            starts: p.shapes.iter().map(|s| {
                s.polylines.first().and_then(|p| p.first()).map(|pt| [pt.x, pt.y])
            }).collect(),
            weed: p.weed.iter().map(|l| l.iter().map(|pt| [pt.x, pt.y]).collect()).collect(),
        }).collect(),
        skipped_not_cut: planned.skipped_not_cut,
        doc_revision: planned.doc_revision.to_string(),
        travel: travel.into_iter().map(|(a, b)| [a.x, a.y, b.x, b.y]).collect(),
    })
}

/// The dialog's pass list as it stands: the order, plus which passes are enabled. The same
/// two facts `cut` is sent, in the same shape, so the preview and the cut cannot disagree
/// about what "configured" means.
#[derive(Debug, Deserialize)]
pub struct TravelPassDto {
    pub key: PassKey,
    pub enabled: bool,
}

/// Travel for the configured pass list the cut dialog currently shows, replanned against
/// the live document. Reorder and enable are UI-side list edits — the plan itself is not
/// resent — so this re-asks the planner rather than letting the frontend recompute travel
/// from geometry it does not have. The revision check keeps the stale-plan rule: travel for
/// a list the operator arranged against a document that has since changed is refused,
/// exactly as the cut itself would be.
pub fn travel_for_order(
    doc: &document::Document,
    doc_revision: &str,
    grouping: Grouping,
    weed: Option<&WeedOptions>,
    configured: &[TravelPassDto],
) -> Result<Vec<[f64; 4]>, IpcError> {
    // Same rule as `prepare_cut`: a revision string that isn't a u64 was never
    // issued by `doc_revision`, so it cannot be the current plan.
    let Ok(expected) = doc_revision.parse::<u64>() else {
        return Err(IpcError::new("stale_plan", "travel request carries an unrecognized plan revision"));
    };
    let planned = plan_passes_for(doc, grouping, weed)
        .map_err(|e| IpcError::new("plan_error", e.to_string()))?;
    if planned.doc_revision != expected {
        return Err(map_cut_error(CutError::StalePlan { expected, actual: planned.doc_revision }));
    }
    // With the revision equal, the plan is the one the dialog's rows came from, so the list
    // must name each planned pass exactly once — anything else is a caller bug, and a pass
    // quietly missing from the list would misdraw the machine's motion as surely as a wrong
    // order would. Disabled passes are named too, which is what keeps that check available:
    // they are dropped here rather than by omitting them.
    let mut remaining: Vec<&DocumentPass> = planned.passes.iter().collect();
    let mut refs: Vec<&DocumentPass> = Vec::with_capacity(configured.len());
    for pass in configured {
        let Some(i) = remaining.iter().position(|p| p.key == pass.key) else {
            // A key planned but already consumed is a duplicate in the list — a mismatch,
            // not an unknown pass; "no planned pass is called X" would be a lie about a
            // pass that exists.
            return Err(if planned.passes.iter().any(|p| p.key == pass.key) {
                IpcError::new("plan_mismatch", "the requested pass list does not name every planned pass exactly once")
            } else {
                map_cut_error(CutError::UnknownPass(pass.key.clone()))
            });
        };
        let planned_pass = remaining.remove(i);
        // The head never travels to a pass that will not be cut — `prepare_cut` filters the
        // same way, and travel drawn through skipped geometry is motion the machine will
        // not make (docs/superpowers/specs/2026-07-24-cut-workflow-design.md).
        if pass.enabled {
            refs.push(planned_pass);
        }
    }
    if !remaining.is_empty() {
        return Err(IpcError::new("plan_mismatch", "the requested pass list does not name every planned pass exactly once"));
    }
    Ok(cutplan::travel_moves(&refs).into_iter().map(|(a, b)| [a.x, a.y, b.x, b.y]).collect())
}

/// A preset's identity is the pair `(machine_id, id)`, not the id: an id is the operator's own
/// string, so `my-vinyl` legitimately exists for a Cameo and a Puma, and keyed on the id alone one
/// machine's save or delete destroyed the other's entry (#153).
///
/// Each operation re-derives the on-disk *user-only* list (builtins always shadow-load, with
/// `builtin:false` forced onto user entries — see `cutplan::presets::load_presets`) so a round trip
/// through `save_user_presets` never writes a builtin back to disk.
///
/// The presets file is a parameter rather than resolved here, the way `pair`/`forget` take
/// `hosts.json`: the command layer resolves the default location, and a test can hand over a
/// temporary file instead of the operator's real one.
pub fn list_presets(path: &Path, machine_id: &str) -> Result<Vec<MaterialPreset>, IpcError> {
    let all = load_presets(path)?;
    Ok(all.into_iter().filter(|p| p.machine_id == machine_id).collect())
}

fn user_entries(path: &Path) -> Result<Vec<MaterialPreset>, IpcError> {
    Ok(load_presets(path)?.into_iter().filter(|p| !p.builtin).collect())
}

/// Whether the pair names a preset that ships with the app. Keyed on the pair like everything
/// else here: a builtin belongs to one machine, and the operator's own id may equal it on another.
fn ships_with_the_app(machine_id: &str, id: &str) -> bool {
    cutplan::presets::builtin_presets()
        .iter()
        .any(|p| p.machine_id == machine_id && p.id == id)
}

pub fn save_preset(path: &Path, preset: MaterialPreset) -> Result<(), IpcError> {
    // What the entry *is* is settled before what it holds. An id-less entry is a save the operator
    // never gets back (`load_presets` drops those, so the editor closes, the file grows and the
    // next listing shows nothing new); a pair that names a builtin can never be saved at all, so
    // saying "force must be 1..=33" first sends the operator to repair settings on an entry that
    // was refusable whatever they held (Codex on PR #264).
    if preset.id.is_empty() {
        return Err(IpcError::new("invalid_preset", "a material preset needs an id"));
    }
    // And the other half of the pair: `list_presets` answers per machine, so an entry naming none
    // is written and then never listed again — the same unreachable save, one field along (Copilot
    // on PR #264).
    if preset.machine_id.is_empty() {
        return Err(IpcError::new("invalid_preset", "a material preset needs the machine it is for"));
    }
    // A user entry saved under a builtin's pair shadows it, and nothing hands the shipped
    // settings back afterwards — the material the app came with is gone for good.
    if ships_with_the_app(&preset.machine_id, &preset.id) {
        return Err(IpcError::new(
            "builtin_preset",
            format!(
                "`{}` is a material preset that ships with the app; save your own under a different id",
                preset.id
            ),
        ));
    }
    // The name is the whole of what the picker shows, so a blank one is a row naming no material.
    if preset.name.trim().is_empty() {
        return Err(IpcError::new("invalid_preset", "a material preset needs a name"));
    }
    // Preflight refuses these settings at the cut, so storing them makes a material the operator
    // can pick from the dialog and never cut with.
    if let Some(reason) = cutplan::preflight::preset_settings_out_of_range(&preset.settings) {
        return Err(IpcError::new("invalid_preset", reason));
    }

    let mut user = user_entries(path)?;
    user.retain(|p| (&p.machine_id, &p.id) != (&preset.machine_id, &preset.id));
    user.push(MaterialPreset { builtin: false, ..preset });
    Ok(save_user_presets(path, &user)?)
}

pub fn delete_preset(path: &Path, machine_id: &str, id: &str) -> Result<(), IpcError> {
    let mut user = user_entries(path)?;
    let before = user.len();
    user.retain(|p| (p.machine_id.as_str(), p.id.as_str()) != (machine_id, id));
    // Reporting success having removed nothing left the entry listed, which reads as the app
    // ignoring the operator. What went wrong is decided from the entries actually on disk, not
    // from the pair: a user entry shadowing a builtin's pair is the operator's to delete, and
    // deleting it reveals the builtin again.
    if user.len() == before {
        return Err(if ships_with_the_app(machine_id, id) {
            IpcError::new(
                "builtin_preset",
                format!("`{id}` ships with the app, so there is nothing of yours to delete"),
            )
        } else {
            IpcError::new(
                "unknown_preset",
                format!("no material preset `{id}` is saved for `{machine_id}`"),
            )
        });
    }
    Ok(save_user_presets(path, &user)?)
}

#[cfg(test)]
mod tests;
