// SPDX-License-Identifier: GPL-3.0-or-later
//! The cut loop driven against a fake device: the bytes a real machine would
//! receive on the plain (`cuthulhu cut`, no `--by-color`) path, the refusals that
//! stop bytes being produced at all, and how the loop reports the way a job ended.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use cli::cut::{ended_message, run, Operator, Outcome};
use cli::pipeline::{plan_cut_from_svg, plan_plain_cut, Device};
use driver_core::{
    DeviceBackendFactory, DeviceInfo, Driver, DriverError, Job, MachineCaps, MachineProfile,
    MockTransport, Settings, Transport, TransportError, TransportKind,
};

const SQUARE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="20mm">
    <rect width="10" height="10" fill="#ff0000"/></svg>"##;

/// Two stroke colours, so `--by-color` plans two passes and there is a second pass
/// to be cancelled during.
const TWO_COLORS: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="20mm">
    <rect width="5" height="5" fill="none" stroke="#ff0000"/>
    <rect x="6" width="5" height="5" fill="none" stroke="#0000ff"/></svg>"##;

/// Pass padding big enough to need several chunk writes, so a cancel can land
/// between two of them. How large a chunk the manager writes is its own business;
/// a test only needs a pass that outgrows one.
const PAD: usize = 64 * 1024;

/// How long a stalled `encode_pass` holds the worker. Only
/// `a_cancel_while_parked_for_confirmation_is_reported_as_a_cancel` uses it, and
/// only to keep the worker inside `Command::Cancel` for longer than the cut loop
/// takes to read one status and send one command. Overshooting costs that test
/// this much wall clock; undershooting only makes it a weaker test, never a
/// failing one, since every interleaving reports the same cancelled outcome.
const CANCEL_STALL: std::time::Duration = std::time::Duration::from_millis(250);

struct FakeDriver {
    profile: MachineProfile,
    pad: usize,
    /// Set once a cancel has been requested; see `CANCEL_STALL`.
    stall_encode: Option<Arc<AtomicBool>>,
}
impl Driver for FakeDriver {
    fn profile(&self) -> &MachineProfile { &self.profile }
    fn caps(&self) -> MachineCaps {
        // Needs an operator to confirm, so the job parks instead of polling.
        MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true }
    }
    fn session_begin(&self) -> Vec<u8> { b"BEGIN".to_vec() }
    fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError> {
        // Cancelling a pass parked for confirmation re-encodes it here to report how
        // many bytes went out, and does so before publishing anything. Stalling here
        // therefore holds the worker with the *pre-cancel* status still published,
        // which is the only window in which the cut loop can read a parked phase
        // that a queued cancel is about to invalidate.
        if self.stall_encode.as_ref().is_some_and(|c| c.load(Ordering::SeqCst)) {
            std::thread::sleep(CANCEL_STALL);
        }
        let mut bytes = format!("PASS{}", pass.polylines.len()).into_bytes();
        bytes.resize(bytes.len() + self.pad, b'X');
        Ok(bytes)
    }
    fn pass_park(&self) -> Vec<u8> { b"PARK".to_vec() }
    fn session_end(&self) -> Vec<u8> { b"END".to_vec() }
    fn abort_bytes(&self) -> Option<Vec<u8>> { None }
}

fn fake_profile(machine_id: &str) -> MachineProfile {
    MachineProfile { id: machine_id.to_string(), name: "fake".into(), width_mm: 330.0, height_mm: 3000.0 }
}

/// Hands out one transport whose written bytes the test can inspect afterwards.
struct TestFactory {
    written: Arc<Mutex<Vec<u8>>>,
}
impl DeviceBackendFactory for TestFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> { vec![info()] }
    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
        Some(Box::new(FakeDriver { profile: fake_profile(machine_id), pad: 0, stall_encode: None }))
    }
    fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
        Ok(Box::new(RecordingTransport { inner: MockTransport::default(), sink: self.written.clone() }))
    }
}

/// Hands out a transport that parks the worker inside the first write of the second
/// pass and signals the test, so a cancel can land on a job that is genuinely
/// mid-flight instead of racing a job that has already finished.
struct GateFactory {
    gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}
impl DeviceBackendFactory for GateFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> { vec![info()] }
    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
        Some(Box::new(FakeDriver { profile: fake_profile(machine_id), pad: PAD, stall_encode: None }))
    }
    fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
        Ok(Box::new(GateTransport {
            inner: MockTransport::default(),
            seen_park: false,
            sync: self.gate.lock().unwrap().take(),
        }))
    }
}

/// Hands out an unpadded pass — one chunk, so its first write is also its last —
/// gated on that write, plus a driver that stalls the cancel the test lands there.
/// Together they park a pass for confirmation with a cancel already queued behind
/// it, which is what a Ctrl-C during a scripted cut does.
struct ParkedCancelFactory {
    gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
    cancelled: Arc<AtomicBool>,
}
impl DeviceBackendFactory for ParkedCancelFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> { vec![info()] }
    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
        Some(Box::new(FakeDriver {
            profile: fake_profile(machine_id),
            pad: 0,
            stall_encode: Some(self.cancelled.clone()),
        }))
    }
    fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
        Ok(Box::new(GateTransport {
            inner: MockTransport::default(),
            // Already "past a park", so the gate arms on the very first write: the
            // one chunk of pass 1. A cancel landing inside it is too late for
            // `transmit_bytes`'s per-chunk flag check to see.
            seen_park: true,
            sync: self.gate.lock().unwrap().take(),
        }))
    }
}

struct GateTransport {
    inner: MockTransport,
    seen_park: bool,
    sync: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
}
impl Transport for GateTransport {
    fn write(&mut self, b: &[u8]) -> Result<usize, TransportError> {
        // The park bytes close pass 1, so the write after them is pass 2 in flight.
        if self.seen_park {
            if let Some((ready, proceed)) = self.sync.take() {
                let _ = ready.send(());
                let _ = proceed.recv(); // held until the test has cancelled
            }
        }
        self.seen_park |= b.ends_with(b"PARK");
        self.inner.write(b)
    }
    fn read(&mut self, buf: &mut [u8], t: std::time::Duration) -> Result<usize, TransportError> {
        self.inner.read(buf, t)
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
    let outcome = run(&plan, info(), factory, Operator::Unattended, |_| Ok(())).expect("cut");

    assert!(matches!(outcome, Outcome::Completed { passes: 1 }), "{outcome:?}");
    assert_eq!(ended_message(&outcome), "done: 1 passes cut");

    let bytes = String::from_utf8(written.lock().unwrap().clone()).expect("utf8");
    assert!(bytes.starts_with("BEGIN"), "session must open once: {bytes}");
    assert!(bytes.ends_with("END"), "session must close once: {bytes}");
    assert_eq!(bytes.matches("PASS").count(), 1, "exactly one pass: {bytes}");
    assert!(!bytes.contains("PARK"), "no inter-pass park on a single pass: {bytes}");
}

/// The ending a caller cannot work out for itself: a job cancelled part way through
/// rests exactly where a finished one does, so a loop that reads only the phase tells
/// an operator who cancelled that every pass was cut.
///
/// Task 14 could not write this test — the wording left via `println!`, and `run`
/// installed a process-wide Ctrl-C handler that no test binary can install twice.
#[test]
fn a_cancel_part_way_through_is_not_reported_as_a_finished_cut() {
    let plan = plan_cut_from_svg(TWO_COLORS, Device::Cameo5, &Settings::default(), &[], None, false).expect("plan");
    assert_eq!(plan.passes.len(), 2, "two stroke colours, two passes");

    let (ready_tx, ready_rx) = mpsc::channel();
    let (proceed_tx, proceed_rx) = mpsc::channel();
    let factory = Arc::new(GateFactory { gate: Mutex::new(Some((ready_tx, proceed_rx))) });

    // `run` still owns its manager; the device it hands out here is the only way to
    // stop the cut from outside — in `main` this becomes the Ctrl-C handler.
    let outcome = run(&plan, info(), factory, Operator::Unattended, |mgr| {
        std::thread::spawn(move || {
            ready_rx.recv().expect("the second pass reached the wire");
            mgr.cancel();
            proceed_tx.send(()).expect("release the parked write");
        });
        Ok(())
    })
    .expect("a cancelled cut is not an error");

    let Outcome::Cancelled { pass, sent } = outcome else { panic!("reported {outcome:?}, not a cancel") };
    assert_eq!(pass, 1, "cancelled during the second pass");
    assert!(sent > 0 && sent < PAD, "stopped part way through that pass, at {sent} bytes");
    assert_eq!(ended_message(&outcome), format!("cancelled at pass {pass} ({sent} bytes sent)"));
}

/// The ending a scripted cut got wrong: `Operator::Unattended` never waits, so it
/// answers a pass parked for confirmation the instant it sees one — and a Ctrl-C
/// that landed a moment earlier has already taken the job, leaving the worker to
/// refuse that answer as `Busy`. The state moving under the loop is not a device
/// fault, and reporting it as one exits 1 on the case
/// `apps/desktop/MANUAL-CHECKLIST.md` asks a human to test: a scripted cut against
/// a Puma, cancelled. The interactive path never had this, because
/// `wait_for_enter_or_cancel` stops waiting once the job reports it was cancelled.
#[test]
fn a_cancel_while_parked_for_confirmation_is_reported_as_a_cancel() {
    let plan = plan_cut_from_svg(TWO_COLORS, Device::Cameo5, &Settings::default(), &[], None, false).expect("plan");
    assert_eq!(plan.passes.len(), 2, "a pass parked for confirmation needs a pass after it");

    let (ready_tx, ready_rx) = mpsc::channel();
    let (proceed_tx, proceed_rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let factory = Arc::new(ParkedCancelFactory {
        gate: Mutex::new(Some((ready_tx, proceed_rx))),
        cancelled: cancelled.clone(),
    });

    let outcome = run(&plan, info(), factory, Operator::Unattended, |mgr| {
        std::thread::spawn(move || {
            ready_rx.recv().expect("the first pass reached the wire");
            mgr.cancel();
            cancelled.store(true, Ordering::SeqCst); // stall the recompute the cancel is about to do
            proceed_tx.send(()).expect("release the gated write");
        });
        Ok(())
    })
    .expect("a cancel while parked is an ending, not a fault");

    let Outcome::Cancelled { pass, sent } = outcome else { panic!("reported {outcome:?}, not a cancel") };
    assert_eq!(pass, 0, "cancelled on the first pass, so the second was never asked for");
    assert!(sent > 0, "that pass had already gone out in full when the cancel landed");
    assert_eq!(ended_message(&outcome), format!("cancelled at pass {pass} ({sent} bytes sent)"));
}

/// Preflight refusals must happen before a transport is ever opened.
#[test]
fn geometry_off_the_bed_never_reaches_a_transport() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10000mm" height="10mm">
        <rect x="9000" width="500" height="5" fill="#000000"/></svg>"##;
    assert!(plan_plain_cut(svg, Device::Cameo5, &Settings::default(), false).is_err());
}
