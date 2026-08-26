// SPDX-License-Identifier: GPL-3.0-or-later

//! A Cut Host: one `DeviceManager` per attached cutter, and the fan-out that lets
//! several clients watch them all over one connection each.
//!
//! Concurrency needs almost nothing here. `DeviceManager::spawn` already gives each
//! cutter its own worker thread, its own cancel flag and its own published
//! `CutStatus`, so a failure on one cutter cannot reach another — that isolation is
//! structural, not implemented.

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use driver_core::manager::{CutPass, DeviceError, DeviceManager};
use driver_core::{DeviceBackendFactory, DeviceInfo, Phase};

use crate::check::{check_passes, PassFault};
use crate::protocol::{Admitted, DeviceSnapshot, DispatchId, Event, Refusal};

/// How long a Cut Host remembers a dispatch id it has accepted.
///
/// Long enough to cover any retry an operator would recognise as one — a dropped reply, a Wi-Fi
/// gap, a laptop lid — and short enough that the daemon can say how long it is. Remembering
/// forever made a Job silently a no-op because the daemon had seen its id in January, which no
/// client could know or work around (#119). Forgetting too soon is the opposite failure: a retry
/// that arrives past this cuts the material a second time.
///
/// Public because it is the *client's* business too: past this window a retry cannot be
/// recognised, so a client holding an unanswered dispatch is holding something that has stopped
/// meaning anything. Reading it from here is what keeps the two sides' idea of the window from
/// drifting apart — the alternative is a number written down twice.
pub const ID_RETENTION: Duration = Duration::from_secs(60 * 60);

/// The longest dispatch id this host will accept.
///
/// A client's own name for one dispatch; the desktop's is about sixty characters. The cap is not
/// about that client — it is that an id is *remembered*, so without a bound an authenticated
/// client could hand a Pi with a gigabyte of RAM a megabyte of id per dispatch and have the daemon
/// hold every one of them for an hour.
const MAX_DISPATCH_ID: usize = 128;

/// How many accepted ids one cutter remembers at once, oldest evicted first.
///
/// The hour alone is not a bound: nothing prunes until the next dispatch arrives, so a daemon left
/// idle keeps whatever the last burst put there. This is what makes the memory a fixed cost rather
/// than a function of how fast a client can dispatch. It only weakens the dedupe after this many
/// *distinct* Jobs on one cutter inside an hour, by which point the oldest is long finished.
const MAX_REMEMBERED_IDS: usize = 512;

pub(crate) struct DeviceSlot {
    pub info: DeviceInfo,
    pub manager: Arc<DeviceManager>,
    /// The most recent Job this cutter was given, so a reattaching client can tell
    /// whose finished cut it is looking at. `CutStatus` cannot say.
    ///
    /// `None` between `dispatch` returning and `cut()` assigning it — a
    /// snapshot-only client racing that window sees an active cutter with no id.
    pub job_id: Mutex<Option<u64>>,
    pub admission: Mutex<Admission>,
}

impl DeviceSlot {
    /// Whether this cutter is spoken for: a Job in flight, *or* a dispatch admitted and not yet
    /// inside `manager.cut`.
    ///
    /// `actions` cannot answer the second half — that is the whole reason `Admission::starting`
    /// exists — so anything deciding whether a transport may be dropped has to ask both. One
    /// predicate rather than two readings, because the two readers (`reconnect` and the shutdown
    /// guard) are exactly where getting it wrong pulls a transport out from under a moving blade.
    pub(crate) fn is_claimed(&self) -> bool {
        self.admission.lock().unwrap().starting || self.manager.status().is_active()
    }
}

/// What a dispatch has to claim, in one place so it can be claimed at once.
///
/// The two halves are one transaction — claim the id and claim the cutter, or
/// neither — and splitting them is what let a caller be told a Job was accepted
/// that never existed. They live together so no lock ordering can get that wrong.
#[derive(Default)]
pub(crate) struct Admission {
    /// Dispatch ids already accepted for this cutter, and when each was last seen. A repeat
    /// starts nothing, until it ages past `ID_RETENTION`.
    accepted: HashMap<DispatchId, Instant>,
    /// Set from admitting a dispatch until its worker's `manager.cut` returns.
    /// `actions.cut` cannot cover that window on its own: the cut runs on a thread
    /// of its own, so until it has told the `DeviceManager` anything, the cutter
    /// still publishes itself as free and a second dispatch would be admitted too.
    ///
    /// Cleared by `StartingClaim`'s `Drop`, never by hand: a path that leaves it set makes the
    /// cutter permanently unclaimable, and the ways to leave it set are exactly the ones an
    /// explicit assignment cannot reach — a panic inside `manager.cut`, or a worker thread that
    /// could not be spawned at all (#120).
    starting: bool,
}

impl Admission {
    /// Drop every accepted id older than `ID_RETENTION` as of `now`.
    ///
    /// Takes the moment rather than reading the clock so a test can look at this from a point in
    /// the future — `Instant` is monotonic from an unspecified epoch, so stepping *backwards* from
    /// `now` is what a freshly booted machine cannot do.
    fn forget_expired(&mut self, now: Instant) {
        self.accepted.retain(|_, seen| now.saturating_duration_since(*seen) < ID_RETENTION);
    }

    /// Drop the oldest ids until at most `MAX_REMEMBERED_IDS` remain.
    ///
    /// The age rule cannot do this on its own: it only runs when a dispatch arrives, so a burst
    /// followed by silence leaves every id of that burst held. Evicting by age keeps the ids a
    /// retry could plausibly still name and drops the ones a retry never will.
    fn forget_oldest_beyond_cap(&mut self) {
        while self.accepted.len() > MAX_REMEMBERED_IDS {
            let Some(oldest) =
                self.accepted.iter().min_by_key(|(_, seen)| **seen).map(|(id, _)| id.clone())
            else {
                return;
            };
            self.accepted.remove(&oldest);
        }
    }
}

/// Holds `Admission::starting` for as long as a dispatch is on its way to `manager.cut`, and
/// clears it however that ends — normally, by a panic unwinding through the worker, or by the
/// worker never running because the thread could not be spawned (the closure, and this with it,
/// is dropped there).
///
/// The flag is half of `is_claimed`, so leaking it set claims the cutter for a Job that will never
/// run: `reconnect` refuses, every later dispatch refuses, and the shutdown guard holds — leaving
/// a restart of `cuthulhu-cutd` as the only way back, which is the outcome `Reconnect` exists to
/// avoid.
struct StartingClaim {
    host: Arc<Host>,
    device: String,
}

impl Drop for StartingClaim {
    fn drop(&mut self) {
        let Some(slot) = self.host.slot(&self.device) else { return };
        // Poison is recovered from rather than propagated: this runs during unwind if the worker
        // panicked, and a panic in a `Drop` while unwinding aborts the process — so the one path
        // that most needs the flag cleared would instead take the daemon down.
        let mut admission = slot.admission.lock().unwrap_or_else(|e| e.into_inner());
        admission.starting = false;
    }
}

/// One cutter a Cut Host is holding, for whoever has to say why it will not exit yet.
pub struct Claim {
    pub device: String,
    pub phase: Phase,
    pub job_id: Option<u64>,
    /// A dispatch admitted and not yet inside `manager.cut`: claimed, with nothing for the phase
    /// or the Job id to describe yet.
    pub starting: bool,
}

pub struct Host {
    /// Insertion order is the enumeration order, kept separately because a
    /// `HashMap` has none and clients render a list.
    order: Vec<String>,
    slots: HashMap<String, DeviceSlot>,
    subscribers: Mutex<Vec<mpsc::SyncSender<Event>>>,
    pub(crate) factory: Arc<dyn DeviceBackendFactory>,
}

impl Host {
    /// Enumerate every attached cutter, connect it, and start pumping its events.
    ///
    /// Connecting here rather than on a client's request is what stops two clients
    /// racing over one cutter's connection state, and what stops a client that dies
    /// mid-Job from orphaning a Transport. It also means `DeviceManager::connect`'s
    /// identity probe has already run against real hardware before any client can
    /// aim at the cutter.
    ///
    /// A cutter that fails to connect is kept, not dropped: its snapshot reports the
    /// failure, which is more use to whoever is standing next to it than a device
    /// that silently does not exist.
    pub fn start(factory: Arc<dyn DeviceBackendFactory>) -> Arc<Host> {
        let infos = factory.list_devices();
        let mut order = Vec::with_capacity(infos.len());
        let mut slots = HashMap::with_capacity(infos.len());
        let mut pumps = Vec::with_capacity(infos.len());

        for info in infos {
            let (manager, events) = DeviceManager::spawn(factory.clone());
            if let Err(e) = manager.connect(info.clone()) {
                eprintln!("cut host: {} did not connect: {e}", info.instance_id);
            }
            order.push(info.instance_id.clone());
            pumps.push((info.instance_id.clone(), events));
            slots.insert(
                info.instance_id.clone(),
                DeviceSlot {
                    info,
                    manager: Arc::new(manager),
                    job_id: Mutex::new(None),
                    admission: Mutex::new(Admission::default()),
                },
            );
        }

        let host = Arc::new(Host { order, slots, subscribers: Mutex::new(Vec::new()), factory });

        for (device, events) in pumps {
            // Weak, not Arc: a strong capture here would hold `host` alive forever, since
            // this thread only exits when `events` ends, and `events` only ends when the
            // `DeviceManager` this pump reads from — reachable solely through `host.slots`
            // — drops. An Arc'd pump waiting on the object that waits on the pump is a
            // cycle neither side can break. Upgrading per event instead means the last
            // external `Arc<Host>` drop lets `Host` drop, which drops `slots`, which drops
            // each `DeviceManager`, which drops its `cmd_tx`; the worker's `cmd_rx.recv()`
            // then errors, the worker returns and drops its event sender, and `events`
            // ends this loop — normally before the next `upgrade()` even has to fail.
            let host = Arc::downgrade(&host);
            thread::spawn(move || {
                for event in events {
                    let Some(host) = host.upgrade() else { break };
                    host.broadcast(Event { device: device.clone(), event });
                }
            });
        }
        host
    }

    pub fn devices(&self) -> Vec<DeviceInfo> {
        self.order.iter().filter_map(|id| self.slots.get(id)).map(|s| s.info.clone()).collect()
    }

    /// Everything a reattaching client needs, for every cutter, in one value.
    ///
    /// `claimed` is one reading: `Admission` is held across the status read, so a dispatch that has
    /// claimed the cutter cannot be reported free before its worker publishes. The guard is dropped
    /// before `job_id` is read, because `claims` needs the same two mutexes and holding both here
    /// is what let the two orders cross — a status poll racing the shutdown guard's report
    /// deadlocked the daemon, and a wedged watch thread can never honour SIGTERM.
    /// **No slot lock is held while another is taken.**
    pub fn snapshots(&self) -> Vec<DeviceSnapshot> {
        self.order
            .iter()
            .filter_map(|id| self.slots.get(id))
            .map(|s| {
                let (claimed, status) = {
                    let admission = s.admission.lock().unwrap();
                    let status = s.manager.status();
                    (admission.starting || status.is_active(), status)
                };
                DeviceSnapshot {
                    info: s.info.clone(),
                    status,
                    job_id: *s.job_id.lock().unwrap(),
                    claimed,
                }
            })
            .collect()
    }

    pub fn subscribe(&self) -> mpsc::Receiver<Event> {
        let (tx, rx) = mpsc::sync_channel(256);
        self.subscribers.lock().unwrap().push(tx);
        rx
    }

    /// Register a Job and start it, returning as soon as it is registered.
    ///
    /// The cut runs on a thread of its own because `DeviceManager::cut` does not
    /// return until the Job reaches its first pause point or finishes
    /// (`manager.rs:648-668`) — for a machine that can be polled, a single-Pass Job
    /// has no pause point, so a synchronous dispatch would hold the client's
    /// connection open for the whole cut. The client learns the `job_id` from the
    /// event stream, which already carries it.
    ///
    /// Refusals happen in this order and none of them touch the machine: unknown
    /// cutter, machine mismatch, Preflight.
    pub fn dispatch(
        self: &Arc<Self>,
        dispatch_id: DispatchId,
        device: &str,
        machine_id: &str,
        passes: Vec<CutPass>,
    ) -> Result<Admitted, Refusal> {
        // Before the cutter is even looked up: an id this host would have to remember for an hour
        // is the one part of a dispatch whose cost outlives the request carrying it.
        if dispatch_id.0.len() > MAX_DISPATCH_ID {
            return Err(Refusal::DispatchIdTooLong { max: MAX_DISPATCH_ID });
        }

        let slot = self.slot(device).ok_or_else(|| Refusal::UnknownDevice(device.to_string()))?;

        if slot.info.machine_id != machine_id {
            return Err(Refusal::MachineMismatch {
                dispatched: machine_id.to_string(),
                attached: slot.info.machine_id.clone(),
            });
        }
        if passes.is_empty() {
            return Err(Refusal::Preflight(PassFault::NoPasses));
        }

        // Against the Driver actually attached, which is the only reason to check
        // again at all: the client planned against a machine it believed was here.
        let driver = self
            .factory
            .driver_for(&slot.info.machine_id)
            .ok_or_else(|| Refusal::UnknownDevice(device.to_string()))?;
        check_passes(&passes, driver.profile(), &driver.caps()).map_err(Refusal::Preflight)?;

        // One critical section for both claims, so no other request can see a
        // half-claimed cutter. Split, the id was visible as taken while the cutter
        // was still being asked about: a concurrent retry read that id, was told
        // Accepted, and then the first request found the cutter Busy and handed the
        // id back — a caller promised a Job that never existed, and the id free
        // again for a third attempt to spend.
        //
        // `status()` reads the cell the worker publishes and never blocks
        // (`driver-core/src/manager.rs:117`), so the device is not called from
        // under this lock, only asked what it last said.
        {
            let mut admission = slot.admission.lock().unwrap();
            admission.forget_expired(Instant::now());
            // `insert` reports whether the id was already there, so a duplicate
            // cannot slip through the gap a separate `contains` would leave. That
            // gap is where a client's retry after a dropped reply would become a
            // second cut of the same material. Checked before the cutter so a retry
            // of a Job already mid-cut stays the no-op it always was, rather than
            // being told Busy by the very state its own first dispatch caused.
            //
            // Answered as `AlreadyAccepted` rather than as a plain success: a no-op and a started
            // Job are different facts, and the operator standing at a cutter that is not moving is
            // the one who needs to be told which happened.
            if admission.accepted.insert(dispatch_id.clone(), Instant::now()).is_some() {
                return Ok(Admitted::AlreadyAccepted);
            }
            // What is legal now is `actions`' answer, not ours to infer — plus the
            // one thing `actions` cannot yet know, an already-admitted dispatch on
            // its way to the manager. A cutter that never connected is kept so its
            // snapshot can say so, and accepting a Job for it would burn the
            // dispatch id on work that cannot start.
            if admission.starting || !slot.manager.status().actions.cut {
                admission.accepted.remove(&dispatch_id);
                return Err(Refusal::Device(DeviceError::Busy));
            }
            // Only once this dispatch is keeping its place. Capping between the insert and the
            // refusal above spent an old id to make room for one that was then handed straight
            // back — a request that started nothing, costing the dedupe history of one that did.
            admission.forget_oldest_beyond_cap();
            admission.starting = true;
        }

        let device = device.to_string();
        let claim = StartingClaim { host: self.clone(), device: device.clone() };
        let id = dispatch_id.clone();
        // `Builder`, not `thread::spawn`: spawn *panics* when a thread cannot be created, and this
        // is a daemon that must survive the day it runs out of threads. On the error path the
        // closure — and the claim inside it — is dropped, which is what puts the cutter back.
        let started = thread::Builder::new().name(format!("dispatch {device}")).spawn(move || {
            let claim = claim;
            let Some(slot) = claim.host.slot(&claim.device) else { return };
            // Only now is the claim the manager's to publish: `cut` returns once the
            // Job has reached its first pause point or finished, so from here
            // `actions` describes this Job rather than the cutter it found free.
            match slot.manager.cut(passes) {
                Ok(job_id) => *slot.job_id.lock().unwrap() = Some(job_id),
                Err(e) => {
                    // A refusal before any motion emits no event and moves no state,
                    // so nothing else will tell anyone. Give the id back: a retry
                    // after a Job that never started must be able to run.
                    slot.admission.lock().unwrap().accepted.remove(&id);
                    eprintln!("cut host: {} refused the job: {e}", claim.device);
                }
            }
            // `claim` drops here, clearing `starting` — after the id has been handed back, so no
            // window shows a free cutter still holding the id of a Job that never ran.
        });

        if let Err(e) = started {
            eprintln!("cut host: {device} could not start a worker for the dispatch: {e}");
            if let Some(slot) = self.slot(&device) {
                slot.admission.lock().unwrap().accepted.remove(&dispatch_id);
            }
            // A sentence, because `DeviceError::Io`'s payload is read verbatim by whoever the
            // client hands it to — and an `io::Error` from a failed `spawn` on its own
            // ("Resource temporarily unavailable (os error 35)") names nothing an operator could
            // act on.
            return Err(Refusal::Device(DeviceError::Io(format!(
                "this host could not start a worker for the cut ({e})"
            ))));
        }
        Ok(Admitted::Started)
    }

    /// What the daemon's shutdown guard asks (`crate::shutdown` is the caller). Built on
    /// `driver-core`'s own predicate rather than a second reading of the phases, plus the one
    /// thing that predicate cannot see: a dispatch admitted and not yet inside `manager.cut`.
    /// Without it a signal landing in that window exits past a Job whose client has already been
    /// told `Accepted` — see `DeviceSlot::is_claimed`.
    pub fn is_any_cut_active(&self) -> bool {
        self.slots.values().any(|s| s.is_claimed())
    }

    /// Every cutter this host is holding, and what it is holding it for.
    ///
    /// The same predicate `is_any_cut_active` decides on, so a daemon that says it is waiting can
    /// always name what for. Filtering on `is_active` instead let the shutdown guard announce a
    /// cut and then print nothing, in exactly the window `starting` exists to cover — an empty
    /// list under "a cut is still running" reads as a confused guard worth forcing past (#124).
    pub fn claims(&self) -> Vec<Claim> {
        self.order
            .iter()
            .filter_map(|id| self.slots.get(id))
            .filter(|s| s.is_claimed())
            .map(|s| {
                // One lock per statement, so nothing here is held while another is taken. Written
                // as a struct expression, the `job_id` temporary lived to the end of that
                // expression and was still held when `admission` was locked — the opposite of
                // `snapshots`' order, which is a cycle between the poll and this report.
                let job_id = *s.job_id.lock().unwrap();
                let starting = s.admission.lock().unwrap().starting;
                Claim { device: s.info.instance_id.clone(), phase: s.manager.status().phase, job_id, starting }
            })
            .collect()
    }

    /// Cancel, resume and confirm take no client identity on purpose. One token is
    /// one trust level: whoever walks to the cutter to swap material for a Pass a
    /// machine cannot be polled through is not necessarily sitting at the laptop
    /// that dispatched the Job.
    pub fn cancel(&self, device: &str) -> Result<(), Refusal> {
        self.with_slot(device, |slot| {
            slot.manager.cancel();
            Ok(())
        })
    }

    pub fn resume(&self, device: &str) -> Result<(), Refusal> {
        self.with_slot(device, |slot| slot.manager.resume().map_err(Refusal::Device))
    }

    pub fn confirm_pass_done(&self, device: &str) -> Result<(), Refusal> {
        self.with_slot(device, |slot| slot.manager.confirm_pass_done().map_err(Refusal::Device))
    }

    /// Drop the cutter's transport and open it again, re-running `DeviceManager::connect`'s
    /// identity probe against real hardware.
    ///
    /// This is the daemon's whole answer to a cancel whose stop nothing confirmed.
    /// `driver-core` refuses another Job there until the transport is re-opened, and the daemon
    /// otherwise connects each cutter exactly once at startup — so without this, one cancelled
    /// Puma takes that cutter out until someone restarts `cuthulhu-cutd`. Re-opening is also the
    /// only honest clearance available: it makes the machine answer a status query again, rather
    /// than the software deciding the blade must have stopped by now.
    ///
    /// Refused while the cutter is claimed, which is the one thing this must never do — dropping
    /// a transport under a moving blade abandons the Job with nothing left to cancel it.
    ///
    /// "Claimed" has to include a dispatch that has been admitted and has not yet reached
    /// `manager.cut`, which `actions` cannot see. `cut` and `disconnect` are two sends on one
    /// channel: if `Cut` wins that race the worker transmits the Pass — motion — parks, and only
    /// then processes the `Disconnect`, whose arm drops the transport and the parked Job
    /// unconditionally. Reading `is_active` alone accepts a reconnect there, which is why this
    /// asks `is_claimed`.
    pub fn reconnect(&self, device: &str) -> Result<(), Refusal> {
        self.with_slot(device, |slot| {
            if slot.is_claimed() {
                return Err(Refusal::Device(DeviceError::Busy));
            }
            slot.manager.disconnect().map_err(Refusal::Device)?;
            slot.manager.connect(slot.info.clone()).map_err(Refusal::Device)
        })
    }

    fn with_slot(
        &self,
        device: &str,
        f: impl FnOnce(&DeviceSlot) -> Result<(), Refusal>,
    ) -> Result<(), Refusal> {
        match self.slot(device) {
            Some(slot) => f(slot),
            None => Err(Refusal::UnknownDevice(device.to_string())),
        }
    }

    pub(crate) fn slot(&self, device: &str) -> Option<&DeviceSlot> {
        self.slots.get(device)
    }

    /// Drops subscribers whose client has gone. A detached client is the normal
    /// case here, not a fault — the Job it started carries on without it.
    //
    // ponytail: a full queue drops the event rather than blocking the pump — a client
    // this far behind rebuilds from `Snapshot` on its next call anyway. Give events
    // their own connection if that ever stops being true.
    fn broadcast(&self, event: Event) {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|tx| {
            match tx.try_send(Event { device: event.device.clone(), event: event.event.clone() }) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => true,
                Err(mpsc::TrySendError::Disconnected(_)) => false,
            }
        });
    }
}

/// Mock cutters, for this crate's own tests and for the integration tests in
/// `tests/`. Public rather than `#[cfg(test)]` because `tests/` compiles as a
/// separate crate and cannot reach test-only code — the cost of shipping two
/// fakes in the public API, paid so that the end-to-end test drives the same
/// `Host` everything else does.
pub mod testing {
    use std::collections::VecDeque;

    use driver_core::{
        DeviceBackendFactory, DeviceInfo, Driver, DriverError, Job, MachineCaps, MachineProfile,
        MockTransport, Transport, TransportError, TransportKind,
    };

    /// A Driver that parks rather than polls. `MockTransport` answers no status
    /// query, so a pollable machine would sit out `DeviceManager`'s 60s completion
    /// budget and then fail — the same reason `apps/desktop/src/device.rs`'s test
    /// Driver makes this choice.
    pub struct TestDriver {
        pub profile: MachineProfile,
        pub caps: MachineCaps,
    }
    impl Driver for TestDriver {
        fn profile(&self) -> &MachineProfile { &self.profile }
        fn caps(&self) -> MachineCaps { self.caps }
        fn session_begin(&self) -> Vec<u8> { b"BEGIN".to_vec() }
        fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError> {
            Ok(format!("PASS{}", pass.polylines.len()).into_bytes())
        }
        fn pass_park(&self) -> Vec<u8> { b"PARK".to_vec() }
        fn session_end(&self) -> Vec<u8> { b"END".to_vec() }
        fn abort_bytes(&self) -> Option<Vec<u8>> { Some(b"ABORT".to_vec()) }
    }

    /// Two cutters, deliberately of different machines: a test that fails one and
    /// asserts the other is untouched has to be able to tell them apart.
    pub struct TwoCutterFactory;

    pub const CAMEO: &str = "usb:1:4";
    pub const PUMA: &str = "serial:/dev/ttyUSB0";

    impl DeviceBackendFactory for TwoCutterFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> {
            vec![
                DeviceInfo {
                    instance_id: CAMEO.into(),
                    machine_id: "cameo5".into(),
                    transport: TransportKind::Usb { locator: "1:4".into() },
                    candidate: false,
                    host: None,
                },
                DeviceInfo {
                    instance_id: PUMA.into(),
                    machine_id: "puma".into(),
                    transport: TransportKind::Serial { path: "/dev/ttyUSB0".into(), baud: 9600 },
                    candidate: true,
                    host: None,
                },
            ]
        }
        fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
            let profile = match machine_id {
                "cameo5" => MachineProfile { id: "cameo5".into(), name: "Cameo".into(), width_mm: 300.0, height_mm: 200.0 },
                "puma" => MachineProfile { id: "puma".into(), name: "Puma".into(), width_mm: 600.0, height_mm: 600.0 },
                _ => return None,
            };
            Some(Box::new(TestDriver {
                profile,
                caps: MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true },
            }))
        }
        fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            // A candidate device owes a probe reply before `DeviceManager::connect` will
            // accept it (`manager.rs:598`), and the Puma here is a candidate because a
            // serial port announces nothing about what is on the other end. Reads stay
            // empty after that one: neither test machine can be polled, so a bug that
            // polled anyway would hit an immediate `Timeout` and surface fast.
            Ok(Box::new(MockTransport {
                reads: VecDeque::from(vec![Ok(b"0\r".to_vec())]),
                ..Default::default()
            }))
        }
    }

    /// Holds every Transport write until the test opens it. This is what makes "dispatch
    /// does not wait for the cut" falsifiable at all: `DeviceManager::cut` returns once
    /// the Job reaches its first pause point (`manager.rs:153-156`), and `TestDriver`
    /// parks for confirmation right after one Pass — so against the fixtures above even
    /// a dispatch that called `cut` synchronously would reply promptly. A shut gate
    /// wedges the worker *inside* `cut`'s transmit, where only a dispatch that truly
    /// did not wait can answer.
    #[derive(Clone, Default)]
    pub struct WriteGate(std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);

    impl WriteGate {
        pub fn open(&self) {
            let (open, cvar) = &*self.0;
            *open.lock().unwrap() = true;
            cvar.notify_all();
        }
        fn wait_open(&self) {
            let (open, cvar) = &*self.0;
            let mut open = open.lock().unwrap();
            while !*open {
                open = cvar.wait(open).unwrap();
            }
        }
    }

    struct GatedTransport(WriteGate);

    impl Transport for GatedTransport {
        fn write(&mut self, b: &[u8]) -> Result<usize, TransportError> {
            self.0.wait_open();
            Ok(b.len())
        }
        fn read(&mut self, _buf: &mut [u8], _t: std::time::Duration) -> Result<usize, TransportError> {
            Err(TransportError::Timeout)
        }
    }

    /// One Cameo behind a [`WriteGate`]. `candidate: false`, deliberately: connecting a
    /// candidate probes the machine — a write — and `Host::start` connects every cutter
    /// before returning, so a probe would park startup itself at the gate.
    #[derive(Default)]
    pub struct GatedCutterFactory {
        gate: WriteGate,
    }

    pub const GATED: &str = "usb:1:7";

    impl GatedCutterFactory {
        pub fn gate(&self) -> WriteGate {
            self.gate.clone()
        }
    }

    impl DeviceBackendFactory for GatedCutterFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> {
            vec![DeviceInfo {
                instance_id: GATED.into(),
                machine_id: "cameo5".into(),
                transport: TransportKind::Usb { locator: "1:7".into() },
                candidate: false,
                host: None,
            }]
        }
        fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
            if machine_id != "cameo5" {
                return None;
            }
            Some(Box::new(TestDriver {
                profile: MachineProfile { id: "cameo5".into(), name: "Cameo".into(), width_mm: 300.0, height_mm: 200.0 },
                caps: MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true },
            }))
        }
        fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            Ok(Box::new(GatedTransport(self.gate.clone())))
        }
    }

    /// One cutter that never connects. `candidate: false` so `DeviceManager::connect`
    /// needs no probe reply before failing — unlike `TwoCutterFactory`'s Puma, there
    /// is no successful open to script a read for.
    pub struct NeverConnectsFactory;

    pub const UNREACHABLE: &str = "usb:9:9";

    impl DeviceBackendFactory for NeverConnectsFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> {
            vec![DeviceInfo {
                instance_id: UNREACHABLE.into(),
                machine_id: "cameo5".into(),
                transport: TransportKind::Usb { locator: "9:9".into() },
                candidate: false,
                host: None,
            }]
        }
        fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
            match machine_id {
                "cameo5" => Some(Box::new(TestDriver {
                    profile: MachineProfile { id: "cameo5".into(), name: "Cameo".into(), width_mm: 300.0, height_mm: 200.0 },
                    caps: MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true },
                })),
                _ => None,
            }
        }
        fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            Err(TransportError::NotFound)
        }
    }

    /// Waits for a dispatch to land its Job on the slot, and returns that Job's id.
    ///
    /// Watching the phase is not enough for a test that reads `job_id` or `starting`, or that
    /// needs the cutter released: the worker publishes the pause phase *before* `manager.cut`
    /// replies, and the slot's `job_id` write and the `StartingClaim` drop happen after that
    /// reply, on the dispatch thread. A test that read them at first sight of the phase had to
    /// win a scheduling race to pass, and lost it on a loaded runner (#129).
    pub fn wait_for_job(host: &super::Host, device: &str) -> u64 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let slot = host.slot(device).expect("known cutter");
            // `job_id` is written before the claim drops, so once `starting` clears the id is there.
            if !slot.admission.lock().unwrap().starting {
                if let Some(job_id) = *slot.job_id.lock().unwrap() {
                    return job_id;
                }
            }
            assert!(std::time::Instant::now() < deadline, "{device}'s dispatch never landed a Job");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use std::sync::Arc;

    use crate::protocol::Refusal;
    use driver_core::manager::CutPass;
    use driver_core::{Job, Settings};
    use geometry::Point;

    fn square_pass() -> CutPass {
        CutPass {
            job: Job {
                polylines: vec![vec![
                    Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 },
                    Point { x: 10.0, y: 10.0 }, Point { x: 0.0, y: 0.0 },
                ]],
                settings: Settings::default(),
            },
        }
    }

    /// Waits for `device` to reach a phase the test can assert on. The worker is a
    /// separate thread, so a bare assertion after `dispatch` would race it.
    fn wait_for(host: &Host, device: &str, want: driver_core::Phase) -> driver_core::CutStatus {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let status = host.slot(device).unwrap().manager.status();
            if status.phase == want {
                return status;
            }
            assert!(std::time::Instant::now() < deadline, "{device} never reached {want:?}, sat at {:?}", status.phase);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn a_dispatch_to_an_unknown_cutter_is_refused() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let refusal = host
            .dispatch(DispatchId("d-1".into()), "usb:9:9", "cameo5", vec![square_pass()])
            .unwrap_err();
        assert!(matches!(refusal, Refusal::UnknownDevice(id) if id == "usb:9:9"));
    }

    /// The refusal the network hop exists to make possible: the client planned for
    /// one machine and a different one is on that port.
    #[test]
    fn a_dispatch_naming_the_wrong_machine_is_refused_before_anything_moves() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let refusal = host
            .dispatch(DispatchId("d-1".into()), CAMEO, "puma", vec![square_pass()])
            .unwrap_err();
        match refusal {
            Refusal::MachineMismatch { dispatched, attached } => {
                assert_eq!(dispatched, "puma");
                assert_eq!(attached, "cameo5");
            }
            other => panic!("expected MachineMismatch, got {other:?}"),
        }
        assert_eq!(host.slot(CAMEO).unwrap().manager.status().phase, driver_core::Phase::Idle);
    }

    #[test]
    fn a_dispatch_that_fails_preflight_is_refused_with_its_sentence() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let off_the_bed = CutPass {
            job: Job {
                polylines: vec![vec![Point { x: 0.0, y: 0.0 }, Point { x: 400.0, y: 0.0 }]],
                settings: Settings::default(),
            },
        };
        let refusal = host
            .dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![off_the_bed])
            .unwrap_err();
        match refusal {
            // The Cameo's test bed is 300x200; the Puma's is 600x600, so this is
            // refused only because the check ran against the machine actually there.
            Refusal::Preflight(fault) => {
                let message = fault.to_string();
                assert!(message.contains("300 x 200"), "got: {message}");
            }
            other => panic!("expected Preflight, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_dispatch_is_refused() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        assert!(host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", Vec::new()).is_err());
    }

    /// The cutter is kept, not dropped, so its snapshot can report the failure —
    /// but that must not make it look dispatchable to a caller reading `actions`.
    #[test]
    fn a_dispatch_to_a_cutter_that_never_connected_is_refused() {
        let host = Host::start(Arc::new(NeverConnectsFactory));
        assert!(!host.slot(UNREACHABLE).unwrap().manager.status().actions.cut);

        let refusal = host
            .dispatch(DispatchId("d-1".into()), UNREACHABLE, "cameo5", vec![square_pass()])
            .unwrap_err();
        assert!(matches!(refusal, Refusal::Device(driver_core::manager::DeviceError::Busy)));
    }

    /// `dispatch` must return without waiting for the cut. While the gate is shut the
    /// worker is wedged inside `DeviceManager::cut`'s transmit, so the cut categorically
    /// has not finished — a reply arriving then is a reply that beat it, and a dispatch
    /// that regressed to calling `cut` synchronously sits at the gate and fails the
    /// receive loudly. Nothing here measures time except the bound on a hang: a
    /// wall-clock assertion also measured the scheduler (#132), and the ungated fixtures
    /// cannot falsify the property at all, since `cut` returns at the Job's first pause
    /// point and `TestDriver` pauses immediately (see `WriteGate`).
    #[test]
    fn a_dispatch_returns_before_the_cut_finishes() {
        let factory = GatedCutterFactory::default();
        let gate = factory.gate();
        let host = Host::start(Arc::new(factory));

        let (tx, rx) = mpsc::channel();
        let dispatching = host.clone();
        // `let _`: on the timeout path below `rx` is gone before the helper sends, and
        // that send failing must not panic over the verdict that matters.
        let helper = thread::spawn(move || {
            let _ = tx.send(dispatching.dispatch(
                DispatchId("d-1".into()),
                GATED,
                "cameo5",
                vec![square_pass()],
            ));
        });
        // Generous: this bounds a hang, not how fast dispatch is. The two errors are
        // different verdicts — Timeout is the blocking this test exists to catch,
        // Disconnected is the helper dying (a panic inside `dispatch`) before replying.
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(admitted) => admitted.unwrap(),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Un-wedge the worker and the helper before failing: the rest of the
                // test run shares this process.
                gate.open();
                panic!("dispatch waited for the cut");
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the dispatch thread died before replying")
            }
        };
        helper.join().expect("the dispatch helper panicked after replying");

        // Only now may the cut proceed; it parks for confirmation as usual.
        gate.open();
        wait_for(&host, GATED, driver_core::Phase::AwaitingConfirmation);
    }

    /// The guard against cutting the same material twice after a dropped reply.
    #[test]
    fn a_repeated_dispatch_id_starts_nothing_further() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        let job_after_first = wait_for_job(&host, CAMEO);

        // A retry arrives. The cutter is mid-Job, so a second cut would be refused
        // Busy anyway — the assertion that matters is that it is accepted as a
        // no-op rather than surfacing an error to a client that did nothing wrong.
        let again = host
            .dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()])
            .unwrap();
        assert_eq!(again, Admitted::AlreadyAccepted, "a no-op must say it was one");
        assert_eq!(*host.slot(CAMEO).unwrap().job_id.lock().unwrap(), Some(job_after_first));
    }

    /// An id is remembered for an hour, so its length is a cost this host carries rather than one
    /// the request carrying it pays. Refused before the cutter is even looked up, and refused
    /// rather than truncated — a truncated id is a different id, and a retry arriving under a
    /// different name is a second cut.
    #[test]
    fn a_dispatch_id_longer_than_a_host_will_remember_is_refused() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let huge = DispatchId("d".repeat(MAX_DISPATCH_ID + 1));
        match host.dispatch(huge.clone(), CAMEO, "cameo5", vec![square_pass()]) {
            Err(Refusal::DispatchIdTooLong { max }) => assert_eq!(max, MAX_DISPATCH_ID),
            other => panic!("expected DispatchIdTooLong, got {other:?}"),
        }
        assert!(!holds_dispatch_id(&host, CAMEO, &huge.0), "a refused id must not be remembered");
        assert_eq!(host.slot(CAMEO).unwrap().manager.status().phase, driver_core::Phase::Idle);

        // The merely long is still fine: the cap refuses abuse, not the desktop's own ids, which
        // run to about sixty characters.
        host.dispatch(DispatchId("d".repeat(MAX_DISPATCH_ID)), CAMEO, "cameo5", vec![square_pass()])
            .expect("an id at the limit is acceptable");
    }

    /// The hour is not a bound on its own: nothing prunes until the next dispatch arrives, so a
    /// daemon left idle after a burst keeps every id of that burst. The cap is what makes the
    /// memory a fixed cost rather than a function of how fast a client can dispatch.
    #[test]
    fn a_cutter_remembers_only_so_many_ids_at_once() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let slot = host.slot(CAMEO).unwrap();
        let mut admission = slot.admission.lock().unwrap();
        // Ages a millisecond apart, so "oldest" is well defined rather than a tie.
        let base = Instant::now();
        for n in 0..MAX_REMEMBERED_IDS + 50 {
            admission
                .accepted
                .insert(DispatchId(format!("d-{n}")), base + Duration::from_millis(n as u64));
        }
        admission.forget_oldest_beyond_cap();

        assert_eq!(admission.accepted.len(), MAX_REMEMBERED_IDS);
        assert!(
            !admission.accepted.contains_key(&DispatchId("d-0".into())),
            "the oldest id survived the cap"
        );
        assert!(
            admission.accepted.contains_key(&DispatchId(format!("d-{}", MAX_REMEMBERED_IDS + 49))),
            "the newest id — the one a retry would actually name — was evicted"
        );
    }

    /// A dispatch that is refused must not spend another Job's place in the dedupe history. The
    /// cap used to run between the insert and the refusal, so a request that started nothing —
    /// aimed at a busy cutter, say — evicted the oldest id to make room for one handed straight
    /// back, and a retry naming that evicted id would then have been cut a second time.
    #[test]
    fn a_refused_dispatch_does_not_evict_an_id_to_make_room_for_itself() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        // Fill to the cap with ids of known ages, oldest first.
        {
            let slot = host.slot(CAMEO).unwrap();
            let mut admission = slot.admission.lock().unwrap();
            let base = Instant::now();
            for n in 0..MAX_REMEMBERED_IDS {
                admission
                    .accepted
                    .insert(DispatchId(format!("d-{n}")), base + Duration::from_millis(n as u64));
            }
        }
        // A busy cutter, so the next dispatch is refused rather than admitted.
        host.slot(CAMEO).unwrap().admission.lock().unwrap().starting = true;

        assert!(matches!(
            host.dispatch(DispatchId("d-refused".into()), CAMEO, "cameo5", vec![square_pass()]),
            Err(Refusal::Device(DeviceError::Busy))
        ));

        let admission = host.slot(CAMEO).unwrap().admission.lock().unwrap();
        assert!(
            admission.accepted.contains_key(&DispatchId("d-0".into())),
            "a refused dispatch evicted the oldest id on its way to being refused"
        );
        assert!(!admission.accepted.contains_key(&DispatchId("d-refused".into())));
        assert_eq!(admission.accepted.len(), MAX_REMEMBERED_IDS);
    }

    /// An id is remembered for a stated length of time, not forever. Kept forever, a daemon up for
    /// months answered `Ok` to a dispatch carrying an id it had seen in January and cut nothing —
    /// and no client could know that, because nothing on the wire said how long ids last (#119).
    #[test]
    fn an_accepted_id_is_forgotten_once_it_is_older_than_the_retention() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);
        assert!(holds_dispatch_id(&host, CAMEO, "d-1"));

        // Looked at from a point past the retention rather than by ageing the entry: `Instant` is
        // monotonic from an unspecified epoch, so a machine booted minutes ago cannot subtract an
        // hour from `now` at all.
        {
            let slot = host.slot(CAMEO).unwrap();
            let mut admission = slot.admission.lock().unwrap();
            admission.forget_expired(Instant::now() + ID_RETENTION + Duration::from_secs(1));
        }
        assert!(!holds_dispatch_id(&host, CAMEO, "d-1"), "an id past its retention is still held");

        // And a fresh one is not swept up with it.
        let slot = host.slot(CAMEO).unwrap();
        let mut admission = slot.admission.lock().unwrap();
        admission.accepted.insert(DispatchId("d-fresh".into()), Instant::now());
        admission.forget_expired(Instant::now());
        assert!(admission.accepted.contains_key(&DispatchId("d-fresh".into())));
    }

    /// A dispatch id forgotten by the host is dispatchable again — the property the retention is
    /// for. Without it the operator's press of Cut is answered `AlreadyAccepted` and nothing moves.
    #[test]
    fn a_forgotten_id_can_start_a_job_again() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        // Landed, not merely parked: the second dispatch below needs the first one's
        // `starting` claim gone, or it is refused Busy rather than answered about its id.
        wait_for_job(&host, CAMEO);
        host.confirm_pass_done(CAMEO).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::Idle);

        host.slot(CAMEO)
            .unwrap()
            .admission
            .lock()
            .unwrap()
            .forget_expired(Instant::now() + ID_RETENTION + Duration::from_secs(1));

        let admitted = host
            .dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()])
            .expect("an id the host no longer remembers is a new Job");
        assert_eq!(admitted, Admitted::Started);
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);
    }

    /// The claim comes off however the dispatch's worker ends. The two ways it used not to — a
    /// panic unwinding out of `manager.cut`, and a thread that could never be spawned — are both
    /// outside any explicit assignment, and either one left the cutter claimed by a Job that would
    /// never run: `reconnect` refused, every dispatch refused, the shutdown guard held, and a
    /// restart of the daemon was the only exit (#120).
    #[test]
    fn a_dropped_starting_claim_releases_the_cutter() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.slot(CAMEO).unwrap().admission.lock().unwrap().starting = true;
        assert!(host.slot(CAMEO).unwrap().is_claimed(), "the fixture has to actually claim it");

        drop(StartingClaim { host: host.clone(), device: CAMEO.into() });

        assert!(!host.slot(CAMEO).unwrap().is_claimed(), "the cutter is still claimed by nothing");
        assert!(host.reconnect(CAMEO).is_ok(), "and still unreachable through the way back");
    }

    /// Runs `dispatch` on two threads released together, so both are inside it at
    /// once. A barrier is enough to reach the window that matters here: it is as
    /// wide as a thread spawn plus a command round-trip, so the second dispatch
    /// arrives long before the first one's worker has told the manager anything.
    fn dispatch_together(
        host: &Arc<Host>,
        device: &str,
        ids: [&str; 2],
    ) -> [Result<Admitted, Refusal>; 2] {
        let gate = std::sync::Barrier::new(2);
        std::thread::scope(|s| {
            let one = s.spawn(|| {
                gate.wait();
                host.dispatch(DispatchId(ids[0].into()), device, "cameo5", vec![square_pass()])
            });
            gate.wait();
            let other =
                host.dispatch(DispatchId(ids[1].into()), device, "cameo5", vec![square_pass()]);
            [one.join().unwrap(), other]
        })
    }

    fn holds_dispatch_id(host: &Host, device: &str, id: &str) -> bool {
        let admission = host.slot(device).unwrap().admission.lock().unwrap();
        admission.accepted.contains_key(&DispatchId(id.into()))
    }

    /// The retry and its own original, arriving together at a cutter that is busy.
    /// Neither can start a Job, so neither may be told one was accepted — the split
    /// claim told the retry exactly that, because it could see an id whose owner was
    /// still being refused. And the id must not survive a dispatch that started
    /// nothing, or the operator's next attempt is silently a no-op.
    #[test]
    fn two_dispatches_of_one_id_to_a_busy_cutter_promise_no_job() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-first".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        let job = wait_for_job(&host, CAMEO);

        // Free-running rather than paired at a barrier: the window a split claim
        // leaves is only the few instructions between releasing the dedupe lock and
        // reading `actions`, which is narrower than the microsecond it takes a
        // barrier to wake a thread. Two threads looping out of step sweep across it
        // instead of aiming at it, and sample it thousands of times.
        let answers = std::thread::scope(|s| {
            let racers: Vec<_> = (0..2)
                .map(|_| {
                    s.spawn(|| {
                        (0..20_000)
                            .map(|_| {
                                host.dispatch(
                                    DispatchId("d-retry".into()),
                                    CAMEO,
                                    "cameo5",
                                    vec![square_pass()],
                                )
                            })
                            .filter(|a| !matches!(a, Err(Refusal::Device(DeviceError::Busy))))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            racers.into_iter().flat_map(|r| r.join().unwrap()).collect::<Vec<_>>()
        });

        assert!(answers.is_empty(), "a busy cutter promised {answers:?}");
        assert!(!holds_dispatch_id(&host, CAMEO, "d-retry"), "an id kept for a Job that never was");
        assert_eq!(*host.slot(CAMEO).unwrap().job_id.lock().unwrap(), Some(job), "the Job never changed");
    }

    /// Two different ids at an idle cutter. Exactly one may be accepted: the cut
    /// runs on a thread of its own, so `actions` still reads free for as long as it
    /// takes that thread to reach the manager, and a claim that is not held across
    /// that gap admits both. The loser must be refused here and not spawn anything,
    /// because a second Job that merely waits for the first is the queueing this
    /// design refuses.
    #[test]
    fn two_dispatches_to_one_idle_cutter_start_exactly_one_job() {
        for round in 0..8 {
            let host = Host::start(Arc::new(TwoCutterFactory));
            let answers = dispatch_together(&host, CAMEO, ["d-a", "d-b"]);

            let accepted = answers.iter().filter(|a| a.is_ok()).count();
            assert_eq!(accepted, 1, "round {round}: an idle cutter took {accepted} Jobs");
            let refusal = answers.iter().find_map(|a| a.as_ref().err()).unwrap();
            assert!(matches!(refusal, Refusal::Device(DeviceError::Busy)), "got {refusal:?}");

            let first = wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);
            assert!(first.actions.confirm);
            assert_eq!(wait_for_job(&host, CAMEO), 1, "round {round}: the winner's Job landed");
            host.confirm_pass_done(CAMEO).unwrap();
            let done = wait_for(&host, CAMEO, driver_core::Phase::Idle);
            assert_eq!(done.ended, Some(driver_core::Ended::Completed));

            // Nothing was left holding a Job, so nothing starts once the first ends.
            std::thread::sleep(std::time::Duration::from_millis(50));
            let after = host.slot(CAMEO).unwrap().manager.status();
            assert_eq!(after.phase, driver_core::Phase::Idle, "round {round}: a second Job queued");
            assert_eq!(*host.slot(CAMEO).unwrap().job_id.lock().unwrap(), Some(1));
        }
    }

    #[test]
    fn a_dispatch_records_the_job_a_reattaching_client_would_ask_about() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for_job(&host, CAMEO);

        let snap = host.snapshots().into_iter().find(|s| s.info.instance_id == CAMEO).unwrap();
        assert!(snap.job_id.is_some(), "a dispatched cutter reports which Job is on it");
        assert!(snap.status.actions.confirm, "and what may be done to it");
    }

    /// #59's isolation criterion. Both cutters are given work; one is cancelled;
    /// the other must be entirely unaffected.
    #[test]
    fn a_failure_on_one_cutter_leaves_the_other_cutting() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-cameo".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        host.dispatch(DispatchId("d-puma".into()), PUMA, "puma", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);
        wait_for(&host, PUMA, driver_core::Phase::AwaitingConfirmation);

        host.slot(CAMEO).unwrap().manager.cancel();
        let cameo = wait_for(&host, CAMEO, driver_core::Phase::Idle);
        assert_eq!(cameo.ended, Some(driver_core::Ended::Cancelled));

        let puma = host.slot(PUMA).unwrap().manager.status();
        assert_eq!(puma.phase, driver_core::Phase::AwaitingConfirmation, "the other cutter kept its Job");
        assert!(puma.actions.confirm);
    }

    #[test]
    fn a_host_with_a_cut_in_flight_reports_it_as_active() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);
        // Landed, not merely parked: `is_any_cut_active` is also satisfied by the dispatch's
        // transient `starting` claim, and this test is about the Job-in-flight branch.
        wait_for_job(&host, CAMEO);
        assert!(host.is_any_cut_active(), "a Host with a cut in flight must report itself active");
    }

    #[test]
    fn a_host_holds_every_cutter_it_enumerates() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let ids: Vec<String> = host.devices().into_iter().map(|d| d.instance_id).collect();
        assert_eq!(ids, vec![CAMEO.to_string(), PUMA.to_string()]);
    }

    /// Each cutter is connected by the host, not by a client. A snapshot taken
    /// before anything is dispatched must therefore already read `Idle` — a
    /// `Disconnected` here would mean a client had to connect first, which is the
    /// race this design removes.
    #[test]
    fn every_cutter_is_connected_and_idle_before_any_client_asks() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let snaps = host.snapshots();
        assert_eq!(snaps.len(), 2);
        for s in &snaps {
            assert_eq!(s.status.phase, driver_core::Phase::Idle, "{} should be connected", s.info.instance_id);
            assert!(s.status.actions.cut, "an idle cutter accepts a cut");
            assert_eq!(s.job_id, None, "nothing has run yet");
            assert!(!s.claimed, "an idle cutter is not held for a dispatch");
        }
    }

    #[test]
    fn snapshots_and_devices_agree_on_which_cutters_exist() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let from_devices: Vec<String> = host.devices().into_iter().map(|d| d.instance_id).collect();
        let from_snaps: Vec<String> = host.snapshots().into_iter().map(|s| s.info.instance_id).collect();
        assert_eq!(from_devices, from_snaps);
    }

    /// `snapshots` must not hold a slot's `admission` while it takes that slot's `job_id`. It did,
    /// and `claims` — written as a struct expression whose `job_id` temporary outlived its own
    /// statement — took the two the other way round, so a status poll racing the shutdown guard's
    /// report wedged both threads. A daemon in that state holds `admission` forever: every later
    /// dispatch blocks on it, and the watch thread that is the only caller of `exit` can no longer
    /// honour SIGTERM, which is exactly the abandoned-cut outcome the guard exists to prevent.
    ///
    /// Pinned from the second lock, not the first: parking a reader on the lock it takes *first*
    /// proves nothing, because it holds nothing yet. Holding `job_id` here forces the poll to park
    /// on `job_id`, and `admission` being free once it is parked is the whole of the property —
    /// one direction removed is a cycle removed, whichever order the other reader uses. `claims`
    /// takes one lock per statement for the same reason, which its own `is_claimed` filter makes
    /// untestable from outside: it parks on `admission` before it ever reaches `job_id`.
    #[test]
    fn a_status_poll_does_not_hold_admission_while_it_takes_a_job_id() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};

        let host = Host::start(Arc::new(TwoCutterFactory));
        let job_id = host.slot(CAMEO).unwrap().job_id.lock().unwrap();

        // The thread says when it is inside the call, so "not finished" cannot mean "never
        // scheduled" — that reading would let the assertion below pass without the poll ever
        // reaching a lock, which is a test that cannot fail rather than one that does not flake.
        let entered = Arc::new(AtomicBool::new(false));
        let polling = {
            let (parked, entered) = (Arc::clone(&host), Arc::clone(&entered));
            std::thread::spawn(move || {
                entered.store(true, Ordering::SeqCst);
                parked.snapshots()
            })
        };

        let started_by = Instant::now() + Duration::from_secs(2);
        while !entered.load(Ordering::SeqCst) {
            assert!(Instant::now() < started_by, "the poll thread never ran");
            std::thread::sleep(Duration::from_millis(2));
        }
        // Its `admission` section is microseconds, so one long beat past entering the call leaves
        // exactly one place it can be: parked on the `job_id` this test holds.
        std::thread::sleep(Duration::from_millis(200));
        assert!(!polling.is_finished(), "the poll finished without ever taking the held `job_id`");

        let deadline = Instant::now() + Duration::from_millis(100);
        while Instant::now() < deadline {
            assert!(
                host.slot(CAMEO).unwrap().admission.try_lock().is_ok(),
                "the poll held `admission` while it waited for `job_id`, which is half a deadlock"
            );
            std::thread::sleep(Duration::from_millis(2));
        }

        drop(job_id);
        polling.join().expect("the poll must finish once `job_id` is free");
    }

    #[test]
    fn a_host_with_nothing_attached_starts_and_reports_nothing() {
        struct Empty;
        impl driver_core::DeviceBackendFactory for Empty {
            fn list_devices(&self) -> Vec<driver_core::DeviceInfo> { Vec::new() }
            fn driver_for(&self, _: &str) -> Option<Box<dyn driver_core::Driver + Send>> { None }
            fn open_transport(&self, _: &driver_core::DeviceInfo)
                -> Result<Box<dyn driver_core::Transport>, driver_core::TransportError> {
                Err(driver_core::TransportError::NotFound)
            }
        }
        let host = Host::start(Arc::new(Empty));
        assert!(host.devices().is_empty());
        assert!(host.snapshots().is_empty());
        assert!(!host.is_any_cut_active());
    }

    #[test]
    fn an_idle_host_reports_no_active_cut() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        assert!(!host.is_any_cut_active(), "nothing dispatched yet");
    }

    /// A pump holding a strong `Arc<Host>` would leak: `Host` never drops while its
    /// own pump threads keep it alive, so `events` never ends, so the pump never
    /// exits. Dropping every external handle to `host` here must be enough on its
    /// own — no `shutdown` call, nothing else pinning it — for the teardown chain
    /// (`Host` drops `slots` drops each `DeviceManager` drops its `cmd_tx`, the
    /// worker's `recv()` errors and it returns, dropping the event sender the pump
    /// is reading) to reach the pump and end it. Observed as the subscriber's
    /// receiver disconnecting.
    ///
    /// Drains rather than asserting on the first message: `Host::start` connects
    /// each cutter before the pumps are spawned, so a connect-time event can still
    /// be queued and arrive after `subscribe`. Those are legitimate and say nothing
    /// about teardown — only a disconnect does.
    #[test]
    fn dropping_the_host_lets_its_event_pumps_end() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let rx = host.subscribe();
        drop(host);

        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(_) => continue, // a connect-time event that beat the drop
                Err(mpsc::RecvTimeoutError::Disconnected) => break, // what this proves
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("the pumps outlived the Host they were waiting on")
                }
            }
        }
    }

    #[test]
    fn a_confirm_advances_the_cutter_that_was_named() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);

        host.confirm_pass_done(CAMEO).unwrap();
        let done = wait_for(&host, CAMEO, driver_core::Phase::Idle);
        assert_eq!(done.ended, Some(driver_core::Ended::Completed));
    }

    /// A second client confirming a Job it did not start is the case this design
    /// requires to work: whoever swaps the material walks to whatever is nearest.
    #[test]
    fn any_client_may_confirm_a_job_another_dispatched() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("from-the-laptop".into()), PUMA, "puma", vec![square_pass()]).unwrap();
        wait_for(&host, PUMA, driver_core::Phase::AwaitingConfirmation);

        // No dispatch id, no client identity — the host does not track ownership.
        host.confirm_pass_done(PUMA).unwrap();
        assert_eq!(wait_for(&host, PUMA, driver_core::Phase::Idle).ended, Some(driver_core::Ended::Completed));
    }

    #[test]
    fn a_cancel_stops_the_cutter_that_was_named() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);

        host.cancel(CAMEO).unwrap();
        assert_eq!(wait_for(&host, CAMEO, driver_core::Phase::Idle).ended, Some(driver_core::Ended::Cancelled));
    }

    #[test]
    fn every_verb_refuses_an_unknown_cutter() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        assert!(matches!(host.cancel("usb:9:9"), Err(Refusal::UnknownDevice(_))));
        assert!(matches!(host.resume("usb:9:9"), Err(Refusal::UnknownDevice(_))));
        assert!(matches!(host.confirm_pass_done("usb:9:9"), Err(Refusal::UnknownDevice(_))));
        assert!(matches!(host.reconnect("usb:9:9"), Err(Refusal::UnknownDevice(_))));
    }

    /// The daemon connects each cutter once at startup, so without `reconnect` a cancel whose
    /// stop nothing confirmed takes that cutter out until `cuthulhu-cutd` is restarted — and
    /// `TestDriver` parks rather than polls, so no cancel of one can ever confirm. Asserted
    /// through `actions` and the dispatch it governs, never a phase: `Idle` is what the cutter
    /// reports on both sides of the reconnect.
    #[test]
    fn a_reconnect_is_the_way_back_from_a_cancel_that_could_not_confirm_the_stop() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), PUMA, "puma", vec![square_pass()]).unwrap();
        // Landed, not merely parked: `reconnect` below must be refused for the cancel's
        // unconfirmed stop, not for a `starting` claim this dispatch has yet to drop.
        wait_for_job(&host, PUMA);

        host.cancel(PUMA).unwrap();
        let stuck = wait_for_ended(&host, PUMA, driver_core::Ended::Cancelled);
        assert!(!stuck.actions.cut, "nothing saw the machine stop, so no Job may follow it");
        assert!(matches!(
            host.dispatch(DispatchId("d-2".into()), PUMA, "puma", vec![square_pass()]),
            Err(Refusal::Device(DeviceError::Busy))
        ));

        host.reconnect(PUMA).unwrap();
        assert!(host.slot(PUMA).unwrap().manager.status().actions.cut, "a re-opened cutter takes a Job again");
        host.dispatch(DispatchId("d-3".into()), PUMA, "puma", vec![square_pass()]).unwrap();
        wait_for(&host, PUMA, driver_core::Phase::AwaitingConfirmation);
    }

    /// Harmless on a healthy cutter, and refused on a busy one — dropping a transport under a
    /// moving blade would abandon the Job with nothing left to cancel it.
    #[test]
    fn a_reconnect_leaves_an_idle_cutter_alone_and_refuses_a_busy_one() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.reconnect(CAMEO).expect("an idle cutter re-opens");
        assert!(host.slot(CAMEO).unwrap().manager.status().actions.cut);

        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);
        // Landed, not merely parked: a `starting` claim also refuses a reconnect, and the busy
        // half of this test is about the Job in flight.
        wait_for_job(&host, CAMEO);
        assert!(matches!(host.reconnect(CAMEO), Err(Refusal::Device(DeviceError::Busy))));
        // And the parked Job is still there to be answered, not silently dropped.
        assert!(host.slot(CAMEO).unwrap().manager.status().actions.confirm);
    }

    /// The window `actions` cannot see: a dispatch admitted and not yet inside `manager.cut`. The
    /// cutter still publishes `Idle`, so a guard reading only `is_active` accepts — and `cut` and
    /// `disconnect` are two sends on one channel, so if `Cut` wins that race the worker transmits
    /// the Pass, parks, and only then processes the `Disconnect` that drops its transport. A
    /// machine still executing buffered bytes, with the cable pulled.
    ///
    /// Driven by setting the state `dispatch` already keeps, rather than by racing two threads at
    /// a window a thread spawn wide. The fix is a state read, so the state is what to pin: a test
    /// that has to win a race in order to fail is worse than one that cannot miss.
    #[test]
    fn a_dispatch_not_yet_inside_the_manager_blocks_a_reconnect_and_holds_a_shutdown() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        assert!(host.slot(CAMEO).unwrap().manager.status().actions.cut, "idle: `actions` sees nothing");
        assert!(!host.is_any_cut_active());

        host.slot(CAMEO).unwrap().admission.lock().unwrap().starting = true;
        let snapshot = host
            .snapshots()
            .into_iter()
            .find(|s| s.info.instance_id == CAMEO)
            .unwrap();
        assert!(snapshot.status.actions.cut, "status alone still calls it free");
        assert!(snapshot.claimed, "the admission gap must travel over Snapshot");

        assert!(matches!(host.reconnect(CAMEO), Err(Refusal::Device(DeviceError::Busy))));
        assert!(host.is_any_cut_active(), "and the daemon must not exit past it either");

        // What the shutdown guard prints has to agree with what it decided. Reading `is_active`
        // there instead announced "a cut is still running" and then listed nothing, in exactly
        // this window — an empty list under that sentence reads as a guard worth forcing past.
        let claims = host.claims();
        assert_eq!(claims.len(), 1, "a held daemon must be able to name what holds it");
        assert_eq!(claims[0].device, CAMEO);
        assert!(claims[0].starting, "and say that it is a dispatch rather than a Job in flight");
        assert_eq!(claims[0].job_id, None);
    }

    /// The other half of the same agreement: a Job actually in flight is named too, so one
    /// predicate really does serve both readings.
    #[test]
    fn a_job_in_flight_is_named_by_the_same_predicate_that_holds_the_daemon() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        let job = wait_for_job(&host, CAMEO);

        let claims = host.claims();
        assert_eq!(host.is_any_cut_active(), !claims.is_empty());
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].phase, driver_core::Phase::AwaitingConfirmation);
        assert_eq!(claims[0].job_id, Some(job));
        assert!(!claims[0].starting);
    }

    #[test]
    fn an_idle_host_holds_nothing_and_names_nothing() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        assert!(host.claims().is_empty());
    }

    fn wait_for_ended(host: &Arc<Host>, device: &str, want: driver_core::Ended) -> driver_core::CutStatus {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let status = host.slot(device).expect("known cutter").manager.status();
            if status.ended == Some(want) {
                return status;
            }
            assert!(std::time::Instant::now() < deadline, "{device} never ended {want:?}, sat at {:?}", status.phase);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// A verb the cutter cannot accept right now comes back as the Device error the
    /// manager gave, not as a Preflight refusal — the client renders `actions` and
    /// should be told it asked at the wrong moment.
    #[test]
    fn a_verb_the_cutter_cannot_accept_returns_the_device_error() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        // Idle: nothing to resume.
        match host.resume(CAMEO) {
            Err(Refusal::Device(driver_core::manager::DeviceError::Busy)) => {}
            other => panic!("expected Device(Busy), got {other:?}"),
        }
    }

    /// Every client attached sees every cutter's events over its own subscription.
    #[test]
    fn a_subscriber_sees_events_from_both_cutters_labelled_by_device() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let events = host.subscribe();
        host.dispatch(DispatchId("d-cameo".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        host.dispatch(DispatchId("d-puma".into()), PUMA, "puma", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);
        wait_for(&host, PUMA, driver_core::Phase::AwaitingConfirmation);

        let mut seen = std::collections::HashSet::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while seen.len() < 2 && std::time::Instant::now() < deadline {
            if let Ok(event) = events.recv_timeout(std::time::Duration::from_millis(200)) {
                seen.insert(event.device);
            }
        }
        assert!(seen.contains(CAMEO) && seen.contains(PUMA), "saw only {seen:?}");
    }

    /// A client going away is the normal case, not a fault: its Job carries on and
    /// the host keeps serving whoever is left.
    #[test]
    fn a_dropped_subscriber_does_not_disturb_the_cut() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let staying = host.subscribe();
        drop(host.subscribe()); // a client that closed its laptop

        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);
        assert!(staying.recv_timeout(std::time::Duration::from_secs(5)).is_ok());
    }
}
