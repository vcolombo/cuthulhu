// SPDX-License-Identifier: GPL-3.0-or-later
//! Device lifecycle: a worker thread owns the transport/driver and a bounded
//! command channel serializes access. `cut` drives a job through session
//! framing (`session_begin` once, per-pass `encode_pass`/`pass_park`,
//! `session_end` once) and a per-pass completion policy; `resume` and
//! `confirm_pass_done` continue a job parked mid-flight. `cancel` is still a
//! no-op lifecycle stub here — completed by Task 8.

use crate::{write_all, DeviceBackendFactory, DeviceInfo, Driver, Job, Transport, TransportError};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

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
    Cut { passes: Vec<CutPass>, reply: mpsc::Sender<Result<u64, DeviceError>> },
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

    /// Submit a job (one `CutPass` per color/pass). Returns the assigned
    /// `job_id` once the worker has driven the job up to its first pause
    /// point (`WaitingForColorSwap`/`AwaitingCompletion`) or completion.
    /// `Busy` if a job is already active or the device isn't `Idle`.
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

    /// Continue a job parked in `WaitingForColorSwap` after a color swap.
    /// `Busy` outside that state.
    pub fn resume(&self) -> Result<(), DeviceError> {
        self.call(|reply| Command::Resume { reply })?
    }

    /// Acknowledge a pass finished on a machine that can't be polled for
    /// completion (`caps().needs_operator_pass_confirm`). `Busy` outside
    /// `AwaitingCompletion`.
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
    emit_for(NO_JOB, state, new, events);
}

/// Like `emit`, but tags the `StateChanged` event with the job it belongs to
/// instead of `NO_JOB`, so listeners can filter a job's own state transitions.
fn emit_for(job_id: u64, state: &mut DeviceState, new: DeviceState, events: &mpsc::Sender<DeviceEvent>) {
    *state = new.clone();
    // Dropped receiver must not panic the worker.
    let _ = events.send(DeviceEvent { job_id, kind: DeviceEventKind::StateChanged(new) });
}

/// A job parked mid-flight (waiting on a color swap or an operator's pass
/// confirmation) — held by the worker between commands so `resume`/
/// `confirm_pass_done` can continue it without re-entering `Command::Cut`.
struct JobProgress {
    job_id: u64,
    passes: Vec<CutPass>,
    /// Index into `passes` of the pass to run next (`resume`) or the pass
    /// currently awaiting confirmation (`confirm_pass_done`).
    pass_index: usize,
}

enum PassCompletion {
    Ready,
    NeedsConfirm,
}

enum PassRunOutcome {
    /// Parked in `WaitingForColorSwap`; `next_pass_index` is the pass to run on resume.
    Paused { next_pass_index: usize },
    /// Parked in `AwaitingCompletion` for `pass_index`.
    AwaitingConfirm { pass_index: usize },
    /// Job finished; worker is `Idle`.
    Done,
}

const WRITE_CHUNK: usize = 4096;

/// Write `bytes` in `WRITE_CHUNK`-sized pieces, updating `Transmitting` state
/// and emitting a `Progress` event after each chunk actually lands.
fn transmit_bytes(
    transport: &mut dyn Transport,
    bytes: &[u8],
    job_id: u64,
    pass_index: usize,
    state: &mut DeviceState,
    events: &mpsc::Sender<DeviceEvent>,
) -> Result<(), DeviceError> {
    let total_bytes = bytes.len();
    let mut submitted_bytes = 0usize;
    for chunk in bytes.chunks(WRITE_CHUNK) {
        write_all(transport, chunk).map_err(DeviceError::from)?;
        submitted_bytes += chunk.len();
        *state = DeviceState::Transmitting { job_id, pass_index, submitted_bytes, total_bytes };
        let _ = events.send(DeviceEvent {
            job_id,
            kind: DeviceEventKind::Progress { pass_index, submitted_bytes, total_bytes },
        });
    }
    Ok(())
}

/// Completion policy per brief/protocol doc: machines that can report status
/// get polled (`ENQ` + read, 250ms interval, 60s cap); machines that can't
/// (`needs_operator_pass_confirm`) wait for an explicit `confirm_pass_done`.
fn resolve_pass_completion(
    driver: &(dyn Driver + Send),
    transport: &mut dyn Transport,
) -> Result<PassCompletion, DeviceError> {
    if driver.caps().needs_operator_pass_confirm {
        return Ok(PassCompletion::NeedsConfirm);
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        write_all(transport, &[0x05]).map_err(DeviceError::from)?; // ENQ status query
        let mut buf = [0u8; 8];
        match transport.read(&mut buf, Duration::from_millis(250)) {
            Ok(n) if n > 0 && buf[0] == b'0' => return Ok(PassCompletion::Ready),
            Ok(_) => {} // not-ready reply (e.g. still moving); keep polling
            Err(TransportError::Timeout) => {} // no reply within the interval; keep polling
            Err(e) => return Err(DeviceError::from(e)), // hard transport error: fail fast
        }
        if Instant::now() >= deadline {
            return Err(DeviceError::Timeout);
        }
    }
}

/// A pass has finished (confirmed ready, one way or another): emit
/// `PassComplete`, then either park for a color swap or close the session.
fn finish_pass(
    job_id: u64,
    pass_index: usize,
    total_passes: usize,
    driver: &(dyn Driver + Send),
    transport: &mut dyn Transport,
    state: &mut DeviceState,
    events: &mpsc::Sender<DeviceEvent>,
) -> Result<PassRunOutcome, DeviceError> {
    let _ = events.send(DeviceEvent { job_id, kind: DeviceEventKind::PassComplete(pass_index) });
    if pass_index + 1 < total_passes {
        write_all(transport, &driver.pass_park()).map_err(DeviceError::from)?;
        let next_pass_index = pass_index + 1;
        emit_for(job_id, state, DeviceState::WaitingForColorSwap { job_id, next_pass_index }, events);
        Ok(PassRunOutcome::Paused { next_pass_index })
    } else {
        write_all(transport, &driver.session_end()).map_err(DeviceError::from)?;
        let _ = events.send(DeviceEvent { job_id, kind: DeviceEventKind::JobComplete });
        emit_for(job_id, state, DeviceState::Idle, events);
        Ok(PassRunOutcome::Done)
    }
}

/// Run `passes[pass_index]` (prefixing `session_begin` when it's the first
/// pass) through transmit + completion policy, then either park or finish.
fn run_from_pass(
    job_id: u64,
    pass_index: usize,
    passes: &[CutPass],
    driver: &(dyn Driver + Send),
    transport: &mut dyn Transport,
    state: &mut DeviceState,
    events: &mpsc::Sender<DeviceEvent>,
) -> Result<PassRunOutcome, DeviceError> {
    let mut bytes = if pass_index == 0 { driver.session_begin() } else { Vec::new() };
    let pass_bytes = driver
        .encode_pass(&passes[pass_index].job)
        .map_err(|e| DeviceError::Io(format!("{e:?}")))?;
    bytes.extend(pass_bytes);
    transmit_bytes(transport, &bytes, job_id, pass_index, state, events)?;
    match resolve_pass_completion(driver, transport)? {
        PassCompletion::NeedsConfirm => {
            emit_for(job_id, state, DeviceState::AwaitingCompletion { job_id, pass_index }, events);
            Ok(PassRunOutcome::AwaitingConfirm { pass_index })
        }
        PassCompletion::Ready => finish_pass(job_id, pass_index, passes.len(), driver, transport, state, events),
    }
}

/// A pass/job failed: report `Failed` + transition to `Error`, returning the
/// same error so the caller can send it back as the command's reply.
fn fail(job_id: u64, e: DeviceError, state: &mut DeviceState, events: &mpsc::Sender<DeviceEvent>) -> DeviceError {
    let _ = events.send(DeviceEvent { job_id, kind: DeviceEventKind::Failed(e.clone()) });
    emit_for(job_id, state, DeviceState::Error(e.clone()), events);
    e
}

fn worker_loop(cmd_rx: mpsc::Receiver<Command>, events: mpsc::Sender<DeviceEvent>, factory: Arc<dyn DeviceBackendFactory>) {
    let mut state = DeviceState::Disconnected;
    let mut transport: Option<Box<dyn Transport>> = None;
    let mut driver: Option<Box<dyn Driver + Send>> = None;
    let mut next_job_id: u64 = 1; // 0 (NO_JOB) is reserved for lifecycle events
    let mut active_job: Option<JobProgress> = None;

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
                active_job = None; // invariant: active_job.is_some() <=> a job is parked
                emit(&mut state, DeviceState::Disconnected, &events);
                let _ = reply.send(Ok(()));
            }
            Command::Cut { passes, reply } => {
                if !matches!(state, DeviceState::Idle) {
                    let _ = reply.send(Err(DeviceError::Busy));
                    continue;
                }
                if passes.is_empty() {
                    let _ = reply.send(Err(DeviceError::Io("cut: no passes".into())));
                    continue;
                }
                let job_id = next_job_id;
                next_job_id += 1;
                // Idle implies a successful prior connect, so both are Some.
                let drv = driver.as_deref().expect("driver present while Idle");
                let tr = transport.as_deref_mut().expect("transport present while Idle");
                match run_from_pass(job_id, 0, &passes, drv, tr, &mut state, &events) {
                    Ok(PassRunOutcome::Paused { next_pass_index }) => {
                        active_job = Some(JobProgress { job_id, passes, pass_index: next_pass_index });
                        let _ = reply.send(Ok(job_id));
                    }
                    Ok(PassRunOutcome::AwaitingConfirm { pass_index }) => {
                        active_job = Some(JobProgress { job_id, passes, pass_index });
                        let _ = reply.send(Ok(job_id));
                    }
                    Ok(PassRunOutcome::Done) => {
                        let _ = reply.send(Ok(job_id));
                    }
                    Err(e) => {
                        let e = fail(job_id, e, &mut state, &events);
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Command::Cancel => {} // Task 8
            Command::Resume { reply } => match (&state, active_job.take()) {
                (DeviceState::WaitingForColorSwap { .. }, Some(job)) => {
                    let JobProgress { job_id, passes, pass_index } = job;
                    let drv = driver.as_deref().expect("driver present while job active");
                    let tr = transport.as_deref_mut().expect("transport present while job active");
                    match run_from_pass(job_id, pass_index, &passes, drv, tr, &mut state, &events) {
                        Ok(PassRunOutcome::Paused { next_pass_index }) => {
                            active_job = Some(JobProgress { job_id, passes, pass_index: next_pass_index });
                            let _ = reply.send(Ok(()));
                        }
                        Ok(PassRunOutcome::AwaitingConfirm { pass_index }) => {
                            active_job = Some(JobProgress { job_id, passes, pass_index });
                            let _ = reply.send(Ok(()));
                        }
                        Ok(PassRunOutcome::Done) => {
                            let _ = reply.send(Ok(()));
                        }
                        Err(e) => {
                            // pass_index >= 1 here, so session_begin already went out and
                            // there's no abort_bytes write on this path yet — the device is
                            // left mid-session until Task 8 wires cancel/abort handling.
                            let e = fail(job_id, e, &mut state, &events);
                            let _ = reply.send(Err(e));
                        }
                    }
                }
                (_, taken) => {
                    active_job = taken;
                    let _ = reply.send(Err(DeviceError::Busy));
                }
            },
            Command::ConfirmPassDone { reply } => match (&state, active_job.take()) {
                (DeviceState::AwaitingCompletion { .. }, Some(job)) => {
                    let JobProgress { job_id, passes, pass_index } = job;
                    let total_passes = passes.len();
                    let drv = driver.as_deref().expect("driver present while job active");
                    let tr = transport.as_deref_mut().expect("transport present while job active");
                    match finish_pass(job_id, pass_index, total_passes, drv, tr, &mut state, &events) {
                        Ok(PassRunOutcome::Paused { next_pass_index }) => {
                            active_job = Some(JobProgress { job_id, passes, pass_index: next_pass_index });
                            let _ = reply.send(Ok(()));
                        }
                        Ok(PassRunOutcome::Done) => {
                            let _ = reply.send(Ok(()));
                        }
                        Ok(PassRunOutcome::AwaitingConfirm { .. }) => unreachable!("finish_pass never re-parks for confirmation"),
                        Err(e) => {
                            let e = fail(job_id, e, &mut state, &events);
                            let _ = reply.send(Err(e));
                        }
                    }
                }
                (_, taken) => {
                    active_job = taken;
                    let _ = reply.send(Err(DeviceError::Busy));
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DriverError, MachineCaps, MachineProfile, MockTransport, Settings, TransportKind};
    use std::collections::VecDeque;
    use std::sync::Mutex;

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

    fn fake_driver_with_caps(profile: MachineProfile, caps: MachineCaps) -> Box<dyn Driver + Send> {
        Box::new(FakeDriver { profile, caps })
    }

    fn fake_driver() -> Box<dyn Driver + Send> {
        fake_driver_with_caps(
            MachineProfile { id: "cameo5".into(), name: "Cameo 5".into(), width_mm: 305.0, height_mm: 1000.0 },
            MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: false },
        )
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
    fn resume_and_confirm_are_busy_with_no_active_job_and_cancel_is_noop() {
        let (mgr, _events) = DeviceManager::spawn(Arc::new(test_factory()));
        mgr.connect(cameo_info()).unwrap();
        assert_eq!(mgr.cut(Vec::new()).unwrap_err(), DeviceError::Io("cut: no passes".into()));
        assert_eq!(mgr.resume().unwrap_err(), DeviceError::Busy);
        assert_eq!(mgr.confirm_pass_done().unwrap_err(), DeviceError::Busy);
        mgr.cancel(); // still a Task 8 no-op, must not panic or hang
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

    // --- cut-flow test support -------------------------------------------

    /// Tees every write into `mirror` (for asserting on the whole session's
    /// wire bytes) while delegating everything else to a scripted `MockTransport`.
    struct TeeTransport { inner: MockTransport, mirror: Arc<Mutex<Vec<u8>>> }
    impl Transport for TeeTransport {
        fn write(&mut self, b: &[u8]) -> Result<usize, TransportError> {
            let n = self.inner.write(b)?;
            self.mirror.lock().unwrap().extend_from_slice(&b[..n]);
            Ok(n)
        }
        fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError> {
            self.inner.read(buf, timeout)
        }
    }

    /// Cameo-caps factory whose transport scripts `ready_reads` "ready" (`b'0'`)
    /// status replies — one per pass that will get ENQ-polled — and mirrors all
    /// written bytes into the returned `Arc<Mutex<Vec<u8>>>`.
    struct ReadyFactory { written: Arc<Mutex<Vec<u8>>>, ready_reads: usize }
    impl DeviceBackendFactory for ReadyFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> { vec![cameo_info()] }
        fn driver_for(&self, _machine_id: &str) -> Option<Box<dyn Driver + Send>> { Some(fake_driver()) }
        fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            let mut reads = VecDeque::new();
            for _ in 0..self.ready_reads {
                reads.push_back(Ok(b"0\x03".to_vec()));
            }
            let inner = MockTransport { reads, ..Default::default() };
            Ok(Box::new(TeeTransport { inner, mirror: self.written.clone() }))
        }
    }
    fn factory_with_ready_reads(n: usize) -> (Arc<ReadyFactory>, Arc<Mutex<Vec<u8>>>) {
        let written = Arc::new(Mutex::new(Vec::new()));
        (Arc::new(ReadyFactory { written: written.clone(), ready_reads: n }), written)
    }

    fn puma_info() -> DeviceInfo {
        DeviceInfo {
            instance_id: "serial:/dev/ttyUSB0".into(),
            machine_id: "puma".into(),
            transport: TransportKind::Serial { path: "/dev/ttyUSB0".into(), baud: 9600 },
            candidate: true,
        }
    }

    /// A machine that needs an operator to confirm each pass by hand — no
    /// status polling. Reads stay empty on purpose: a bug that polled anyway
    /// would hit an immediate `TransportError::Timeout` and surface fast.
    struct PumaFactory;
    impl DeviceBackendFactory for PumaFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> { vec![puma_info()] }
        fn driver_for(&self, _machine_id: &str) -> Option<Box<dyn Driver + Send>> {
            Some(fake_driver_with_caps(
                MachineProfile { id: "puma".into(), name: "Puma".into(), width_mm: 300.0, height_mm: 1000.0 },
                MachineCaps { supports_speed: false, supports_force: false, needs_operator_pass_confirm: true },
            ))
        }
        fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            Ok(Box::new(MockTransport::default()))
        }
    }

    fn empty_job() -> Job { Job { polylines: Vec::new(), settings: Settings::default() } }
    fn one_pass_job() -> Vec<CutPass> { vec![CutPass { job: empty_job() }] }
    fn two_pass_job() -> Vec<CutPass> { vec![CutPass { job: empty_job() }, CutPass { job: empty_job() }] }

    fn wait_for_state(mgr: &DeviceManager, pred: impl Fn(&DeviceState) -> bool) {
        for _ in 0..200 {
            if pred(&mgr.snapshot()) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for expected state; last snapshot: {:?}", mgr.snapshot());
    }

    fn drain(events: &mpsc::Receiver<DeviceEvent>) -> Vec<DeviceEvent> { events.try_iter().collect() }

    fn count_subseq(haystack: &[u8], needle: &[u8]) -> usize {
        if needle.is_empty() || haystack.len() < needle.len() {
            return 0;
        }
        haystack.windows(needle.len()).filter(|w| *w == needle).count()
    }

    #[test]
    fn two_pass_job_frames_session_once_and_pauses_for_swap() {
        let (factory, written) = factory_with_ready_reads(2);
        let (mgr, events) = DeviceManager::spawn(factory);
        mgr.connect(cameo_info()).unwrap();
        drain(&events); // discard connect-time (NO_JOB) events; only the job's own events matter below

        let job_id = mgr.cut(two_pass_job()).unwrap();
        wait_for_state(&mgr, |s| matches!(s, DeviceState::WaitingForColorSwap { .. }));
        mgr.resume().unwrap();
        wait_for_state(&mgr, |s| matches!(s, DeviceState::Idle));

        let evs = drain(&events);
        assert!(!evs.is_empty());
        assert!(evs.iter().all(|e| e.job_id == job_id));
        assert_eq!(evs.iter().filter(|e| matches!(e.kind, DeviceEventKind::PassComplete(_))).count(), 2);
        assert!(evs.iter().any(|e| matches!(e.kind, DeviceEventKind::JobComplete)));

        let written = written.lock().unwrap();
        assert_eq!(count_subseq(&written, &[0x1b, 0x04]), 1, "one prologue for the whole job");
        assert_eq!(count_subseq(&written, b"SO0"), 1, "one epilogue for the whole job");
        mgr.shutdown();
    }

    #[test]
    fn resume_outside_swap_state_is_busy_and_cut_while_active_is_busy() {
        let (factory, _written) = factory_with_ready_reads(0);
        let (mgr, _events) = DeviceManager::spawn(factory);
        mgr.connect(cameo_info()).unwrap();
        assert_eq!(mgr.resume().unwrap_err(), DeviceError::Busy); // Idle: no active job to resume

        let (factory2, _written2) = factory_with_ready_reads(1);
        let (mgr2, _events2) = DeviceManager::spawn(factory2);
        mgr2.connect(cameo_info()).unwrap();
        mgr2.cut(two_pass_job()).unwrap(); // pauses in WaitingForColorSwap after pass 1
        wait_for_state(&mgr2, |s| matches!(s, DeviceState::WaitingForColorSwap { .. }));
        assert_eq!(mgr2.cut(one_pass_job()).unwrap_err(), DeviceError::Busy); // job still active

        mgr.shutdown();
        mgr2.shutdown();
    }

    #[test]
    fn operator_confirm_path_for_puma_caps() {
        let (mgr, events) = DeviceManager::spawn(Arc::new(PumaFactory));
        mgr.connect(puma_info()).unwrap();
        let job_id = mgr.cut(one_pass_job()).unwrap();
        wait_for_state(&mgr, |s| matches!(s, DeviceState::AwaitingCompletion { .. }));
        mgr.confirm_pass_done().unwrap();
        wait_for_state(&mgr, |s| matches!(s, DeviceState::Idle));

        let evs = drain(&events);
        assert!(evs.iter().any(|e| e.job_id == job_id && matches!(e.kind, DeviceEventKind::JobComplete)));
        mgr.shutdown();
    }

    #[test]
    fn stale_job_events_are_distinguishable() {
        let (factory, _written) = factory_with_ready_reads(2);
        let (mgr, events) = DeviceManager::spawn(factory);
        mgr.connect(cameo_info()).unwrap();

        let job1 = mgr.cut(one_pass_job()).unwrap();
        wait_for_state(&mgr, |s| matches!(s, DeviceState::Idle));
        let job2 = mgr.cut(one_pass_job()).unwrap();
        wait_for_state(&mgr, |s| matches!(s, DeviceState::Idle));

        assert_ne!(job1, job2);
        let evs = drain(&events);
        assert!(evs.iter().any(|e| e.job_id == job1 && matches!(e.kind, DeviceEventKind::JobComplete)));
        assert!(evs.iter().any(|e| e.job_id == job2 && matches!(e.kind, DeviceEventKind::JobComplete)));
        mgr.shutdown();
    }
}
