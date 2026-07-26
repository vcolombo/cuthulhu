// SPDX-License-Identifier: GPL-3.0-or-later
//! The plain (`cuthulhu cut`, no `--by-color`) path, driven against a fake
//! device: the bytes a real machine would receive, and the refusals that stop
//! bytes being produced at all.
use std::sync::{Arc, Mutex};

use cli::cut::{run, Operator};
use cli::pipeline::{plan_plain_cut, Device};
use driver_core::{
    DeviceBackendFactory, DeviceInfo, Driver, DriverError, Job, MachineCaps, MachineProfile,
    MockTransport, Settings, Transport, TransportError, TransportKind,
};

const SQUARE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="20mm">
    <rect width="10" height="10" fill="#ff0000"/></svg>"##;

struct FakeDriver {
    profile: MachineProfile,
}
impl Driver for FakeDriver {
    fn profile(&self) -> &MachineProfile { &self.profile }
    fn caps(&self) -> MachineCaps {
        // Needs an operator to confirm, so the job parks instead of polling.
        MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true }
    }
    fn session_begin(&self) -> Vec<u8> { b"BEGIN".to_vec() }
    fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError> {
        Ok(format!("PASS{}", pass.polylines.len()).into_bytes())
    }
    fn pass_park(&self) -> Vec<u8> { b"PARK".to_vec() }
    fn session_end(&self) -> Vec<u8> { b"END".to_vec() }
    fn abort_bytes(&self) -> Option<Vec<u8>> { None }
}

/// Hands out one transport whose written bytes the test can inspect afterwards.
struct TestFactory {
    written: Arc<Mutex<Vec<u8>>>,
}
impl DeviceBackendFactory for TestFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> { vec![info()] }
    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
        Some(Box::new(FakeDriver {
            profile: MachineProfile {
                id: machine_id.to_string(),
                name: "fake".into(),
                width_mm: 330.0,
                height_mm: 3000.0,
            },
        }))
    }
    fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
        Ok(Box::new(RecordingTransport { inner: MockTransport::default(), sink: self.written.clone() }))
    }
}

struct RecordingTransport {
    inner: MockTransport,
    sink: Arc<Mutex<Vec<u8>>>,
}
impl Transport for RecordingTransport {
    fn write(&mut self, b: &[u8]) -> Result<usize, TransportError> {
        self.sink.lock().unwrap().extend_from_slice(b);
        self.inner.write(b)
    }
    fn read(&mut self, buf: &mut [u8], t: std::time::Duration) -> Result<usize, TransportError> {
        self.inner.read(buf, t)
    }
}

fn info() -> DeviceInfo {
    DeviceInfo {
        instance_id: "test:0".into(),
        machine_id: "cameo5".into(),
        transport: TransportKind::Usb { locator: "0:0".into() },
        candidate: false,
    }
}

#[test]
fn a_plain_cut_sends_one_framed_pass() {
    let plan = plan_plain_cut(SQUARE, Device::Cameo5, &Settings::default(), false).expect("plan");
    assert_eq!(plan.passes.len(), 1, "a plain cut is one pass");

    let written = Arc::new(Mutex::new(Vec::new()));
    let factory = Arc::new(TestFactory { written: written.clone() });
    run(&plan, info(), factory, Operator::Unattended).expect("cut");

    let bytes = String::from_utf8(written.lock().unwrap().clone()).expect("utf8");
    assert!(bytes.starts_with("BEGIN"), "session must open once: {bytes}");
    assert!(bytes.ends_with("END"), "session must close once: {bytes}");
    assert_eq!(bytes.matches("PASS").count(), 1, "exactly one pass: {bytes}");
    assert!(!bytes.contains("PARK"), "no inter-pass park on a single pass: {bytes}");
}

/// Preflight refusals must happen before a transport is ever opened.
#[test]
fn geometry_off_the_bed_never_reaches_a_transport() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10000mm" height="10mm">
        <rect x="9000" width="500" height="5" fill="#000000"/></svg>"##;
    assert!(plan_plain_cut(svg, Device::Cameo5, &Settings::default(), false).is_err());
}
