// SPDX-License-Identifier: GPL-3.0-or-later
//! Device lifecycle: a worker thread owns the transport/driver and a bounded
//! command channel serializes access. `cut`/`cancel`/`resume`/`confirm_pass_done`
//! are lifecycle stubs here (`DeviceError::Busy` / no-op) — completed by Tasks 7-8.

use crate::{DeviceBackendFactory, DeviceInfo, Driver, Job, Transport, TransportError};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum DeviceState {
    Disconnected,
    Connecting,
    Idle,
    Transmitting { job_id: u64, pass_index: usize, submitted_bytes: usize, total_bytes: usize },
    AwaitingCompletion { job_id: u64, pass_index: usize },
    WaitingForColorSwap { job_id: u64, next_pass_index: usize },
    CancelRequested { job_id: u64 },
    Stopping { job_id: u64 },
    Cancelled { job_id: u64, pass_index: usize, submitted_bytes: usize, completion_known: bool },
    Disconnecting,
    Error(DeviceError),
}

#[derive(Clone, Debug, PartialEq)]
pub enum DeviceError { Disconnected, Busy, Timeout, WriteZero, Io(String) }

impl From<TransportError> for DeviceError {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::NotFound => DeviceError::Disconnected,
            TransportError::Timeout => DeviceError::Timeout,
            TransportError::WriteZero => DeviceError::WriteZero,
            TransportError::Io(s) => DeviceError::Io(s),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceEvent { pub job_id: u64, pub kind: DeviceEventKind }

#[derive(Debug, Clone)]
pub enum DeviceEventKind {
    StateChanged(DeviceState),
    Progress { pass_index: usize, submitted_bytes: usize, total_bytes: usize },
    PassComplete(usize),
    JobComplete,
    Failed(DeviceError),
}

/// One per color, in configured order (Task 7).
pub struct CutPass { pub job: Job }

/// Lifecycle events (connect/disconnect) aren't scoped to a job; they're
/// reported with this sentinel job_id. Job ids start at 1 — 0 is reserved
/// for lifecycle events, so Task 7's job-id counter must begin at 1.
const NO_JOB: u64 = 0;

enum Command {
    Connect { info: DeviceInfo, reply: mpsc::Sender<Result<(), DeviceError>> },
    Disconnect { reply: mpsc::Sender<Result<(), DeviceError>> },
    Snapshot { reply: mpsc::Sender<DeviceState> },
    Cut { #[allow(dead_code)] passes: Vec<CutPass>, reply: mpsc::Sender<Result<u64, DeviceError>> }, // passes consumed by Task 7
    Cancel,
    Resume { reply: mpsc::Sender<Result<(), DeviceError>> },
    ConfirmPassDone { reply: mpsc::Sender<Result<(), DeviceError>> },
    Shutdown,
}

pub struct DeviceManager {
    cmd_tx: mpsc::SyncSender<Command>,
    handle: thread::JoinHandle<()>,
}

impl DeviceManager {
    pub fn spawn(factory: Arc<dyn DeviceBackendFactory>) -> (DeviceManager, mpsc::Receiver<DeviceEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::sync_channel::<Command>(16);
        let (event_tx, event_rx) = mpsc::channel::<DeviceEvent>();
        let handle = thread::spawn(move || worker_loop(cmd_rx, event_tx, factory));
        (DeviceManager { cmd_tx, handle }, event_rx)
    }

    /// Send a command built from a fresh reply channel and wait for the reply.
    /// A channel failure (worker gone) is reported as `DeviceError::Disconnected`.
    fn call<T>(&self, make: impl FnOnce(mpsc::Sender<T>) -> Command) -> Result<T, DeviceError> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx.send(make(reply)).map_err(|_| DeviceError::Disconnected)?;
        rx.recv().map_err(|_| DeviceError::Disconnected)
    }

    pub fn connect(&self, info: DeviceInfo) -> Result<(), DeviceError> {
        self.call(|reply| Command::Connect { info, reply })?
    }

    pub fn disconnect(&self) -> Result<(), DeviceError> {
        self.call(|reply| Command::Disconnect { reply })?
    }

    pub fn snapshot(&self) -> DeviceState {
        self.call(|reply| Command::Snapshot { reply }).unwrap_or(DeviceState::Disconnected)
    }

    /// Stub: real job submission lands in Task 7.
    pub fn cut(&self, passes: Vec<CutPass>) -> Result<u64, DeviceError> {
        self.call(|reply| Command::Cut { passes, reply })?
    }

    /// Stub: real cancellation lands in Task 8 (a cancel-flag `AtomicBool`
    /// the worker checks between blocking writes). Fire-and-forget: `try_send`
    /// so a full command queue (worker busy writing) can't block the caller —
    /// Task 8's flag is how a busy worker actually observes the cancel.
    pub fn cancel(&self) {
        let _ = self.cmd_tx.try_send(Command::Cancel);
    }

    /// Stub: real resume lands in Task 7.
    pub fn resume(&self) -> Result<(), DeviceError> {
        self.call(|reply| Command::Resume { reply })?
    }

    /// Stub: real operator-confirm handling lands in Task 7.
    pub fn confirm_pass_done(&self) -> Result<(), DeviceError> {
        self.call(|reply| Command::ConfirmPassDone { reply })?
    }

    pub fn shutdown(self) {
        // try_send, not send: a blocking send here could itself hang if the
        // worker is wedged (e.g. blocked in a transport write, Task 7) and
        // the queue is full. Skip the send in that case rather than block —
        // dropping cmd_tx below still wakes a worker parked in cmd_rx.recv().
        let _ = self.cmd_tx.try_send(Command::Shutdown);
        drop(self.cmd_tx);
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = self.handle.join();
            let _ = done_tx.send(());
        });
        if done_rx.recv_timeout(Duration::from_secs(5)).is_err() {
            eprintln!("device manager worker did not shut down within 5s; detaching");
        }
    }
}

fn emit(state: &mut DeviceState, new: DeviceState, events: &mpsc::Sender<DeviceEvent>) {
    *state = new.clone();
    // Dropped receiver must not panic the worker.
    let _ = events.send(DeviceEvent { job_id: NO_JOB, kind: DeviceEventKind::StateChanged(new) });
}

// transport/driver are held across commands but not read yet — Task 7 adds the
// write path (encode_pass/write_all) that consumes them.
#[allow(unused_variables, unused_assignments)]
fn worker_loop(cmd_rx: mpsc::Receiver<Command>, events: mpsc::Sender<DeviceEvent>, factory: Arc<dyn DeviceBackendFactory>) {
    let mut state = DeviceState::Disconnected;
    let mut transport: Option<Box<dyn Transport>> = None;
    let mut driver: Option<Box<dyn Driver + Send>> = None;

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            Command::Shutdown => break,
            Command::Snapshot { reply } => {
                let _ = reply.send(state.clone());
            }
            Command::Connect { info, reply } => {
                if !matches!(state, DeviceState::Disconnected | DeviceState::Error(_)) {
                    let _ = reply.send(Err(DeviceError::Busy));
                    continue;
                }
                emit(&mut state, DeviceState::Connecting, &events);
                let outcome = factory.open_transport(&info).map_err(DeviceError::from).and_then(|t| {
                    factory
                        .driver_for(&info.machine_id)
                        .ok_or_else(|| DeviceError::Io(format!("no driver for machine `{}`", info.machine_id)))
                        .map(|d| (t, d))
                });
                match outcome {
                    Ok((t, d)) => {
                        transport = Some(t);
                        driver = Some(d);
                        emit(&mut state, DeviceState::Idle, &events);
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        emit(&mut state, DeviceState::Error(e.clone()), &events);
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Command::Disconnect { reply } => {
                emit(&mut state, DeviceState::Disconnecting, &events);
                transport = None;
                driver = None;
                emit(&mut state, DeviceState::Disconnected, &events);
                let _ = reply.send(Ok(()));
            }
            Command::Cut { reply, .. } => {
                let _ = reply.send(Err(DeviceError::Busy)); // Task 7
            }
            Command::Cancel => {} // Task 8
            Command::Resume { reply } => {
                let _ = reply.send(Err(DeviceError::Busy)); // Task 7
            }
            Command::ConfirmPassDone { reply } => {
                let _ = reply.send(Err(DeviceError::Busy)); // Task 7
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DriverError, MachineCaps, MachineProfile, MockTransport, TransportKind};

    struct FakeDriver { profile: MachineProfile, caps: MachineCaps }
    impl Driver for FakeDriver {
        fn profile(&self) -> &MachineProfile { &self.profile }
        fn caps(&self) -> MachineCaps { self.caps }
        fn session_begin(&self) -> Vec<u8> { vec![0x1b, 0x04] }
        fn encode_pass(&self, _pass: &Job) -> Result<Vec<u8>, DriverError> { Ok(Vec::new()) }
        fn pass_park(&self) -> Vec<u8> { Vec::new() }
        fn session_end(&self) -> Vec<u8> { b"SO0".to_vec() }
        fn abort_bytes(&self) -> Option<Vec<u8>> { None }
    }

    fn fake_driver() -> Box<dyn Driver + Send> {
        Box::new(FakeDriver {
            profile: MachineProfile { id: "cameo5".into(), name: "Cameo 5".into(), width_mm: 305.0, height_mm: 1000.0 },
            caps: MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: false },
        })
    }

    fn cameo_info() -> DeviceInfo {
        DeviceInfo {
            instance_id: "usb:1:4".into(),
            machine_id: "cameo5".into(),
            transport: TransportKind::Usb { locator: "1:4".into() },
            candidate: false,
        }
    }

    struct TestFactory;
    impl DeviceBackendFactory for TestFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> { vec![cameo_info()] }
        fn driver_for(&self, _machine_id: &str) -> Option<Box<dyn Driver + Send>> { Some(fake_driver()) }
        fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            Ok(Box::new(MockTransport::default()))
        }
    }
    fn test_factory() -> TestFactory { TestFactory }

    /// Fails the first `open_transport` call, succeeds on every call after —
    /// drives the connect-fails-then-reconnect-recovers path.
    struct FlakyOpenFactory { attempts: std::sync::atomic::AtomicUsize }
    impl FlakyOpenFactory {
        fn new() -> Self { FlakyOpenFactory { attempts: std::sync::atomic::AtomicUsize::new(0) } }
    }
    impl DeviceBackendFactory for FlakyOpenFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> { vec![cameo_info()] }
        fn driver_for(&self, _machine_id: &str) -> Option<Box<dyn Driver + Send>> { Some(fake_driver()) }
        fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            use std::sync::atomic::Ordering;
            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(TransportError::NotFound)
            } else {
                Ok(Box::new(MockTransport::default()))
            }
        }
    }

    #[test]
    fn connect_transitions_disconnected_to_idle_and_events_fire() {
        let (mgr, events) = DeviceManager::spawn(Arc::new(test_factory()));
        assert!(matches!(mgr.snapshot(), DeviceState::Disconnected));
        mgr.connect(cameo_info()).unwrap();
        assert!(matches!(mgr.snapshot(), DeviceState::Idle));
        let kinds: Vec<_> = events.try_iter().collect();
        assert!(kinds.iter().any(|e| matches!(e.kind, DeviceEventKind::StateChanged(DeviceState::Connecting))));
        assert!(kinds.iter().any(|e| matches!(e.kind, DeviceEventKind::StateChanged(DeviceState::Idle))));
        mgr.shutdown();
    }

    #[test]
    fn connect_failure_yields_error_state_and_reconnect_recovers() {
        let (mgr, _events) = DeviceManager::spawn(Arc::new(FlakyOpenFactory::new()));
        assert!(mgr.connect(cameo_info()).is_err());
        assert!(matches!(mgr.snapshot(), DeviceState::Error(_)));
        mgr.connect(cameo_info()).unwrap(); // recovery: a later successful connect clears Error
        assert!(matches!(mgr.snapshot(), DeviceState::Idle));
        mgr.shutdown();
    }

    #[test]
    fn double_connect_is_busy_and_shutdown_joins() {
        let (mgr, _e) = DeviceManager::spawn(Arc::new(test_factory()));
        mgr.connect(cameo_info()).unwrap();
        assert_eq!(mgr.connect(cameo_info()).unwrap_err(), DeviceError::Busy);
        mgr.shutdown(); // must return (join), not hang
    }

    #[test]
    fn dropped_event_receiver_does_not_panic_worker() {
        let (mgr, events) = DeviceManager::spawn(Arc::new(test_factory()));
        drop(events);
        mgr.connect(cameo_info()).unwrap();
        assert!(matches!(mgr.snapshot(), DeviceState::Idle));
        mgr.shutdown();
    }

    #[test]
    fn cut_cancel_resume_confirm_are_lifecycle_stubs() {
        let (mgr, _events) = DeviceManager::spawn(Arc::new(test_factory()));
        mgr.connect(cameo_info()).unwrap();
        assert_eq!(mgr.cut(Vec::new()).unwrap_err(), DeviceError::Busy);
        assert_eq!(mgr.resume().unwrap_err(), DeviceError::Busy);
        assert_eq!(mgr.confirm_pass_done().unwrap_err(), DeviceError::Busy);
        mgr.cancel(); // no-op, must not panic or hang
        mgr.shutdown();
    }

    #[test]
    fn disconnect_returns_to_disconnected() {
        let (mgr, _events) = DeviceManager::spawn(Arc::new(test_factory()));
        mgr.connect(cameo_info()).unwrap();
        mgr.disconnect().unwrap();
        assert!(matches!(mgr.snapshot(), DeviceState::Disconnected));
        mgr.shutdown();
    }
}
