// SPDX-License-Identifier: GPL-3.0-or-later

//! A Cut Host: one `DeviceManager` per attached cutter, and the fan-out that lets
//! several clients watch them all over one connection each.
//!
//! Concurrency needs almost nothing here. `DeviceManager::spawn` already gives each
//! cutter its own worker thread, its own cancel flag and its own published
//! `CutStatus`, so a failure on one cutter cannot reach another — that isolation is
//! structural, not implemented.

use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use driver_core::manager::{CutPass, DeviceError, DeviceManager};
use driver_core::{DeviceBackendFactory, DeviceInfo};

use crate::check::{check_passes, PassFault};
use crate::protocol::{DeviceSnapshot, DispatchId, Event, Refusal};

pub(crate) struct DeviceSlot {
    pub info: DeviceInfo,
    pub manager: Arc<DeviceManager>,
    /// The most recent Job this cutter was given, so a reattaching client can tell
    /// whose finished cut it is looking at. `CutStatus` cannot say.
    ///
    /// `None` between `dispatch` returning and `cut()` assigning it — a
    /// snapshot-only client racing that window sees an active cutter with no id.
    pub job_id: Mutex<Option<u64>>,
    /// Dispatch ids already accepted for this cutter. A repeat starts nothing.
    pub dispatches: Mutex<HashSet<DispatchId>>,
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
                eprintln!("cut host: {} did not connect: {e:?}", info.instance_id);
            }
            order.push(info.instance_id.clone());
            pumps.push((info.instance_id.clone(), events));
            slots.insert(
                info.instance_id.clone(),
                DeviceSlot {
                    info,
                    manager: Arc::new(manager),
                    job_id: Mutex::new(None),
                    dispatches: Mutex::new(HashSet::new()),
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
    pub fn snapshots(&self) -> Vec<DeviceSnapshot> {
        self.order
            .iter()
            .filter_map(|id| self.slots.get(id))
            .map(|s| DeviceSnapshot {
                info: s.info.clone(),
                status: s.manager.status(),
                job_id: *s.job_id.lock().unwrap(),
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
    ) -> Result<(), Refusal> {
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

        // One lock acquisition, not two: `insert` reports whether the id was already
        // there, so a duplicate cannot slip through the gap a separate `contains`
        // would leave. That gap is where a client's retry after a dropped reply
        // would become a second cut of the same material. Checked before `actions`
        // so a retry of a Job already mid-cut stays the no-op it always was, rather
        // than being told Busy by the very state its own first dispatch caused.
        if !slot.dispatches.lock().unwrap().insert(dispatch_id.clone()) {
            return Ok(());
        }

        // What is legal now is `actions`' answer, not ours to infer. A cutter that
        // never connected is kept so its snapshot can say so, and accepting a Job
        // for it would burn the dispatch id on work that cannot start.
        if !slot.manager.status().actions.cut {
            slot.dispatches.lock().unwrap().remove(&dispatch_id);
            return Err(Refusal::Device(DeviceError::Busy));
        }

        let host = self.clone();
        let device = device.to_string();
        thread::spawn(move || {
            let Some(slot) = host.slot(&device) else { return };
            match slot.manager.cut(passes) {
                Ok(job_id) => *slot.job_id.lock().unwrap() = Some(job_id),
                Err(e) => {
                    // A refusal before any motion emits no event and moves no state,
                    // so nothing else will tell anyone. Give the id back: a retry
                    // after a Job that never started must be able to run.
                    slot.dispatches.lock().unwrap().remove(&dispatch_id);
                    eprintln!("cut host: {device} refused the job: {e:?}");
                }
            }
        });
        Ok(())
    }

    /// What the daemon's shutdown guard asks, using `driver-core`'s own predicate
    /// rather than a second reading of the phases.
    ///
    /// No caller consults this yet — `cuthulhu-cutd` has no signal handling and no
    /// shutdown path. Wiring one is later work; this is the predicate it will ask.
    pub fn is_any_cut_active(&self) -> bool {
        self.slots.values().any(|s| s.manager.status().is_active())
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
                },
                DeviceInfo {
                    instance_id: PUMA.into(),
                    machine_id: "puma".into(),
                    transport: TransportKind::Serial { path: "/dev/ttyUSB0".into(), baud: 9600 },
                    candidate: true,
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

    /// `dispatch` must return without waiting for the cut. Asserted by time: the
    /// test Driver parks at `AwaitingConfirmation` and stays there until something
    /// confirms, so a `dispatch` that waited for `DeviceManager::cut` would block
    /// here forever.
    #[test]
    fn a_dispatch_returns_before_the_cut_finishes() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let started = std::time::Instant::now();
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "dispatch waited for the cut");
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);
    }

    /// The guard against cutting the same material twice after a dropped reply.
    #[test]
    fn a_repeated_dispatch_id_starts_nothing_further() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);
        let job_after_first = *host.slot(CAMEO).unwrap().job_id.lock().unwrap();

        // A retry arrives. The cutter is mid-Job, so a second cut would be refused
        // Busy anyway — the assertion that matters is that it is accepted as a
        // no-op rather than surfacing an error to a client that did nothing wrong.
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        assert_eq!(*host.slot(CAMEO).unwrap().job_id.lock().unwrap(), job_after_first);
    }

    #[test]
    fn a_dispatch_records_the_job_a_reattaching_client_would_ask_about() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);

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
        }
    }

    #[test]
    fn snapshots_and_devices_agree_on_which_cutters_exist() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let from_devices: Vec<String> = host.devices().into_iter().map(|d| d.instance_id).collect();
        let from_snaps: Vec<String> = host.snapshots().into_iter().map(|s| s.info.instance_id).collect();
        assert_eq!(from_devices, from_snaps);
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
