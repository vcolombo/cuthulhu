// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use cut_host::client::HostClient;
use cutplan::presets::{
    default_presets_path, load_presets, resolve_settings, save_user_presets, MaterialPreset,
    SettingsOverride,
};
use cutplan::{plan_cut, plan_passes_with, DocumentPass, CutError, Grouping, PassKey, PassSelection, PlanOptions};
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
        ClientError::Transport(_) => "host_unreachable",
    };
    IpcError::new(code, e.to_string())
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

/// Whether an id written at `written` is still one a retry can be sent under.
///
/// The one place this desktop decides that, read by `reserve_dispatch_id` — which prunes by it —
/// and by `retry_would_reuse`, which is how the sentence above learns whether the retry it offers
/// is one the next press will actually make. Two copies of the rule would be two policies, and the
/// promise #286 removed was exactly what a disagreement between them produces.
///
/// Aligned to the host's own `ID_RETENTION` rather than a number of this side's invention: past it
/// the host has forgotten the id, so reusing it protects nothing and only hides that it protects
/// nothing. The two windows do not coincide — this ages from when the id was written, while a
/// host re-stamps an id every time a duplicate arrives — and neither side's cap is in it at all.
fn still_reusable(written: std::time::Instant, now: std::time::Instant) -> bool {
    now.saturating_duration_since(written) < cut_host::host::ID_RETENTION
}

#[derive(Deserialize)]
pub struct CutRequest {
    pub device_instance_id: String,
    pub doc_revision: String,
    /// How the dialog grouped the passes it is naming. Sent rather than remembered: the plan,
    /// the travel and the cut are three round trips, and a mode kept in `AppState` could be
    /// changed between them while the stale-plan check only guards the document.
    pub grouping: Grouping,
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
    /// The last thing that went wrong with this host, or `None` while nothing has. The name is the
    /// serialized one this field has always had, kept because every consumer only prints the
    /// message: since #283 it also carries a host that answered outside the protocol, which was
    /// reached and whose sentence says so, and before that a refused token, a changed certificate
    /// and a refusal. The row prints it when it is present; nothing reads what is in it.
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
    /// The dispatch id a Job's next press goes out under while an entry for it stands, by the Job it
    /// names.
    ///
    /// The host deduplicates on the dispatch id precisely so a retry after a dropped reply
    /// cannot cut the same material twice — but only the sender knows whether a given press of
    /// Cut *is* that retry, and nothing in the Job says so: a retry and a second sheet of the
    /// same design are byte-identical. What separates them is this entry: while a Job has one, the
    /// next press reuses the id in it instead of minting a fresh one.
    ///
    /// Written before the request goes out, and dropped by an answer that settles what the host did:
    /// `Accepted` means it has the Job, a refusal means it started nothing. A dropped reply settles
    /// neither, and since #283 neither does a reply the request could not use — it arrived, so
    /// nothing was lost, but it says nothing about whether the Job began.
    ///
    /// "Settles" is the client's reading of the reply, not a proof about the host. An `Accepted` is
    /// believed without checking which dispatch it names (#285), and past the bounds below the id it
    /// holds may mean nothing to the host any more. Deduplicating by memory is best-effort on both
    /// sides, and nothing here upgrades it.
    ///
    /// So an entry does not mean "outcome unknown", and a dispatch that never left this machine is
    /// the case that shows the difference: its outcome is known — nothing started — and the entry
    /// stays anyway, because clearing it buys nothing. A host that was never reached cannot have
    /// seen the id either, so the next Cut sending it again is cut normally rather than
    /// deduplicated away (see `execute_cut`). Whose entry it is *is* answerable, by `dispatching`
    /// below; it is only that the answer changes nothing here. Kept, in other words, wherever
    /// letting go would buy nothing or risk a press's retry, which is a weaker condition than
    /// doubt.
    ///
    /// Beyond those two answers an entry leaves only by expiry or by making room, below. An earlier
    /// version aged them out after fifteen minutes, which was the
    /// wrong fix for a dispatch nobody revisits: inside the host's window, time cannot show that
    /// the operator loaded fresh material, so expiring early mints a new id for a Job the host
    /// still remembers under the old one and cuts the design twice. What makes an entry nobody
    /// revisits harmless is that the next Cut is *told* it was read as a retry
    /// (`CutStarted::duplicate`), and that a settling answer clears the entry, so pressing Cut
    /// again really does cut (#121).
    ///
    /// The one honest expiry is `ID_RETENTION`, the host's own constant: an hour after this entry
    /// was written, holding it only pretends to a protection that may well have lapsed. Nothing on
    /// either side can prevent a re-cut past that point — which is the true limit of deduplicating
    /// by memory, not something a client-side rule can close.
    ///
    /// "May well", because sharing the constant does not make the two windows the same window. This
    /// entry ages from when the id was written; a host's `Admission` re-stamps an id every time a
    /// duplicate arrives, so a Job retried near the hour can be remembered there after it has
    /// expired here. And two capacity bounds cut in ahead of the hour on both sides, neither
    /// controlling the other's: this map evicts its oldest entry at `MAX_JOBS_IN_DOUBT`, a host
    /// forgets its oldest accepted id at `MAX_REMEMBERED_IDS`. So an entry can outlive the host's
    /// memory of its id and an id can outlive this entry, in either direction. All of it is a
    /// backstop against exhaustion rather than a policy, which is why the numbers are set where
    /// reaching them is itself the unusual event.
    ///
    /// **Lock order:** taken alone, or last — never with `hosts` or a `HostConnection` acquired
    /// while holding it, and under `dispatching` where a clear has to ask about both at once
    /// (`settle_dispatch`).
    in_doubt: Mutex<HashMap<JobKey, (cut_host::protocol::DispatchId, std::time::Instant)>>,
    /// The presses of Cut dispatching each Job, and what each of them has been told.
    ///
    /// Ownership, which `in_doubt` cannot express on its own: two presses of the same Job can be
    /// in flight at once, sharing the one entry, and an answer that settles one of them says
    /// nothing about the other. Clearing the entry on the strength of that answer took away the
    /// record the other press's retry sends under, so the next press minted a name the host had
    /// never seen (#290). The entry is cleared by whichever settling answer finds that every
    /// press still in flight has had one.
    ///
    /// **Lock order:** before `in_doubt`, never after; and like `in_doubt`, never with `hosts` or
    /// a `HostConnection` acquired while holding it — `settle_dispatch` runs inside `with_host`,
    /// which means this is taken with a connection already held.
    dispatching: Mutex<HashMap<JobKey, Presses>>,
    /// The last status each remote cutter actually reported, so something can be said about one
    /// without dialling it.
    last_remote_status: Mutex<HashMap<(HostId, String), CutStatus>>,
    /// Remote dispatches this desktop started and has not since seen end, by cutter and by the
    /// press that started each one.
    ///
    /// The cache above cannot answer for the window that matters most: a dispatch accepted a
    /// moment ago has not been polled yet, so the newest thing anyone heard is the `Idle` from
    /// before it — and the window-close guard would wave the operator past a cut it just started.
    /// This says "we started something and nothing has told us it ended" without dialling.
    ///
    /// Per press rather than a boolean per cutter, because a press that learns the host started
    /// nothing retracts its own mark, and one press's refusal says nothing about another press —
    /// of this Job or of any other on that cutter — that was accepted and is cutting right now
    /// (#290). A poll that finds the cutter free clears every mark on it at once, which is not the
    /// same act: it is the cutter itself saying it would take another Job, and no press outranks
    /// that.
    ///
    /// Cleared by any poll that hears a cutter say it would take a Job — `status` for the aimed
    /// one, `list_devices` for every cutter on every paired host — and by forgetting the host.
    /// Between those it holds one id per press, so a session that dispatches without ever polling
    /// again grows this by one entry per press; the numbers are an operator's finger, not a loop.
    remote_dispatched: Mutex<HashMap<(HostId, String), HashSet<AttemptId>>>,
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
/// answer, not a policy anyone should reach: entries clear on a settling answer, and expire on the
/// host's own `ID_RETENTION` — though not in step with the host's own memory of the id, for the
/// reasons on `in_doubt`. The oldest is evicted, which is the one a retry is least likely to name.
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

/// One press of Cut, as something the state it writes can be attributed to.
///
/// Both pieces of per-Job dispatch state are shared by presses — the id a retry goes out under,
/// and the mark that holds the window — and neither had room for two, so an answer about one press
/// cleared what another was still using (#290). A press retracts only the mark carrying its own
/// id; the Job's in-doubt entry is shared, and what a press may do to it is `settle_dispatch`.
/// The alternatives, and why a count is not one of them, are in
/// `docs/adr/0001-a-dispatch-attempt-owns-what-it-wrote.md`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct AttemptId(u64);

impl AttemptId {
    /// Distinct for the life of the process, which is the life of every set that holds one. The
    /// nonce `reserve_dispatch_id` puts in a `DispatchId` counts something else — fresh ids minted,
    /// not presses made — and the two deliberately do not meet: this identity is private to the
    /// desktop, and that one goes on the wire.
    fn next() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        AttemptId(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

/// The presses dispatching one Job, and the id each of them has been answered about.
///
/// Both halves are needed to decide a clear. A press still in flight and not yet answered is a
/// press whose reply may still be lost, so the id it went out under has to stay; and an answer
/// naming an id the entry no longer holds — pruned and re-minted mid-flight — settles that id and
/// not the entry's. So the entry goes when every press in flight has been answered and one of
/// those answers was about the id the entry still holds.
///
/// Recording the answers rather than acting on each alone is what stops two presses that both
/// settle from each deferring to the other and leaving the Job in doubt with nothing outstanding.
#[derive(Default)]
struct Presses {
    in_flight: HashSet<AttemptId>,
    answered: HashMap<AttemptId, cut_host::protocol::DispatchId>,
}

/// A press counted as dispatching for exactly as long as it is in flight.
///
/// A guard rather than a pair of calls because the registration is what stops a *sibling* press's
/// answer from clearing this one's id, so an early return or a panic that skipped the removal
/// would leave a Job no answer could settle, and every press until `ID_RETENTION` expires the
/// entry read as a retry rather than cut.
///
/// What Drop deliberately does *not* do is clear anything. The mark on the cutter outlives the
/// press by design — an accepted dispatch whose reply was lost is exactly what the close guard
/// must still warn about — and an unsettled press's id outlives it for the same reason. Only an
/// answer clears, and only through `settle_dispatch`.
#[must_use = "the press stops counting as dispatching as soon as this is dropped"]
struct Dispatching<'a> {
    dev: &'a DeviceManagerHandle,
    key: JobKey,
    id: AttemptId,
}

impl Drop for Dispatching<'_> {
    fn drop(&mut self) {
        // `into_inner` rather than `unwrap`, as everywhere this repo locks in a `Drop`: a panic
        // here while unwinding aborts the process, so the one path that most needs the press
        // deregistered would instead take the editor down.
        let mut dispatching = self.dev.dispatching.lock().unwrap_or_else(|e| e.into_inner());
        let Some(presses) = dispatching.get_mut(&self.key) else {
            // Unreachable: this press's own id keeps the entry non-empty, and only a press's Drop
            // removes an entry. Asserted rather than trusted, because the state it would leave is
            // silent and durable — a Job no answer can settle.
            debug_assert!(false, "a press's registration went missing while it was in flight");
            return;
        };
        presses.in_flight.remove(&self.id);
        presses.answered.remove(&self.id);
        if presses.in_flight.is_empty() {
            dispatching.remove(&self.key);
        }
    }
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
            dispatching: Mutex::new(HashMap::new()),
            last_remote_status: Mutex::new(HashMap::new()),
            remote_dispatched: Mutex::new(HashMap::new()),
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
            let Some(client) = host.ensure() else { continue };
            // Read before the call and compared after, as in `status`: a dispatch that lands while
            // this is in flight makes what it learned stale, and the stale answer must not be what
            // clears the mark that dispatch just made.
            let epoch = self.dispatch_epoch.load(std::sync::atomic::Ordering::SeqCst);
            match client.snapshots() {
                Ok(snapshots) => {
                    if self.dispatch_epoch.load(std::sync::atomic::Ordering::SeqCst) == epoch {
                        let mut dispatched = self.remote_dispatched.lock().unwrap();
                        for free in snapshots.iter().filter(|s| s.status.actions.cut) {
                            // Every press's mark on that cutter, on the cutter's own authority —
                            // the same act `status` performs for the aimed one, and for the same
                            // reason: it says it would take another Job.
                            dispatched.remove(&(id.clone(), free.info.instance_id.clone()));
                        }
                    }
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
        let conn = HostConnection { paired, client: None, last_error: None };
        self.hosts.lock().unwrap().insert(id, Arc::new(Mutex::new(conn)));
    }

    /// Drops the host and the status its cutters last reported.
    ///
    /// The cache goes because host ids are recycled — `hosts::next_id` counts past the surviving
    /// hosts — so a Pi paired after this one is forgotten can inherit its name, and with it a
    /// `Cutting` reading from another machine that would hold the window until the first poll of
    /// the new one.
    ///
    /// The dispatch marks are *not* dropped here, because which of them are stale is not something
    /// this verb knows: `forget` asks the host and retracts the marks that answer covered, which
    /// leaves a dispatch accepted after the answer still holding the window (see `retract_marks`).
    /// The in-doubt ids for the host's Jobs stay too, on `ID_RETENTION`: an unreachable host may
    /// still remember an id, and a re-pair inside the hour must not mint a fresh name for a Job it
    /// has.
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
        // Read before the host is asked, so the retraction below covers exactly the presses this
        // answer is about. A dispatch accepted while the question was in flight is not one the
        // answer speaks for, and its mark stays: the warning is the last thing this desktop can
        // still offer for a Job whose cancel route it is about to discard.
        let marks = self.marks_for(id);
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
        // A host that answered idle has just said those marks are stale; a host forced past
        // unreachable had the operator told outright what forgetting discards. Either way the marks
        // go, because the poll that would otherwise clear them is a call to the host this just
        // removed — a mark left behind is a quit prompt nothing can ever stop raising.
        self.retract_marks(&marks);
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
                if let Err(
                    e @ (cut_host::client::ClientError::Transport(_)
                    | cut_host::client::ClientError::WrongReply { .. }
                    | cut_host::client::ClientError::WrongDispatch),
                ) = &out
                {
                    // A connection that broke mid-call stays broken, and `ensure` only redials
                    // when there is no client at all — so leaving this one in place fails every
                    // later verb on this host against the same dead socket. `list_devices` has
                    // always dropped it here; the call that most needs the same is the retry
                    // after a lost reply, which is by definition made on a connection that just
                    // failed (see `execute_cut`).
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
                            //
                            // Every press's mark goes: the cutter has answered for the machine, so
                            // no press's record of having started something there survives it.
                            // `list_devices` performs the same clear for every cutter it hears
                            // from, which is what makes a mark on an un-aimed cutter releasable at
                            // all. Neither is a clear a press owns, and that is the reason a mark
                            // is attributed rather than counted — a retraction arriving after this
                            // must not take a later press's mark with it.
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
        self.remote_dispatched.lock().unwrap().values().any(|presses| !presses.is_empty())
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
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            None | Some(Route::Local) => {
                Ok(self.manager()?.resume()?)
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
                Ok(self.manager()?.confirm_pass_done()?)
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
        let planned = plan_passes_with(&app.editor.doc, request.grouping)
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
            Route::Local => Ok(CutStarted { job_id: self.manager()?.cut(passes)?, duplicate: false }),
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
                // Chosen and written down in one step, before any network call. Choosing and
                // recording separately let two presses of Cut both find nothing and mint an id
                // each: if the first reached the host and lost its reply, the second's entry
                // replaced the only record of it, and the retry that should have been recognised
                // went out under a name the host had never seen.
                // `first_attempt` is information, not ownership, and it is narrower than it looks:
                // `true` says only that no entry existed for this Job before this call. So `false`
                // means an entry stood already — usually an earlier dispatch nothing settled, and
                // sometimes an earlier press that never reached a host. What it changes is what a
                // failure here means. It is deliberately not used to decide whether to undo the
                // reservation.
                //
                // Ownership is `attempt`: the press is counted as dispatching and the id chosen in
                // one call, so no sibling's settling answer can land between the two and clear the
                // entry this press is about to dispatch under (#290).
                let (attempt, dispatch_id, first_attempt) = self.begin_dispatch(&key);
                // Marked before the request rather than after the answer: an accepted dispatch
                // whose reply is lost is exactly the case the window-close guard must still warn
                // about, and by then there is nothing to write it from.
                self.mark_dispatched(&id, &device, attempt.id);
                // Bumped either side of the call, not just before it. A poll that begins after the
                // mark but before the host has the Job sees a cutter that would still take one,
                // and clearing on that reading throws away the mark the dispatch just made. Only a
                // poll that ran entirely after the dispatch finished sees an unchanged count.
                self.dispatch_epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                let reached_the_host = std::sync::atomic::AtomicBool::new(false);
                let answer_settled = std::sync::atomic::AtomicBool::new(false);
                let sent = self.with_host(&id, |c| {
                    reached_the_host.store(true, std::sync::atomic::Ordering::SeqCst);
                    let sent = c.dispatch(&dispatch_id, &device, &machine_id, passes);
                    // Cleared by an answer that settles what the host did — a refusal included,
                    // since a host that refused says it started nothing. A reply this request
                    // cannot use settles nothing: it arrived, so nothing was lost, but it does not
                    // say whether the Job began, so the entry stays.
                    //
                    // Keeping it is the only thing that leaves host-side deduplication a chance: a
                    // fresh id matches nothing the host has seen, so nothing there can read the
                    // next press as this dispatch again. The same id may still be read as a new
                    // Job — a host forgets ids past `ID_RETENTION` and past `MAX_REMEMBERED_IDS`,
                    // and a peer answering outside the protocol may dedupe nothing at all — which
                    // is why the refusal below is `unconfirmed` rather than an all-clear. Clearing
                    // the entry would remove the chance rather than improve on it.
                    //
                    // A reply about another dispatch stays for the same reason and with less
                    // known, not more: a wrong variant at least came from a peer answering this
                    // request badly, while an `Accepted` naming another Job says nothing about
                    // this one at all (#285).
                    let settled = !matches!(
                        sent,
                        Err(cut_host::client::ClientError::Transport(_)
                            | cut_host::client::ClientError::WrongReply { .. }
                            | cut_host::client::ClientError::WrongDispatch)
                    );
                    if settled {
                        self.settle_dispatch(&key, &dispatch_id, attempt.id);
                    }
                    // The same fact decides what the operator is told below, which is why it is
                    // carried out rather than re-derived there: an answer that settled what the
                    // host did is the whole of what happened to this attempt (#288).
                    answer_settled.store(settled, std::sync::atomic::Ordering::SeqCst);
                    // A refusal is the host saying it started nothing, so it is not something the
                    // close guard should hold the window for — this press's mark, and no other.
                    // A sibling press of the same Job may have been accepted and be cutting right
                    // now, and this answer is not about that one (#290).
                    if matches!(sent, Err(cut_host::client::ClientError::Refused(_))) {
                        self.unmark_dispatched(&id, &device, attempt.id);
                    }
                    sent
                });
                self.dispatch_epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                if !reached_the_host.load(std::sync::atomic::Ordering::SeqCst) {
                    // Nothing was started by *this* press, so nothing of this press's holds the
                    // window. Another press's mark is untouched, for the reason above.
                    self.unmark_dispatched(&id, &device, attempt.id);
                    // The reservation stays, and now by choice rather than for want of knowing
                    // whose it is: `first_attempt` says whether the entry is this call's, and
                    // `settle_dispatch` could clear it on the same terms as an answered dispatch.
                    // Neither is asked to, because there is nothing to gain — a dispatch that never
                    // reached a host is one the host cannot have seen either, so the next Cut
                    // sending this id is cut normally rather than deduplicated away. What it costs
                    // is that an identical Cut pressed later is reported as a duplicate rather than
                    // cut, which is the safe direction and the one #121 asked to be visible.
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
                    // Whether this call left anything of this Job outstanding. An answer that said
                    // what the host did settles it, and a call that never reached a host settles it
                    // too — there was nothing there to have started anything.
                    //
                    // Nothing outstanding at all, and the failure is the whole of what happened, so
                    // it stays the plain error it is. Both halves matter, and a refusal is why the
                    // second one is not enough on its own: it settles this attempt while saying
                    // nothing about an earlier one, so it is plain only when there was no earlier
                    // one (#288). A first press against an offline Pi must not claim the Job may be
                    // cutting; a press made while an earlier dispatch is unsettled must.
                    Err(e)
                        if first_attempt
                            && (!reached_the_host.load(std::sync::atomic::Ordering::SeqCst)
                                || answer_settled.load(std::sync::atomic::Ordering::SeqCst)) =>
                    {
                        Err(e)
                    }
                    // What the next press can do about it is a fact about this desktop's own
                    // record, so it is asked of the record rather than inferred from the failure —
                    // and asked as the *next press* will answer it, since an entry this one is
                    // reusing can age out of reuse while the call it is on is still in flight.
                    Err(e) => Err(unconfirmed(&e, self.retry_would_reuse(&key, std::time::Instant::now()))),
                }
            }
        }
    }

    /// Register this press as dispatching `key`, and choose the id it goes out under.
    ///
    /// One call rather than two, because the order is the invariant: a press that chose an id
    /// before it was counted could have a sibling's settling answer land in the gap, clear the
    /// entry it is about to dispatch under, and leave a later retry sending a name the host has
    /// never seen. Two statements can be swapped by an edit; one cannot.
    ///
    /// The `bool` is `reserve_dispatch_id`'s: `true` means this call is what recorded the entry.
    fn begin_dispatch(&self, key: &JobKey) -> (Dispatching<'_>, cut_host::protocol::DispatchId, bool) {
        let id = AttemptId::next();
        // Held across the reservation, in the order this pair of locks is declared in: a sibling's
        // answer cannot run between the two halves and find nobody dispatching.
        let mut dispatching = self.dispatching.lock().unwrap();
        dispatching.entry(key.clone()).or_default().in_flight.insert(id);
        let (dispatch_id, first_attempt) = self.reserve_dispatch_id(key);
        drop(dispatching);
        (Dispatching { dev: self, key: key.clone(), id }, dispatch_id, first_attempt)
    }

    /// Account for what this press was told, and drop the Job's in-doubt entry once every press in
    /// flight has been told something that settles it.
    ///
    /// A settling answer is the host saying what it did with `id`, which is the whole of what this
    /// press has to account for — and none of what a *sibling* press dispatching the same id has:
    /// its reply may still be lost, and this entry is what its retry would go out under (#290). So
    /// the answer is recorded rather than acted on alone, and whichever press finds that no
    /// unanswered press is left is the one that clears. In the ordinary single-press case that is
    /// this press, in this call.
    ///
    /// A press that ends *without* a settling answer takes its record away with it, so it can never
    /// be the press that satisfies this — which is the point: something may be cutting under that
    /// id, and the entry is what a retry would send.
    ///
    /// The id is compared as well as the Job, because the entry can be pruned and re-minted while
    /// a dispatch is in flight (`still_reusable`, `MAX_JOBS_IN_DOUBT`): a fresh entry under this
    /// key belongs to whoever minted it, and an answer about the id it replaced says nothing about
    /// it. So one of the answers has to name the id the entry actually holds.
    fn settle_dispatch(&self, key: &JobKey, id: &cut_host::protocol::DispatchId, attempt: AttemptId) {
        // Both locks, in the order `dispatching` declares: asking and clearing have to be one
        // step, or a press beginning between them joins an entry this call is about to delete —
        // and its retry then goes out under a name the host has never seen, which is the failure
        // the whole entry exists to prevent.
        let mut dispatching = self.dispatching.lock().unwrap();
        let mut in_doubt = self.in_doubt.lock().unwrap();
        let Some(presses) = dispatching.get_mut(key) else { return };
        presses.answered.insert(attempt, id.clone());
        let every_press_answered = presses.in_flight.iter().all(|p| presses.answered.contains_key(p));
        let Some((held, _)) = in_doubt.get(key) else { return };
        let held_id_answered = presses.answered.values().any(|answered| answered == held);
        if every_press_answered && held_id_answered {
            in_doubt.remove(key);
        }
    }

    fn mark_dispatched(&self, host: &HostId, device: &str, attempt: AttemptId) {
        self.remote_dispatched
            .lock()
            .unwrap()
            .entry((host.clone(), device.to_owned()))
            .or_default()
            .insert(attempt);
    }

    /// Retract this press's mark, having learned the host started nothing for it.
    ///
    /// Only this press's: the mark is what raises the quit prompt, and a refusal answers for the
    /// dispatch it refused rather than for another press that was accepted and is cutting (#290).
    /// A cutter's own "I would take a Job now" is the answer that clears them all, in `status` and
    /// `list_devices`.
    fn unmark_dispatched(&self, host: &HostId, device: &str, attempt: AttemptId) {
        let mut dispatched = self.remote_dispatched.lock().unwrap();
        let key = (host.clone(), device.to_owned());
        if let Some(presses) = dispatched.get_mut(&key) {
            presses.remove(&attempt);
            // An empty set and no entry mean the same thing, so only one spelling is stored: the
            // map holds a cutter for exactly as long as some press holds it.
            if presses.is_empty() {
                dispatched.remove(&key);
            }
        }
    }

    /// Drop exactly the marks named, having learned the presses that wrote them started nothing.
    ///
    /// The retraction `forget` makes, and deliberately narrower than "every mark for that host":
    /// what makes them stale is the host's own answer that it is idle, and a dispatch accepted
    /// after that answer is not covered by it. That press keeps its mark and the operator keeps the
    /// warning — which is all this desktop can still offer, having just discarded the token that
    /// could have stopped it.
    fn retract_marks(&self, known: &[((HostId, String), AttemptId)]) {
        for ((host, device), attempt) in known {
            self.unmark_dispatched(host, device, *attempt);
        }
    }

    /// Every mark this desktop is holding for `host`, as the presses that wrote them.
    fn marks_for(&self, host: &HostId) -> Vec<((HostId, String), AttemptId)> {
        self.remote_dispatched
            .lock()
            .unwrap()
            .iter()
            .filter(|((h, _), _)| h == host)
            .flat_map(|(key, presses)| presses.iter().map(|p| (key.clone(), *p)).collect::<Vec<_>>())
            .collect()
    }

    /// The id this Job goes out under — the one its unsettled dispatch already used, or a fresh
    /// one — recorded as in doubt in the same breath. `true` means this call is what recorded it,
    /// which is what tells a caller that never reached the host that the entry it would be
    /// discarding is its own and not an *earlier* dispatch's. Read rather than acted on:
    /// `execute_cut` uses it only to decide what the operator is told, because a never-reached
    /// dispatch's entry is harmless to leave (see the comment there).
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
        // were, and `still_reusable` is where that is decided — see there for why the boundary is
        // the host's own constant. This is *not* the fifteen-minute expiry that was removed; that
        // one discarded protection while it still existed.
        in_doubt.retain(|_, (_, written)| still_reusable(*written, now));
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

    /// Whether a press made now would send this Job out under the id an unsettled dispatch already
    /// used, which is the only thing that gives a repeated Cut a chance of being recognised rather
    /// than cut again.
    ///
    /// The retention rule lives here and `reserve_dispatch_id` prunes by it, so the sentence an
    /// unconfirmed dispatch carries and the id the next press actually mints answer the same
    /// question. A second copy of that rule would be a second policy, and the two disagreeing is
    /// exactly how a promise becomes false: an entry can cross `ID_RETENTION` while the call
    /// offering the retry is still in flight, and `contains_key` alone would offer to reuse an id
    /// the next press is about to prune (#286).
    ///
    /// `now` is a parameter because the boundary is the whole of the behaviour, and a caller that
    /// can only pass the present cannot ask about either side of it.
    fn retry_would_reuse(&self, key: &JobKey, now: std::time::Instant) -> bool {
        self.in_doubt
            .lock()
            .unwrap()
            .get(key)
            .is_some_and(|(_, written)| still_reusable(*written, now))
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
}

/// Summarizes `plan_passes_with` output for the UI — not the raw `DocumentPasses`
/// (which carries full flattened polylines the cut dialog doesn't need).
///
/// Takes the grouping rather than defaulting it: unlike `cutplan::plan_passes`, this has no
/// caller that means "whatever the default is" — the dialog always has a mode selected.
pub fn plan_cut_response(doc: &document::Document, grouping: Grouping)
    -> Result<PlanCutResponse, IpcError> {
    let planned = plan_passes_with(doc, grouping)
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
    configured: &[TravelPassDto],
) -> Result<Vec<[f64; 4]>, IpcError> {
    // Same rule as `prepare_cut`: a revision string that isn't a u64 was never
    // issued by `doc_revision`, so it cannot be the current plan.
    let Ok(expected) = doc_revision.parse::<u64>() else {
        return Err(IpcError::new("stale_plan", "travel request carries an unrecognized plan revision"));
    };
    let planned = plan_passes_with(doc, grouping)
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
        plan_passes_with(&app.editor.doc, Grouping::Color).unwrap()
    }

    fn request_from(plan: DocumentPasses) -> CutRequest {
        CutRequest {
            device_instance_id: test_instance().instance_id,
            doc_revision: plan.doc_revision.to_string(),
            // The mode the passes were planned under. `plan_for` uses colour grouping, so
            // this must too, or every request here would be refused as an unknown key.
            grouping: Grouping::Color,
            passes: plan.passes.iter().map(|p| ConfiguredPassDto {
                key: p.key.clone(), enabled: true, preset_id: None,
                speed: None, force: None, repeat_count: None,
            }).collect(),
        }
    }

    fn colour(c: u32) -> PassKey { PassKey::Color(Some(c)) }

    const GREEN: u32 = 0x00FF00FF;

    /// Paint an existing shape, so a fixture can carry a stroke and a fill that key
    /// differently under `Stroke` and `Fill`.
    fn paint(app: &mut AppState, id: document::NodeId, stroke: Option<u32>, fill: Option<u32>) {
        let before = app.editor.doc.get(id).unwrap().clone();
        let mut after = before.clone();
        after.style = document::Style { stroke, fill };
        app.editor.doc.apply(document::Delta(vec![document::NodeOp::Update { id, before, after }]));
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
        let request = CutRequest { device_instance_id: test_instance().instance_id,
            doc_revision: revision.to_string(), grouping: Grouping::Color, passes: vec![] };
        let err = dev.cut_from_request(&app, request).unwrap_err();
        assert_eq!(err.code, "nothing_to_cut");
    }

    #[test]
    fn an_unknown_pass_is_rejected_not_dropped() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        let mut request = request_from(plan);
        request.passes[0].key = colour(0xDEADBEEF); // doesn't match any planned pass
        let err = dev.cut_from_request(&app, request).unwrap_err();
        assert_eq!(err.code, "unknown_pass");
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

    fn on(key: PassKey) -> TravelPassDto { TravelPassDto { key, enabled: true } }
    fn off(key: PassKey) -> TravelPassDto { TravelPassDto { key, enabled: false } }

    #[test]
    fn travel_for_order_follows_the_requested_order() {
        let (app, revision) = two_color_doc();

        // The plan's own first-seen order reproduces exactly what plan_cut_response sent.
        let planned = travel_for_order(&app.editor.doc, &revision, Grouping::Color, &[on(colour(RED)), on(colour(BLUE))]).unwrap();
        assert_eq!(planned, plan_cut_response(&app.editor.doc, Grouping::Color).unwrap().travel);
        assert_eq!(planned.len(), 1);
        assert!(planned[0][2] >= 100.0, "red first: travel lands on the blue rect at x=100, got {planned:?}");

        let reversed = travel_for_order(&app.editor.doc, &revision, Grouping::Color, &[on(colour(BLUE)), on(colour(RED))]).unwrap();
        assert_eq!(reversed.len(), 1);
        assert!(reversed[0][0] >= 100.0 && reversed[0][2] <= 10.0,
            "blue first: travel leaves x=100 for the red rect at the origin, got {reversed:?}");
    }

    /// The head does not travel to a pass that will not be cut. A disabled pass is still
    /// named, so the completeness check below survives.
    #[test]
    fn travel_for_order_skips_a_disabled_pass() {
        let (app, revision) = two_color_doc();
        let travel = travel_for_order(&app.editor.doc, &revision, Grouping::Color, &[on(colour(RED)), off(colour(BLUE))]).unwrap();
        assert!(travel.is_empty(), "nothing to travel to with only one pass cut, got {travel:?}");

        // And with everything off there is no motion at all.
        let none = travel_for_order(&app.editor.doc, &revision, Grouping::Color, &[off(colour(RED)), off(colour(BLUE))]).unwrap();
        assert!(none.is_empty(), "no pass is cut, so the head does not move: {none:?}");
    }

    #[test]
    fn travel_for_order_with_a_stale_revision_is_refused() {
        let (mut app, revision) = two_color_doc();
        app.add_rect(5.0, 5.0);
        let err = travel_for_order(&app.editor.doc, &revision, Grouping::Color, &[on(colour(RED)), on(colour(BLUE))]).unwrap_err();
        assert_eq!(err.code, "stale_plan");
    }

    #[test]
    fn travel_for_order_with_an_unknown_key_is_refused() {
        let (app, revision) = two_color_doc();
        let unknown = TravelPassDto { key: colour(0xDEADBEEF), enabled: true };
        let err = travel_for_order(&app.editor.doc, &revision, Grouping::Color, &[on(colour(RED)), unknown]).unwrap_err();
        assert_eq!(err.code, "unknown_pass");
    }

    /// Disabling a pass drops it from the travel, never from the list — a pass genuinely
    /// missing is a frontend bug, and silently drawing travel around it would hide it.
    #[test]
    fn travel_for_order_missing_a_planned_pass_is_refused() {
        let (app, revision) = two_color_doc();
        let err = travel_for_order(&app.editor.doc, &revision, Grouping::Color, &[on(colour(RED))]).unwrap_err();
        assert_eq!(err.code, "plan_mismatch");
    }

    #[test]
    fn travel_for_order_naming_a_pass_twice_is_a_mismatch_not_an_unknown_pass() {
        let (app, revision) = two_color_doc();
        let err = travel_for_order(&app.editor.doc, &revision, Grouping::Color, &[on(colour(RED)), on(colour(RED))]).unwrap_err();
        assert_eq!(err.code, "plan_mismatch");
    }

    #[test]
    fn plan_cut_response_carries_each_shapes_first_world_point() {
        let (app, _) = two_color_doc();
        let response = plan_cut_response(&app.editor.doc, Grouping::Color).unwrap();
        for pass in &response.passes {
            assert_eq!(pass.starts.len(), pass.node_ids.len(), "starts is parallel to node_ids");
        }
        // The blue rect's translate must show in its start — a local-space point here
        // would put the badge at the origin instead of on the shape.
        let blue = response.passes.iter().find(|p| p.key == colour(BLUE)).unwrap();
        let start = blue.starts[0].unwrap();
        assert!(start[0] >= 100.0, "world-space start, got {start:?}");
    }

    /// The grouping the dialog asked for is the grouping that gets cut. Without it the
    /// operator could preview a fill-grouped plan and cut a stroke-grouped one, because each
    /// command plans the document itself.
    #[test]
    fn a_cut_honours_the_grouping_it_was_sent() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        // A red stroke over a green fill: the two colour modes key this shape differently,
        // so the request's grouping is observable in what matches.
        let id = app.add_rect(10.0, 10.0);
        paint(&mut app, id, Some(RED), Some(GREEN));
        let revision = plan_cut_response(&app.editor.doc, Grouping::Fill).unwrap().doc_revision;

        let request = CutRequest {
            device_instance_id: test_instance().instance_id,
            doc_revision: revision,
            grouping: Grouping::Fill,
            passes: vec![ConfiguredPassDto {
                key: colour(RED), enabled: true, preset_id: None,
                speed: None, force: None, repeat_count: None }],
        };
        // Fill grouping keys that shape on its fill, so the stroke's key names nothing.
        assert_eq!(dev.cut_from_request(&app, request).unwrap_err().code, "unknown_pass");
    }

    /// Travel is replanned with the same grouping, for the same reason.
    #[test]
    fn travel_honours_the_grouping_it_was_sent() {
        let mut app = AppState::new();
        let id = app.add_rect(10.0, 10.0);
        paint(&mut app, id, Some(RED), Some(GREEN));
        let revision = plan_cut_response(&app.editor.doc, Grouping::Fill).unwrap().doc_revision;

        assert!(travel_for_order(&app.editor.doc, &revision, Grouping::Fill,
            &[on(colour(GREEN))]).is_ok());
        assert_eq!(travel_for_order(&app.editor.doc, &revision, Grouping::Fill,
            &[on(colour(RED))]).unwrap_err().code, "unknown_pass");
    }

    /// The response names its passes in the spelling a request must send back.
    #[test]
    fn a_plan_response_names_its_passes_by_key() {
        let mut app = AppState::new();
        app.add_rect(10.0, 10.0);
        let response = plan_cut_response(&app.editor.doc, Grouping::Single).unwrap();
        assert_eq!(response.passes[0].key, PassKey::All);
    }

    /// A preset-keyed pass is cut with that preset's settings. This is the whole point of
    /// grouping by material: `prepare_cut` reads only `preset_id`, so a row that arrives with
    /// none is cut with defaults no matter what its key says.
    #[test]
    fn a_preset_keyed_pass_cuts_with_that_presets_settings() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        let id = app.add_rect(10.0, 10.0);
        app.set_material_preset(vec![id], document::PresetAssignment::Preset("cameo5-htv".into()))
            .expect("assignable");
        let revision = plan_cut_response(&app.editor.doc, Grouping::Preset).unwrap().doc_revision;

        let request = CutRequest {
            device_instance_id: test_instance().instance_id,
            doc_revision: revision,
            grouping: Grouping::Preset,
            passes: vec![ConfiguredPassDto {
                key: PassKey::Preset(Some("cameo5-htv".into())),
                enabled: true,
                // What the dialog sends for a preset-keyed row: the key's own id.
                preset_id: Some("cameo5-htv".into()),
                speed: None, force: None, repeat_count: None }],
        };
        let (_, passes) = dev.prepare_cut(&app, request).expect("planned");
        let builtin = cutplan::presets::builtin_presets().into_iter()
            .find(|p| p.id == "cameo5-htv").expect("premise: the builtin exists");
        assert_eq!(passes[0].job.settings.speed, builtin.settings.speed);
        assert_eq!(passes[0].job.settings.force, builtin.settings.force);
    }

    /// Greptile's P1 on PR #152 read this as "a preset key silently uses defaults". It is the
    /// operator clearing the preset on a row: the key says which shapes share the pass, the row's
    /// `preset_id` says what settings to cut them with, and those are deliberately two fields. A
    /// colour-keyed pass has no preset either and cuts with the operator's settings - the ordinary
    /// case nobody calls a bug. Refusing here would make preset-keyed passes the only kind that
    /// cannot be cut with settings of the operator's own choosing.
    ///
    /// It is not silent: the row shows "No preset" where it showed the material, and the speed and
    /// force fields show what will be used. The refusal below is for an id that *cannot resolve* -
    /// the file changing under the operator - which is a different fact from a choice.
    #[test]
    fn a_preset_keyed_pass_with_the_preset_cleared_cuts_with_the_operators_settings() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        let id = app.add_rect(10.0, 10.0);
        app.set_material_preset(vec![id], document::PresetAssignment::Preset("cameo5-htv".into()))
            .expect("assignable");
        let revision = plan_cut_response(&app.editor.doc, Grouping::Preset).unwrap().doc_revision;

        let request = CutRequest {
            device_instance_id: test_instance().instance_id,
            doc_revision: revision,
            grouping: Grouping::Preset,
            passes: vec![ConfiguredPassDto {
                key: PassKey::Preset(Some("cameo5-htv".into())),
                enabled: true,
                // The operator picked "No preset" on a row keyed by a material.
                preset_id: None,
                speed: Some(7), force: None, repeat_count: None }],
        };
        let (_, passes) = dev.prepare_cut(&app, request).expect("planned, not refused");
        let builtin = cutplan::presets::builtin_presets().into_iter()
            .find(|p| p.id == "cameo5-htv").expect("premise: the builtin exists");
        assert_eq!(passes[0].job.settings.speed, Some(7), "the operator's own speed is used");
        // Force is what discriminates, and speed cannot: an override beats a preset either way
        // (`presets.rs:44`), so a speed assertion passes even if the preset were wrongly
        // re-derived from the key. Force was left unset, so it says which of the two happened.
        assert!(builtin.settings.force.is_some(),
            "premise: HTV states a force, so re-deriving it from the key would show here");
        assert_eq!(passes[0].job.settings.force, None,
            "no preset was selected, so no preset's force is applied");
    }

    /// A pass naming a preset the file cannot resolve is refused rather than cut with
    /// defaults. Greptile's P1 on PR #152: a machine-scoped preset disappears for ordinary
    /// reasons — the project was converted, the entry was deleted — and cutting real material
    /// with settings unrelated to the pass's own name is the failure that costs a sheet.
    #[test]
    fn a_pass_naming_an_unavailable_preset_is_refused_not_defaulted() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        let id = app.add_rect(10.0, 10.0);
        app.set_material_preset(vec![id], document::PresetAssignment::Preset("deleted-by-hand".into()))
            .expect("assignable");
        let revision = plan_cut_response(&app.editor.doc, Grouping::Preset).unwrap().doc_revision;

        let request = CutRequest {
            device_instance_id: test_instance().instance_id,
            doc_revision: revision,
            grouping: Grouping::Preset,
            passes: vec![ConfiguredPassDto {
                key: PassKey::Preset(Some("deleted-by-hand".into())),
                enabled: true,
                preset_id: Some("deleted-by-hand".into()),
                speed: None, force: None, repeat_count: None }],
        };
        let err = dev.prepare_cut(&app, request).unwrap_err();
        assert_eq!(err.code, "unknown_preset");
        assert!(err.message.contains("deleted-by-hand"), "names the preset it cannot find: {}", err.message);
    }

    /// The fail-open Codex found on the gate re-run, from the other end: a pass keyed `preset:`
    /// carries an *empty* preset id, and an empty id resolves to no preset. The refusal has to
    /// fire — before this, the TypeScript mirror dropped the id, the request named no preset at
    /// all, and `prepare_cut` skipped its lookup and produced default speed and force.
    #[test]
    fn a_pass_keyed_on_an_empty_preset_id_is_refused_not_defaulted() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let revision = plan_cut_response(&app.editor.doc, Grouping::Preset).unwrap().doc_revision;

        let request = CutRequest {
            device_instance_id: test_instance().instance_id,
            doc_revision: revision,
            grouping: Grouping::Preset,
            passes: vec![ConfiguredPassDto {
                // What the dialog sends for a `preset:` row now that both grammars parse it.
                key: PassKey::Preset(Some(String::new())),
                enabled: true,
                preset_id: Some(String::new()),
                speed: None, force: None, repeat_count: None }],
        };
        let err = dev.prepare_cut(&app, request).unwrap_err();
        assert_eq!(err.code, "unknown_preset",
            "an empty id names no material, so the cut is refused rather than defaulted");
    }

    /// A preset belonging to another cutter is not this cut's preset. Greptile's P1 on the third
    /// push: `load_presets` returns every machine's entries, and while builtin ids are
    /// machine-prefixed, a *user* preset's id is the operator's own string — so a Puma entry
    /// could resolve for a Cameo cut and hand it that machine's speed and force.
    ///
    /// `puma-htv` stands in for the collision: it is a real builtin for the other machine, so
    /// the lookup finds it if and only if the machine filter is missing. (Greptile could not run
    /// this itself — its container lacked `gdk-3.0.pc` — so it is written here from its
    /// finding.)
    #[test]
    fn a_preset_owned_by_a_different_machine_is_refused() {
        let mut app = AppState::new();
        let dev = test_device_setup(); // connects `cameo5`
        let id = app.add_rect(10.0, 10.0);
        app.set_material_preset(vec![id], document::PresetAssignment::Preset("puma-htv".into()))
            .expect("assignable");
        let revision = plan_cut_response(&app.editor.doc, Grouping::Preset).unwrap().doc_revision;

        let request = CutRequest {
            device_instance_id: test_instance().instance_id,
            doc_revision: revision,
            grouping: Grouping::Preset,
            passes: vec![ConfiguredPassDto {
                key: PassKey::Preset(Some("puma-htv".into())),
                enabled: true,
                preset_id: Some("puma-htv".into()),
                speed: None, force: None, repeat_count: None }],
        };
        let err = dev.prepare_cut(&app, request).unwrap_err();
        assert_eq!(err.code, "unknown_preset",
            "a Puma preset must not supply settings for a Cameo cut");

        // Premise: that id really is a preset — just not one for this machine.
        assert!(cutplan::presets::builtin_presets().iter().any(|p| p.id == "puma-htv"),
            "premise: puma-htv is a builtin, so only the machine filter can refuse it");
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

    /// The wrong-state refusal, which needs no hardware to reach: `resume` with nothing parked.
    /// It used to arrive as `device_error` with `Busy` inside a `Debug` string, indistinguishable
    /// to a caller from a cable pull — the two things this issue exists to separate (#73).
    #[test]
    fn a_verb_the_cutter_cannot_accept_reports_the_busy_code() {
        let dev = test_device_setup();
        let err = dev.resume().unwrap_err();
        assert_eq!(err.code, "device_busy", "got {err:?}");
        assert_eq!(err.message, "the cutter cannot do that right now");
    }

    /// A cutter whose transport takes the worker thread down as it opens.
    ///
    /// The only way to reach `Disconnected`'s worker-gone site from out here:
    /// `DeviceManagerHandle::shutdown` clears the stored manager, so a verb after it reports
    /// `shut_down` rather than reaching a dead worker at all, and `DeviceManager`'s own command
    /// channel is private to `driver-core`.
    struct DeadWorkerFactory;
    impl DeviceBackendFactory for DeadWorkerFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> { vec![test_instance()] }
        fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
            TestFactory.driver_for(machine_id)
        }
        fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            panic!("a transport that takes the worker thread with it")
        }
    }

    /// The other half of the code the eight sites collapsed: nothing refused this verb, there is
    /// simply no worker left to answer it. The panic the harness prints is this test working.
    #[test]
    fn a_verb_with_no_worker_left_reports_the_disconnected_code() {
        let (dev, _events) = DeviceManagerHandle::new(Arc::new(DeadWorkerFactory));
        let err = dev.connect(test_instance()).unwrap_err();
        assert_eq!(err.code, "device_disconnected", "got {err:?}");
        assert_eq!(err.message, "the cutter is not connected");
    }

    /// A cutter jamming, unplugged or asked for a verb it cannot do must read the same whether it
    /// hangs off this laptop or off a Pi. The Cut Host route carries a real `DeviceError` across
    /// the wire and `host_error` used to fold it — with every other refusal — into
    /// `host_refused`, so the identical fault had two codes and two sentences (#73).
    #[test]
    fn a_device_fault_from_a_host_reports_what_the_same_fault_reports_locally() {
        use cut_host::client::ClientError;
        use cut_host::protocol::Refusal;

        let dev = test_device_setup();
        let local = dev.resume().unwrap_err();
        let remote = host_error(&ClientError::Refused(Refusal::Device(
            driver_core::manager::DeviceError::Busy,
        )));
        assert_eq!((remote.code, remote.message), (local.code, local.message));

        // And the arm is not a catch-all: a refusal that is the host's own keeps the host's code.
        let its_own = host_error(&ClientError::Refused(Refusal::UnknownDevice("usb:1:4".into())));
        assert_eq!(its_own.code, "host_refused", "got {its_own:?}");
    }

    /// A host that answered was reached, and the code has to say so. A wrong reply used to arrive
    /// as `host_unreachable` with a `Debug` rendering of the reply in the message, so the code
    /// claimed the opposite of what had just happened (#283).
    #[test]
    fn a_reply_a_verb_cannot_use_is_not_reported_as_an_unreachable_host() {
        use cut_host::client::ClientError;

        let err = host_error(&ClientError::WrongReply { expected: "Snapshots", found: "Devices" });
        assert_eq!(err.code, "host_wrong_reply", "got {err:?}");
        assert_eq!(err.message, "this host answered with `Devices` where `Snapshots` was expected");

        // And it did not take the arm next to it with it. A poll that timed out is one example of
        // the many payloads `Transport` carries; what they have in common is that no reply this
        // desktop could use ever arrived.
        let offline = host_error(&ClientError::Transport("timed out".into()));
        assert_eq!(offline.code, "host_unreachable", "got {offline:?}");
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
                &cut_host::protocol::DispatchId("d-1".into()),
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

    /// The whole distinction, in the one place it lives. Flip either half and this fails. Asserted
    /// against the entry rather than against "an answer", because since #283 an answer that does not
    /// settle what the host did leaves the entry standing — what reuse turns on is the entry.
    #[test]
    fn an_id_is_reused_only_while_its_job_still_has_an_entry() {
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

        // A settling answer clears the entry, and a deliberate re-cut of the same design is then its
        // own Job rather than a duplicate of the one that finished.
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

    /// Entries clear on a settling answer and expire on `ID_RETENTION`, so this is a backstop
    /// rather than a policy — but without it a session dispatching thousands of distinct designs at
    /// hosts that never answer grows a map nothing ever prunes.
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

    /// An abandoned entry is deliberately not aged out on a timer of this desktop's own choosing.
    /// Time cannot show that the operator loaded fresh material, so expiring one early mints a new
    /// id for a Job the host still remembers under the old one — and cuts the material a second
    /// time. What makes an entry nobody revisits harmless is that the next press is told it was read
    /// as a retry, and that a settling answer clears it, so pressing Cut again really does cut
    /// (#121). The two prunings that do exist are the host's own `ID_RETENTION` and the cap, both on
    /// `in_doubt`; neither is reached here, and neither is a timer of this desktop's invention.
    ///
    /// Named for what is asserted rather than for the scenario: nothing is dispatched, no time
    /// passes, and nothing settles. Two reservations, one entry, one id.
    #[test]
    fn an_entry_nobody_clears_keeps_naming_the_same_id() {
        let dev = test_device_setup();
        let key = key_for(&a_square(10.0));
        let (abandoned, _) = dev.reserve_dispatch_id(&key);

        // The next press joins it, and the entry is still the only one.
        assert_eq!(dev.reserve_dispatch_id(&key).0, abandoned);
        assert_eq!(dev.in_doubt.lock().unwrap().len(), 1);
    }

    /// The boundary the offered retry turns on, asked from both sides of it. An entry this call is
    /// reusing can age past `ID_RETENTION` while the call is still in flight — the reservation is
    /// taken before the request and the sentence is written after the answer, and a dispatch has a
    /// 30-second body budget to spend in between — and the next press would then prune it and mint
    /// a fresh id. Offering to reuse an id that is about to be discarded is the promise #286
    /// removed, arrived at from the other direction, so the sentence asks the same question
    /// `reserve_dispatch_id` answers rather than only whether an entry exists.
    #[test]
    fn a_retry_is_offered_only_while_the_id_is_one_the_next_press_would_reuse() {
        use std::time::Duration;
        let dev = test_device_setup();
        let key = key_for(&a_square(10.0));
        dev.reserve_dispatch_id(&key);
        let written = dev.in_doubt.lock().unwrap().get(&key).map(|(_, w)| *w).expect("just reserved");

        assert!(dev.retry_would_reuse(&key, written), "the id it was just given is not offered");
        assert!(
            dev.retry_would_reuse(&key, written + cut_host::host::ID_RETENTION - Duration::from_secs(1)),
            "an id the host still remembers is not offered"
        );
        assert!(
            !dev.retry_would_reuse(&key, written + cut_host::host::ID_RETENTION),
            "an id the next press would prune was offered as a retry"
        );
        assert!(
            !dev.retry_would_reuse(&key_for(&a_square(20.0)), written),
            "a Job with no entry at all was offered a retry"
        );
    }

    /// A dispatch that never reached a host must not drop an *earlier* dispatch's reservation: that
    /// entry is what a later retry still needs, and clearing it here would send that retry out
    /// under a name the host has never seen. That a call keeps its *own* reservation as well is
    /// asserted separately, by `a_press_that_never_reached_a_host_keeps_its_id_for_the_next_one`.
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

    /// A dispatch answered with a reply that is not `Accepted` learned nothing about whether the
    /// Job began, so it stays in doubt and keeps its id. Asserted here on the desktop's own side of
    /// that: the entry survives, and the next press reserves the same id rather than a fresh one.
    /// What the *host* then does with a repeated id is the host's, and this fixture does not model
    /// it — a peer answering outside the protocol may honour no dedupe at all, which is why the
    /// refusal is `unconfirmed` rather than an all-clear.
    ///
    /// The reply is one no real `Host` sends to a `Dispatch`, so it takes a peer built to break the
    /// protocol; `handle_request` answers every request with a reply that request admits (#283).
    #[test]
    fn a_dispatch_answered_with_the_wrong_reply_stays_in_doubt() {
        let host = cut_host::client::testing::start_host_answering(vec![
            cut_host::protocol::Response::Ok,
            cut_host::protocol::Response::Ok,
        ]);
        let dev = test_device_setup();
        dev.add_host(PairedHost {
            fingerprint: host.fingerprint.clone(),
            ..a_paired_host("host-1", &host.addr)
        });
        let aimed = host_cameo(&HostId("host-1".into()));
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        let key = JobKey {
            host: HostId("host-1".into()),
            device: aimed.instance_id.clone(),
            digest: job_digest(&aimed.machine_id, &a_square(10.0)),
        };
        let err = dev
            .execute_cut(aimed, a_square(10.0))
            .expect_err("`Ok` is not a dispatch this desktop can read as accepted");
        assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
        assert!(
            err.message.contains("answered with `Ok` where `Accepted` was expected"),
            "the reply is not named: {}",
            err.message
        );

        let held = dev.in_doubt.lock().unwrap().get(&key).map(|(id, _)| id.clone());
        assert!(held.is_some(), "a dispatch nothing confirmed must keep the id its retry needs");
        // And the next press reserves that id rather than minting a second name the host has never
        // seen — `reserve_dispatch_id` is the call `execute_cut` makes to choose one.
        assert_eq!(dev.reserve_dispatch_id(&key).0, held.unwrap());

        // The same failure is what the host's own row then shows, because `with_host_within`
        // records it and drops the client alongside it. Pinned here because the row used to say
        // the host could not be reached, directly under the cutters it had just listed.
        let view = dev.host_views().pop().expect("the host is still paired");
        assert_eq!(
            view.unreachable.as_deref(),
            Some("this host answered with `Ok` where `Accepted` was expected")
        );
    }

    /// The same, for the one reply that would otherwise have been believed. `Accepted` is the
    /// variant this dispatch asked for, so nothing about its shape refuses it — only the
    /// `dispatch_id` inside it does. Without that check `Admitted::Started` told the operator
    /// their Job had begun on a host that was answering about a different one, and cleared the
    /// entry a real retry needs in the same breath (#285).
    ///
    /// The fixture cannot know the id the desktop minted — a digest plus a nonce, chosen inside
    /// `execute_cut` — which is what makes any id scripted here a mismatch.
    #[test]
    fn a_dispatch_answered_about_another_job_stays_in_doubt() {
        let host = cut_host::client::testing::start_host_answering(vec![
            cut_host::protocol::Response::Ok,
            cut_host::protocol::Response::Accepted {
                dispatch_id: cut_host::protocol::DispatchId("some-other-job".into()),
                admitted: cut_host::protocol::Admitted::Started,
            },
        ]);
        let dev = test_device_setup();
        dev.add_host(PairedHost {
            fingerprint: host.fingerprint.clone(),
            ..a_paired_host("host-1", &host.addr)
        });
        let aimed = host_cameo(&HostId("host-1".into()));
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        let key = JobKey {
            host: HostId("host-1".into()),
            device: aimed.instance_id.clone(),
            digest: job_digest(&aimed.machine_id, &a_square(10.0)),
        };
        let err = dev
            .execute_cut(aimed, a_square(10.0))
            .expect_err("an `Accepted` about another dispatch is not this Job's start");
        assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
        assert!(
            err.message.contains("answered about a different dispatch than the one it was sent"),
            "the mismatch is not named: {}",
            err.message
        );
        assert!(
            !err.message.contains("some-other-job"),
            "the peer's own id reached the operator: {}",
            err.message
        );

        let held = dev.in_doubt.lock().unwrap().get(&key).map(|(id, _)| id.clone());
        assert!(held.is_some(), "an answer about another Job settles nothing about this one");
        assert_eq!(dev.reserve_dispatch_id(&key).0, held.unwrap());

        let view = dev.host_views().pop().expect("the host is still paired");
        assert_eq!(
            view.unreachable.as_deref(),
            Some("this host answered about a different dispatch than the one it was sent")
        );
    }

    /// A refusal reached the host — that is what a refusal is — and the guard keeping a plain error
    /// plain asked only whether the host had been reached. So every remote refusal was reported as a
    /// dispatch that might be cutting: an off-the-bed cut on a *first* press lost its own code and
    /// told the operator their Job may already be running and to press Cut again (#288). A
    /// mis-scaled document is the ordinary way to arrive here.
    #[test]
    fn a_refused_remote_cut_is_reported_as_the_refusal_it_is() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        // 400 mm square on the test Cameo's 300x200 bed.
        let err = dev.execute_cut(aimed, a_square(400.0)).expect_err("a cut off the bed is refused");
        assert_eq!(err.code, "host_refused", "got {err:?}");
        assert!(
            err.message.contains("outside the 300 x 200 mm cutting area"),
            "the refusal lost its own sentence: {}",
            err.message
        );
        assert!(
            !err.message.contains("may already be cutting"),
            "a refusal invented a cut in flight: {}",
            err.message
        );
        // And the same refusal holds nothing: a host that refused started nothing, so the
        // window-close guard has no dispatch of this to warn about either.
        assert!(dev.remote_dispatched.lock().unwrap().is_empty(), "a refusal held the window open");
    }

    /// What an unconfirmed dispatch is allowed to promise. The sentence used to end "the host
    /// recognizes the same Job and will not cut it twice", which three bounds can make false — the
    /// host forgets an accepted id past its retention and past its capacity cap, and `in_doubt` is
    /// process-local, so a desktop restarted between the two presses has nothing to reuse. That is
    /// the one message in the app telling an operator it is safe to press a button that starts a
    /// blade, and it promised an outcome this side cannot know (#286).
    ///
    /// What is left is what is true: the retry goes out under the same id, a host that still
    /// remembers that id reads it as this Job, and the cutter is the only place to learn which
    /// happened.
    #[test]
    fn an_unconfirmed_dispatch_promises_no_more_than_the_same_id() {
        let host = cut_host::client::testing::start_host_answering(vec![
            cut_host::protocol::Response::Ok,
            cut_host::protocol::Response::Ok,
        ]);
        let dev = test_device_setup();
        dev.add_host(PairedHost {
            fingerprint: host.fingerprint.clone(),
            ..a_paired_host("host-1", &host.addr)
        });
        let aimed = host_cameo(&HostId("host-1".into()));
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        let err = dev
            .execute_cut(aimed, a_square(10.0))
            .expect_err("`Ok` is not a dispatch this desktop can read as accepted");
        assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
        assert!(
            !err.message.contains("will not cut it twice"),
            "the promise survived: {}",
            err.message
        );
        assert!(
            err.message.contains("only the cutter can tell you"),
            "the one way to find out is not offered: {}",
            err.message
        );
        assert!(
            err.message.contains("under the same id"),
            "the retry's one protection is not stated: {}",
            err.message
        );
    }

    /// A refusal on a press made while an earlier dispatch is unsettled settles *this* attempt and
    /// says nothing about that one, so it stays unconfirmed — and clears the entry, which leaves no
    /// id for the next press to send. The advice inverts with it: pressing Cut again is a new Job,
    /// not the retry the other branch describes, and telling the operator otherwise is the promise
    /// this stopped making (#286, #288).
    #[test]
    fn an_unconfirmed_dispatch_with_no_id_left_offers_no_retry() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        // An earlier dispatch of this Job is outstanding when the press below is made.
        let key = JobKey {
            host: id.clone(),
            device: aimed.instance_id.clone(),
            digest: job_digest(&aimed.machine_id, &a_square(400.0)),
        };
        dev.reserve_dispatch_id(&key);

        let err = dev.execute_cut(aimed, a_square(400.0)).expect_err("a cut off the bed is refused");
        assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
        assert!(
            err.message.contains("new Job rather than a retry"),
            "a press with no id left was offered a retry: {}",
            err.message
        );
        assert!(
            !err.message.contains("same id"),
            "an id the refusal cleared was offered anyway: {}",
            err.message
        );
        assert!(!dev.in_doubt.lock().unwrap().contains_key(&key), "the refusal settled this attempt");
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

    /// A refusal settles the press it answered and no other, so the Job's id survives it while a
    /// sibling press is still dispatching under that id — the record that press's retry sends
    /// under, and the only thing that can stop a lost reply from cutting the material twice
    /// (#290). The operator's advice inverts with it: a retry is still the thing to press.
    ///
    /// The sibling is staged rather than raced, because what is being pinned is the rule and not
    /// the timing: `begin_dispatch` is exactly what a press mid-dispatch holds, and a thread parked
    /// inside `with_host` to reproduce it would pin the scheduler instead.
    #[test]
    fn a_refusal_leaves_a_concurrent_press_the_id_its_retry_needs() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        let key = JobKey {
            host: id.clone(),
            device: aimed.instance_id.clone(),
            digest: job_digest(&aimed.machine_id, &a_square(400.0)),
        };
        // A press of the same Job, mid-dispatch and holding the id.
        let (sibling, held, first) = dev.begin_dispatch(&key);
        assert!(first, "the sibling is what reserved the id this test is about");

        let err = dev.execute_cut(aimed, a_square(400.0)).expect_err("a cut off the bed is refused");
        assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
        assert_eq!(
            dev.in_doubt.lock().unwrap().get(&key).map(|(id, _)| id.clone()),
            Some(held),
            "a refusal cleared the id a concurrent press was dispatching under"
        );
        assert!(
            err.message.contains("under the same id"),
            "the retry that is still available was not offered: {}",
            err.message
        );

        // Once that press is over and nothing settled it, the entry is still the retry's — and
        // this is what makes the assertion above about the sibling rather than about nothing.
        drop(sibling);
        assert!(dev.in_doubt.lock().unwrap().contains_key(&key), "an unsettled press keeps its id");
    }

    /// The same rule for the mark that holds the window, and one step wider: the mark is the
    /// cutter's, so a refused press must not clear one left by an accepted press of *any* Job on
    /// that cutter. Clearing it waved the operator past a running remote cut, which is #158
    /// arriving by a second route (#290).
    #[test]
    fn a_refusal_leaves_a_concurrent_press_holding_the_window() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        // An accepted press of some Job on this cutter, not yet seen to end.
        dev.mark_dispatched(&id, &aimed.instance_id, AttemptId::next());

        let err = dev.execute_cut(aimed, a_square(400.0)).expect_err("a cut off the bed is refused");
        // The press's own outcome is settled and nothing of *its* Job is outstanding, so it is
        // the plain refusal (#288) — the mark it must not clear belongs to a different Job.
        assert_eq!(err.code, "host_refused", "got {err:?}");
        assert!(
            dev.a_cut_may_be_running(),
            "a refused press cleared the mark an accepted press was still held by"
        );
    }

    /// A refused press with nothing else outstanding does stop holding the window, which is what
    /// makes the test above about attribution rather than about a mark that never clears.
    #[test]
    fn a_refusal_with_nothing_else_outstanding_releases_the_window() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        dev.execute_cut(aimed, a_square(400.0)).expect_err("a cut off the bed is refused");
        assert!(!dev.a_cut_may_be_running(), "a refusal is the host saying it started nothing");
    }

    /// The guard answers for the cut, not for the aim: dispatch, then use either verb the UI
    /// offers for looking elsewhere, and the Job is still running on the Pi. Both used to close
    /// the window in silence — the second one especially, since with no aim at all the guard
    /// returned `false` however many dispatches were outstanding (#158).
    #[test]
    fn a_dispatch_holds_the_window_after_the_aim_moves_away() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&id);
        dev.connect(aimed.clone()).expect("aiming at a host is bookkeeping only");

        dev.execute_cut(aimed.clone(), a_square(10.0)).expect("dispatch");

        dev.connect(test_instance()).expect("aiming back at the local cutter");
        assert!(dev.a_cut_may_be_running(), "the remote cut stopped counting when the aim moved");

        dev.connect(aimed).expect("aiming at the host again");
        dev.disconnect().expect("disconnecting a remote aim is bookkeeping only");
        assert!(dev.connected.lock().unwrap().is_none(), "the aim must really be gone");
        assert!(dev.a_cut_may_be_running(), "with no aim the guard stopped answering at all");
    }

    /// Quitting stops what only this process can stop. A Job parked on the local cutter is
    /// cancelled here rather than left, because nothing would be able to stop or resume it once
    /// this process — which owns that transport — is gone.
    #[test]
    fn quitting_cancels_local_motion_and_leaves_a_host_cutting() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        dev.cut_from_request(&app, request_from(plan)).expect("cut");
        assert!(dev.status().is_active(), "the local cut must be parked mid-flight to be cancellable");

        dev.stop_local_motion();
        let stopped = wait_for_cancelled(&dev);
        assert!(!stopped.actions.cut, "quitting left the local cutter taking Jobs");
    }

    /// The other half, and the load-bearing one: a Cut Host owns the Job it accepted, so quitting
    /// leaves it cutting. That is the rule `disconnect` already followed, and the reason
    /// `force_quit` no longer routes a cancel by whatever happens to be aimed (#158).
    #[test]
    fn quitting_leaves_a_cut_host_job_running() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());
        dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");

        let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
        wait_until(|| cameo_is_active(&direct), "the cut never started");

        dev.stop_local_motion();
        assert!(cameo_is_active(&direct), "quitting reached into a Job the Cut Host owns");
    }

    /// Forgetting a host retracts the marks its answer covered, because the poll that would
    /// otherwise clear one is a call to the host that has just gone: left behind, a mark raises the
    /// quit prompt on every close for the rest of the session with no verb that could ever satisfy
    /// it (#158). The status cache goes with it, because host ids are recycled.
    #[test]
    fn forgetting_a_host_releases_the_window_its_dispatch_held() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());
        // Polled before the cut, so the cache holds a reading for the forget to clear. Nothing
        // polls after it, which is what leaves the mark standing over a Job that has since ended —
        // the state an operator who closed the dialog is in.
        dev.status();
        dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");

        // The Job has to be over before the host can be forgotten at all — `forget` refuses while
        // it says it is cutting.
        let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
        wait_until(|| cameo_is_active(&direct), "the cut never started");
        direct.confirm_pass_done(cut_host::host::testing::CAMEO).unwrap();
        wait_until(|| !cameo_is_active(&direct), "the cut never finished");
        assert!(dev.a_cut_may_be_running(), "the mark is what this test is about");

        dev.forget(&id, &hosts_path, false).expect("an idle host can be forgotten");
        assert!(!dev.a_cut_may_be_running(), "a forgotten host's mark can never be cleared again");
        assert!(
            dev.last_remote_status.lock().unwrap().is_empty(),
            "a recycled host id would inherit this reading"
        );
    }

    /// A press that has returned is a press that is no longer dispatching. Nothing in the desktop
    /// reads that directly, which is why the test does: a registration left behind makes every
    /// later answer defer to a press that has gone, so the Job stays in doubt and every press until
    /// `ID_RETENTION` is read as a retry rather than cut.
    #[test]
    fn a_finished_press_stops_counting_as_dispatching() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&id);
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        dev.execute_cut(aimed.clone(), a_square(10.0)).expect("dispatch");
        assert!(dev.dispatching.lock().unwrap().is_empty(), "a finished press stayed registered");

        // And the settling answer it got really did clear the Job, which is what a leaked
        // registration would have prevented.
        let key = JobKey {
            host: id,
            device: aimed.instance_id,
            digest: job_digest(&aimed.machine_id, &a_square(10.0)),
        };
        assert!(!dev.in_doubt.lock().unwrap().contains_key(&key), "an accepted dispatch settles the Job");
    }

    /// Two presses of one Job that are both answered before either finishes must not each defer to
    /// the other: the Job would stay in doubt with nothing outstanding, and every later press would
    /// be read as a retry and answered "already accepted" while the cutter never moved.
    #[test]
    fn two_presses_that_both_settle_leave_nothing_in_doubt() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        let key = JobKey {
            host: HostId("host-1".into()),
            device: cut_host::host::testing::CAMEO.into(),
            digest: job_digest("cameo5", &a_square(10.0)),
        };

        let (first, id, _) = dev.begin_dispatch(&key);
        let (second, joined, _) = dev.begin_dispatch(&key);
        assert_eq!(id, joined, "the second press joins the id already reserved");

        dev.settle_dispatch(&key, &id, first.id);
        assert!(
            dev.in_doubt.lock().unwrap().contains_key(&key),
            "the first answer said nothing about the press still dispatching"
        );

        dev.settle_dispatch(&key, &joined, second.id);
        assert!(
            !dev.in_doubt.lock().unwrap().contains_key(&key),
            "with every press answered the Job is no longer in doubt"
        );
    }

    /// An answer about an id the entry no longer holds settles that id, not the entry: the entry
    /// can be pruned and re-minted while a dispatch is in flight, and whoever minted the
    /// replacement may still need it.
    #[test]
    fn an_answer_about_a_replaced_id_leaves_the_replacement() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        let key = JobKey {
            host: HostId("host-1".into()),
            device: cut_host::host::testing::CAMEO.into(),
            digest: job_digest("cameo5", &a_square(10.0)),
        };

        let (press, stale, _) = dev.begin_dispatch(&key);
        // What pruning plus a later press leaves behind: the same Job, a different id.
        let minted = cut_host::protocol::DispatchId("minted-by-someone-else".into());
        dev.in_doubt.lock().unwrap().insert(key.clone(), (minted.clone(), std::time::Instant::now()));

        dev.settle_dispatch(&key, &stale, press.id);
        assert_eq!(
            dev.in_doubt.lock().unwrap().get(&key).map(|(id, _)| id.clone()),
            Some(minted),
            "an answer about the id that was replaced deleted its replacement"
        );
    }

    /// A local cut holds the window too, through the arm of the guard that involves no mark at all.
    #[test]
    fn a_local_cut_holds_the_window() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        assert!(!dev.a_cut_may_be_running(), "nothing has been cut yet");

        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        dev.cut_from_request(&app, request_from(plan)).expect("cut");
        assert!(dev.a_cut_may_be_running(), "the guard waved through a local cut in progress");
    }

    /// A press that never reached a host retracts its own mark and no other — the same rule the
    /// refusal branch follows, in separate code, so it is a separate test.
    #[test]
    fn a_press_that_never_reached_a_host_leaves_another_press_holding_the_window() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        let aimed = host_cameo(&HostId("host-1".into()));
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        // An accepted press on this cutter, not yet seen to end.
        dev.mark_dispatched(&HostId("host-1".into()), &aimed.instance_id, AttemptId::next());

        dev.execute_cut(aimed, a_square(10.0)).expect_err("nothing answers this host");
        assert!(
            dev.a_cut_may_be_running(),
            "a press that never left the machine cleared a running cut's mark"
        );
    }

    /// The mark is written before the request precisely so a dispatch whose reply is lost still
    /// holds the window: by the time the reply fails to arrive there is nothing left to write it
    /// from, and the Job may be cutting on the Pi.
    #[test]
    fn a_dispatch_whose_reply_was_lost_holds_the_window() {
        let host = cut_host::client::testing::start_host_answering(vec![
            cut_host::protocol::Response::Ok,
            cut_host::protocol::Response::Ok,
        ]);
        let dev = test_device_setup();
        dev.add_host(PairedHost {
            fingerprint: host.fingerprint.clone(),
            ..a_paired_host("host-1", &host.addr)
        });
        let aimed = host_cameo(&HostId("host-1".into()));
        *dev.connected.lock().unwrap() = Some(aimed.clone());

        let err = dev.execute_cut(aimed, a_square(10.0)).expect_err("`Ok` is not an accepted dispatch");
        assert_eq!(err.code, "dispatch_unconfirmed", "got {err:?}");
        assert!(
            dev.a_cut_may_be_running(),
            "a dispatch nothing confirmed left the window free to close"
        );
    }

    /// Polling one cutter says nothing about another: the clear is keyed to the cutter that
    /// answered, or a dialog open on an idle cutter would wipe the mark of the Pi cutting next to
    /// it — #158 by way of its own fix.
    #[test]
    fn a_poll_of_one_cutter_leaves_another_holding_the_window() {
        let idle = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let watched = dev
            .pair("Idle Pi".into(), idle.addr.clone(), HOST_TOKEN.into(), idle.fingerprint.clone(), &hosts_path)
            .expect("pairing with the idle host");
        let elsewhere = HostId("host-elsewhere".into());
        dev.add_host(a_paired_host("host-elsewhere", "127.0.0.1:1"));

        // A press outstanding on the *other* host's cutter.
        dev.mark_dispatched(&elsewhere, cut_host::host::testing::CAMEO, AttemptId::next());

        *dev.connected.lock().unwrap() = Some(host_cameo(&watched));
        assert!(dev.status().actions.cut, "the aimed cutter is free, which is what makes it clear marks");
        assert!(dev.a_cut_may_be_running(), "polling one cutter cleared a mark belonging to another");
        // And the same for the wide clear `list_devices` performs.
        dev.list_devices();
        assert!(dev.a_cut_may_be_running(), "the device-list poll cleared a mark for an unreachable host");
    }

    /// The clear has to be as wide as the guard: a Job dispatched to a Pi the operator then aimed
    /// away from is released by the device-list poll the cut dialog already runs, and by nothing
    /// else. Without it the mark stood for the rest of the session and every close raised a prompt
    /// no verb could satisfy (#158).
    #[test]
    fn a_poll_of_every_cutter_releases_a_mark_the_aim_left_behind() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("pairing with the loopback host");
        let aimed = host_cameo(&id);
        dev.connect(aimed.clone()).expect("aiming at a host is bookkeeping only");
        dev.execute_cut(aimed, a_square(10.0)).expect("dispatch");

        // The operator goes back to their own cutter while the Pi finishes.
        dev.connect(test_instance()).expect("aiming back at the local cutter");
        assert!(dev.a_cut_may_be_running(), "the dispatch stopped counting when the aim moved");

        let direct = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
        wait_until(|| cameo_is_active(&direct), "the cut never started");
        direct.confirm_pass_done(cut_host::host::testing::CAMEO).unwrap();
        wait_until(|| !cameo_is_active(&direct), "the cut never finished");

        wait_until(
            || {
                dev.list_devices();
                !dev.a_cut_may_be_running()
            },
            "a finished cut on an un-aimed cutter still held the window",
        );
    }

    /// Forgetting one host says nothing about another's cutters.
    #[test]
    fn forgetting_one_host_leaves_another_holding_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();
        let cutting = HostId("host-cutting".into());
        dev.add_host(a_paired_host("host-cutting", "127.0.0.1:1"));
        dev.add_host(a_paired_host("host-elsewhere", "127.0.0.1:1"));
        dev.mark_dispatched(&cutting, cut_host::host::testing::CAMEO, AttemptId::next());

        dev.forget(&HostId("host-elsewhere".into()), &hosts_path, true).expect("forced past unreachable");
        assert!(
            dev.a_cut_may_be_running(),
            "forgetting an unrelated host discarded the warning about the one that is cutting"
        );
    }

    /// A mark written while the forget's own question was in flight is not one that answer speaks
    /// for, so it survives: the operator keeps the warning about a Job whose cancel route this
    /// desktop has just discarded, which is the last thing it can still offer.
    #[test]
    fn forgetting_a_host_keeps_a_mark_its_answer_did_not_cover() {
        let dev = test_device_setup();
        let id = HostId("host-1".into());
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

        dev.mark_dispatched(&id, cut_host::host::testing::CAMEO, AttemptId::next());
        let answered_for = dev.marks_for(&id);
        // The press that lands between the host's answer and the retraction.
        dev.mark_dispatched(&id, cut_host::host::testing::CAMEO, AttemptId::next());

        dev.retract_marks(&answered_for);
        assert!(
            dev.a_cut_may_be_running(),
            "a dispatch the host's answer never covered lost its warning anyway"
        );
    }

    // --- presets: identity is (machine_id, id), so one cutter's entry cannot destroy another's ---

    fn a_user_preset(machine: &str, id: &str, force: u32) -> MaterialPreset {
        MaterialPreset {
            id: id.into(),
            name: format!("{machine} {id}"),
            machine_id: machine.into(),
            settings: cutplan::presets::PresetSettings {
                speed: Some(5), force: Some(force), repeat_count: 1,
            },
            builtin: false,
        }
    }

    /// The whole of #153, against a temporary presets file: an operator's id is their own string,
    /// so `my-vinyl` names one material on a Cameo and another on a Puma. Keyed on the id alone,
    /// saving one overwrote the other and deleting one removed both — with the preset editor (#55)
    /// that is an operator watching their settings vanish.
    #[test]
    fn a_presets_id_belongs_to_one_machine_when_saved_deleted_or_shadowed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");

        save_preset(&path, a_user_preset("cameo5", "my-vinyl", 11)).expect("saved");
        save_preset(&path, a_user_preset("puma", "my-vinyl", 22)).expect("the other machine's");

        let force_of = |machine: &str| {
            list_presets(&path, machine).unwrap().into_iter()
                .find(|p| p.id == "my-vinyl").map(|p| p.settings.force)
        };
        assert_eq!(force_of("cameo5"), Some(Some(11)), "saving the Puma's entry overwrote it");
        assert_eq!(force_of("puma"), Some(Some(22)));

        // Editing one is a save over the same pair, and touches only that machine's entry.
        save_preset(&path, a_user_preset("puma", "my-vinyl", 33)).expect("edited");
        assert_eq!(force_of("cameo5"), Some(Some(11)), "an edit on one machine changed the other");
        assert_eq!(force_of("puma"), Some(Some(33)));

        // A delete names the machine, and cannot reach another's entry under the same id.
        delete_preset(&path, "puma", "my-vinyl").expect("deleted");
        assert_eq!(force_of("puma"), None);
        assert_eq!(force_of("cameo5"), Some(Some(11)),
            "deleting the Puma's preset removed the Cameo's");

        // Shadowing keys on the pair too: an entry under one machine named with another's builtin
        // id leaves that builtin listed.
        let shadow = cutplan::presets::builtin_presets().into_iter()
            .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins").id;
        save_preset(&path, a_user_preset("puma", &shadow, 24)).expect("saved under the Puma");
        let cameo = list_presets(&path, "cameo5").unwrap();
        let listed: Vec<_> = cameo.iter().filter(|p| p.id == shadow).collect();
        assert_eq!(listed.len(), 1, "one entry for {shadow} on the Cameo, got {listed:#?}");
        assert!(listed[0].builtin, "the Cameo's builtin was shadowed by a Puma entry");
    }

    /// A `presets.json` written before #153 loads with the same entries afterwards — every entry
    /// already carried its `machine_id`, so only the uniqueness key changed — and deleting a user
    /// shadow still reveals that machine's builtin.
    #[test]
    fn a_presets_file_written_before_the_pair_key_loads_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        let builtin = cutplan::presets::builtin_presets().into_iter()
            .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins");
        std::fs::write(&path, format!(r#"{{"version":1,"presets":[
            {{"id":"{}","name":"Mine","machine_id":"cameo5",
             "settings":{{"speed":1,"force":99,"repeat_count":1}},"builtin":false}},
            {{"id":"my-vinyl","name":"Vinyl","machine_id":"puma",
             "settings":{{"speed":2,"force":2,"repeat_count":1}},"builtin":false}}
        ]}}"#, builtin.id)).unwrap();

        let cameo = list_presets(&path, "cameo5").unwrap();
        let shadowing = cameo.iter().find(|p| p.id == builtin.id).expect("the shadow loads");
        assert_eq!(shadowing.name, "Mine", "the user entry still shadows its own machine's builtin");
        assert!(list_presets(&path, "puma").unwrap().iter().any(|p| p.id == "my-vinyl"),
            "the other machine's entry loads unchanged");

        delete_preset(&path, "cameo5", &builtin.id).expect("deleted");
        let revealed = list_presets(&path, "cameo5").unwrap();
        let after = revealed.iter().find(|p| p.id == builtin.id).expect("the builtin is back");
        assert!(after.builtin, "deleting a user shadow must reveal the builtin it hid");
        assert_eq!(after.name, builtin.name);
    }

    /// A user entry under a builtin's own pair hides it, and the app offers no way back to the
    /// settings it shipped with — so the save is refused rather than the material lost.
    #[test]
    fn saving_over_a_builtins_pair_is_refused_and_leaves_it_shipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        let builtin = cutplan::presets::builtin_presets().into_iter()
            .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins");

        let err = save_preset(&path, a_user_preset("cameo5", &builtin.id, 12)).unwrap_err();
        assert_eq!(err.code, "builtin_preset",
            "a save over a builtin's pair was allowed through: {}", err.message);
        assert!(err.message.contains(&builtin.id),
            "the refusal names no preset: {}", err.message);

        assert!(user_entries(&path).unwrap().is_empty(),
            "a refused save still wrote an entry to the file");
        let listed = list_presets(&path, "cameo5").unwrap();
        let after = listed.iter().find(|p| p.id == builtin.id).expect("the builtin is still listed");
        assert!(after.builtin, "the refused save shadowed the builtin anyway");
        assert_eq!(after.settings, builtin.settings, "the shipped settings were overwritten");
    }

    /// Each of these is a preset the operator cannot use afterwards: an id-less entry is dropped
    /// on the next load, a blank name is a picker row naming no material, and settings past the
    /// machine's edge are refused at the cut. All three are refused where they are typed.
    #[test]
    fn a_preset_the_operator_could_not_use_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");

        let no_id = MaterialPreset { id: String::new(), ..a_user_preset("cameo5", "mine", 12) };
        let no_machine = MaterialPreset { machine_id: String::new(), ..a_user_preset("cameo5", "mine", 12) };
        let blank_name = MaterialPreset { name: "   ".into(), ..a_user_preset("cameo5", "mine", 12) };
        let past_the_edge = a_user_preset("cameo5", "mine", 99);

        for (preset, what) in [(no_id, "an empty id"), (no_machine, "no machine"),
                               (blank_name, "a blank name"), (past_the_edge, "a force out of range")] {
            let err = save_preset(&path, preset).unwrap_err();
            assert_eq!(err.code, "invalid_preset", "{what} was saved: {}", err.message);
        }
        assert!(user_entries(&path).unwrap().is_empty(),
            "a refused save still wrote an entry to the file");
    }

    /// An entry with two faults is named by the one the operator can act on. A pair that names a
    /// builtin is unsavable whatever it holds, so reporting the force first sends them to fix a
    /// number that was never what refused the save.
    #[test]
    fn what_a_preset_is_refuses_it_before_what_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        let builtin = cutplan::presets::builtin_presets().into_iter()
            .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins");

        let both_wrong = a_user_preset("cameo5", &builtin.id, 99);
        let err = save_preset(&path, both_wrong).unwrap_err();
        assert_eq!(err.code, "builtin_preset",
            "a shipped pair was reported as a settings fault: {}", err.message);
    }

    /// A delete that removed nothing used to report success, leaving the entry listed — which
    /// reads as the app ignoring the operator. Which nothing it was is worth saying: a builtin is
    /// not theirs to delete, an unsaved id never existed.
    #[test]
    fn a_delete_that_removed_nothing_says_which_nothing_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        save_preset(&path, a_user_preset("cameo5", "my-vinyl", 12)).expect("saved");

        let unknown = delete_preset(&path, "cameo5", "never-saved").unwrap_err();
        assert_eq!(unknown.code, "unknown_preset",
            "deleting an id nobody saved reported success: {}", unknown.message);

        let builtin = cutplan::presets::builtin_presets().into_iter()
            .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins");
        let shipped = delete_preset(&path, "cameo5", &builtin.id).unwrap_err();
        assert_eq!(shipped.code, "builtin_preset",
            "deleting an unshadowed builtin reported success: {}", shipped.message);

        assert!(list_presets(&path, "cameo5").unwrap().iter().any(|p| p.id == "my-vinyl"),
            "a refused delete removed the operator's own entry");
    }

    /// The refusals above must not reach the ordinary case: the operator's own preset is still
    /// saved, edited over its pair, and deleted.
    #[test]
    fn a_preset_of_the_operators_own_still_saves_edits_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");

        save_preset(&path, a_user_preset("cameo5", "my-vinyl", 12)).expect("saved");
        save_preset(&path, a_user_preset("cameo5", "my-vinyl", 21)).expect("edited");
        let edited = list_presets(&path, "cameo5").unwrap().into_iter()
            .find(|p| p.id == "my-vinyl").expect("the edit is listed");
        assert_eq!(edited.settings.force, Some(21), "the edit did not reach the file");

        delete_preset(&path, "cameo5", "my-vinyl").expect("deleted");
        assert!(!list_presets(&path, "cameo5").unwrap().iter().any(|p| p.id == "my-vinyl"),
            "the entry survived its delete");
    }

    /// The conversion, end to end below the IPC call. Every one of the five sites used to send
    /// the code `preset_error` with a `Debug` rendering of the value, so a file this build is
    /// too old to read and a damaged one arrived under the same code, told apart only by the
    /// discriminant inside the message (#278).
    #[test]
    fn a_presets_file_this_build_cannot_read_is_refused_in_words_with_its_own_code() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        std::fs::write(&path, r#"{"presets":[]}"#).expect("wrote the fixture");

        // `list_presets` is the read side; `delete_preset` reaches the same read through
        // `user_entries`, so both must speak cutplan's words rather than each writing their own.
        for err in [list_presets(&path, "cameo5").unwrap_err(),
                    delete_preset(&path, "cameo5", "my-vinyl").unwrap_err()] {
            assert_eq!(err.code, "presets_corrupt", "{}", err.message);
            assert_eq!(err.message, "the presets file does not state a usable whole-number \
                                     version, so this build cannot tell what format it is in");
        }

        std::fs::write(&path, r#"{"version":3,"presets":[]}"#).expect("wrote the fixture");
        let newer = list_presets(&path, "cameo5").unwrap_err();
        assert_eq!(newer.code, "presets_unknown_version",
            "a version this build does not read shares a code with a damaged file: {}", newer.message);
    }

    /// A refusal that does not depend on writing the file must not lose the race to one that
    /// does: an entry the editor should never have sent is named as that, not as a disk fault,
    /// however the disk is behaving. This is the ordering the e2e fake mirrors.
    #[test]
    fn what_a_preset_is_refuses_it_before_the_file_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        // A directory at the presets path, not a file where its parent belongs: reading a
        // directory fails on every platform, where a non-directory *parent* is `ENOTDIR` on Unix
        // and `ERROR_PATH_NOT_FOUND` on Windows — which Rust maps to `NotFound`, so `load_presets`
        // would call it a first run there and this test would read a disk fault it never got
        // (Codex on PR #280).
        let path = dir.path().join("presets.json");
        std::fs::create_dir(&path).expect("put a directory where the file goes");

        // Every refusal `save_preset` makes without touching the file, not just one: with only
        // the blank name asserted, moving any of the other four below the read left this green
        // while those saves reported a disk fault (Codex on PR #280).
        let mine = || a_user_preset("cameo5", "mine", 12);
        let builtin = cutplan::presets::builtin_presets().into_iter()
            .find(|p| p.machine_id == "cameo5").expect("premise: the Cameo ships builtins");
        let unsavable = [
            ("a blank name", MaterialPreset { name: "  ".into(), ..mine() }, "invalid_preset"),
            ("an empty id", MaterialPreset { id: String::new(), ..mine() }, "invalid_preset"),
            ("no machine", MaterialPreset { machine_id: String::new(), ..mine() }, "invalid_preset"),
            ("a force out of range", a_user_preset("cameo5", "mine", 99), "invalid_preset"),
            ("a builtin's pair", a_user_preset("cameo5", &builtin.id, 12), "builtin_preset"),
        ];
        for (what, preset, code) in unsavable {
            let err = save_preset(&path, preset).unwrap_err();
            assert_eq!(err.code, code,
                "{what} was reported as a disk fault: {}", err.message);
        }

        // And the file really is unusable, so the assertion above is not vacuous: a valid entry
        // is refused by it. `presets_unreadable`, not `presets_unwritable`, because `save_preset`
        // re-reads the user entries first and a save that cannot see what it would replace must
        // not write over it.
        let valid = a_user_preset("cameo5", "mine", 12);
        assert_eq!(save_preset(&path, valid).unwrap_err().code, "presets_unreadable");
    }
}
