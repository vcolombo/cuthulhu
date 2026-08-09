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

use driver_core::manager::DeviceManager;
use driver_core::{DeviceBackendFactory, DeviceInfo};

use crate::protocol::{DeviceSnapshot, DispatchId, Event};

pub(crate) struct DeviceSlot {
    pub info: DeviceInfo,
    pub manager: Arc<DeviceManager>,
    /// The most recent Job this cutter was given, so a reattaching client can tell
    /// whose finished cut it is looking at. `CutStatus` cannot say.
    pub job_id: Mutex<Option<u64>>,
    /// Dispatch ids already accepted for this cutter. A repeat starts nothing.
    pub dispatches: Mutex<HashSet<DispatchId>>,
}

pub struct Host {
    /// Insertion order is the enumeration order, kept separately because a
    /// `HashMap` has none and clients render a list.
    order: Vec<String>,
    slots: HashMap<String, DeviceSlot>,
    subscribers: Mutex<Vec<mpsc::Sender<Event>>>,
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
            let host = host.clone();
            thread::spawn(move || {
                // Ends when the manager drops its sender, which happens at shutdown.
                for event in events {
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
        let (tx, rx) = mpsc::channel();
        self.subscribers.lock().unwrap().push(tx);
        rx
    }

    /// What the daemon's shutdown guard asks, using `driver-core`'s own predicate
    /// rather than a second reading of the phases.
    pub fn is_any_cut_active(&self) -> bool {
        self.slots.values().any(|s| s.manager.status().is_active())
    }

    pub(crate) fn slot(&self, device: &str) -> Option<&DeviceSlot> {
        self.slots.get(device)
    }

    /// Drops subscribers whose client has gone. A detached client is the normal
    /// case here, not a fault — the Job it started carries on without it.
    fn broadcast(&self, event: Event) {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|tx| {
            tx.send(Event { device: event.device.clone(), event: event.event.clone() }).is_ok()
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
                "cameo5" => MachineProfile { id: "cameo5".into(), name: "Cameo".into(), width_mm: 300.0, height_mm: 300.0 },
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
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use std::sync::Arc;

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
}
