// SPDX-License-Identifier: GPL-3.0-or-later
//! Device lifecycle: a worker thread owns the transport/driver and a bounded
//! command channel serializes access. `cut` drives a job through session
//! framing (`session_begin` once, per-pass `encode_pass`/`pass_park`,
//! `session_end` once) and a per-pass completion policy; `resume` and
//! `confirm_pass_done` continue a job parked mid-flight. `cancel` is
//! cooperative/best-effort: a shared flag the worker checks between transmit
//! chunks and ENQ polls (for a busy worker), plus a queued `Command::Cancel`
//! for a worker parked at `recv()` — see `DeviceManager::cancel`.

use crate::{write_all, DeviceBackendFactory, DeviceInfo, Driver, Job, Transport, TransportError};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct DeviceEvent { pub job_id: u64, pub kind: DeviceEventKind }

#[derive(Debug, Clone, Serialize)]
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
    cancel_flag: Arc<AtomicBool>,
}

impl DeviceManager {
    pub fn spawn(factory: Arc<dyn DeviceBackendFactory>) -> (DeviceManager, mpsc::Receiver<DeviceEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::sync_channel::<Command>(16);
        let (event_tx, event_rx) = mpsc::channel::<DeviceEvent>();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let worker_flag = cancel_flag.clone();
        let handle = thread::spawn(move || worker_loop(cmd_rx, event_tx, factory, worker_flag));
        (DeviceManager { cmd_tx, handle, cancel_flag }, event_rx)
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

    /// Cooperative, best-effort cancellation: sets a shared flag the worker
    /// checks between transmit chunks and ENQ-poll iterations (the only way to
    /// interrupt a *busy* worker, since it can't see a queued command until it
    /// returns to `cmd_rx.recv()`), and also queues `Command::Cancel` via
    /// `try_send` for a worker already *parked* in `WaitingForColorSwap`/
    /// `AwaitingCompletion`. Emits `CancelRequested -> Stopping -> Cancelled
    /// { completion_known }`, which then stays the resting/observable state
    /// (`snapshot()` can see it) until the next `cut` transitions back to
    /// `Idle`. `completion_known` is only `true` when an ENQ poll actually
    /// confirmed the device is ready (never for `needs_operator_pass_confirm`
    /// machines). A no-op when no job is active.
    pub fn cancel(&self) {
        // ponytail: a concurrent cancel()/cut() enqueue can race — which job
        // the flag lands on isn't guaranteed by send order alone, but it's
        // safe either way: worst case the *new* job gets cancelled instead
        // of the one the caller meant.
        self.cancel_flag.store(true, Ordering::SeqCst);
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
        self.cancel_flag.store(true, Ordering::SeqCst);
        let _ = self.cmd_tx.try_send(Command::Cancel); // best-effort: unpark a job parked at recv() so it cancels cleanly
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
    Cancelled,
}

enum PassRunOutcome {
    /// Parked in `WaitingForColorSwap`; `next_pass_index` is the pass to run on resume.
    Paused { next_pass_index: usize },
    /// Parked in `AwaitingCompletion` for `pass_index`.
    AwaitingConfirm { pass_index: usize },
    /// Job finished; worker is `Idle`.
    Done,
    /// Cancelled mid-pass, either mid-transmit or mid completion-poll.
    Cancelled { pass_index: usize, submitted_bytes: usize },
}

enum TransmitOutcome {
    Completed,
    Cancelled { submitted_bytes: usize },
}

const WRITE_CHUNK: usize = 4096;

/// Write `bytes` in `WRITE_CHUNK`-sized pieces, updating `Transmitting` state
/// and emitting a `Progress` event after each chunk actually lands. Emits a
/// single `StateChanged(Transmitting)` event up front (not one per chunk) so
/// listeners — the GUI, in particular — see the device enter `Transmitting`
/// and can offer a cancel control for the whole pass. Checks `cancel_flag`
/// before each chunk so a cancel mid-transmit stops promptly.
fn transmit_bytes(
    transport: &mut dyn Transport,
    bytes: &[u8],
    job_id: u64,
    pass_index: usize,
    state: &mut DeviceState,
    events: &mpsc::Sender<DeviceEvent>,
    cancel_flag: &AtomicBool,
) -> Result<TransmitOutcome, DeviceError> {
    let total_bytes = bytes.len();
    let mut submitted_bytes = 0usize;
    emit_for(job_id, state, DeviceState::Transmitting { job_id, pass_index, submitted_bytes, total_bytes }, events);
    for chunk in bytes.chunks(WRITE_CHUNK) {
        if cancel_flag.load(Ordering::SeqCst) {
            return Ok(TransmitOutcome::Cancelled { submitted_bytes });
        }
        write_all(transport, chunk).map_err(DeviceError::from)?;
        submitted_bytes += chunk.len();
        *state = DeviceState::Transmitting { job_id, pass_index, submitted_bytes, total_bytes };
        let _ = events.send(DeviceEvent {
            job_id,
            kind: DeviceEventKind::Progress { pass_index, submitted_bytes, total_bytes },
        });
    }
    Ok(TransmitOutcome::Completed)
}

/// Completion policy per brief/protocol doc: machines that can report status
/// get polled (`ENQ` + read, 250ms interval, 60s cap); machines that can't
/// (`needs_operator_pass_confirm`) wait for an explicit `confirm_pass_done`.
/// Checks `cancel_flag` at the top of each poll iteration.
fn resolve_pass_completion(
    driver: &(dyn Driver + Send),
    transport: &mut dyn Transport,
    cancel_flag: &AtomicBool,
) -> Result<PassCompletion, DeviceError> {
    if driver.caps().needs_operator_pass_confirm {
        return Ok(PassCompletion::NeedsConfirm);
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    let interval = Duration::from_millis(250);
    loop {
        if cancel_flag.load(Ordering::SeqCst) {
            return Ok(PassCompletion::Cancelled);
        }
        let iter_start = Instant::now();
        write_all(transport, &[0x05]).map_err(DeviceError::from)?; // ENQ status query
        let mut buf = [0u8; 8];
        match transport.read(&mut buf, interval) {
            Ok(n) if n > 0 && buf[0] == b'0' => return Ok(PassCompletion::Ready),
            Ok(_) => {} // not-ready reply (e.g. still moving); keep polling
            Err(TransportError::Timeout) => {} // no reply within the interval; keep polling
            Err(e) => return Err(DeviceError::from(e)), // hard transport error: fail fast
        }
        if Instant::now() >= deadline {
            return Err(DeviceError::Timeout);
        }
        // Pace to the full interval even when the device replies promptly
        // (e.g. "not ready") instead of timing out the read.
        thread::sleep(interval.saturating_sub(iter_start.elapsed()));
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
    cancel_flag: &AtomicBool,
) -> Result<PassRunOutcome, DeviceError> {
    let mut bytes = if pass_index == 0 { driver.session_begin() } else { Vec::new() };
    let pass_bytes = driver
        .encode_pass(&passes[pass_index].job)
        .map_err(|e| DeviceError::Io(format!("{e:?}")))?;
    bytes.extend(pass_bytes);
    match transmit_bytes(transport, &bytes, job_id, pass_index, state, events, cancel_flag)? {
        TransmitOutcome::Cancelled { submitted_bytes } => {
            return Ok(PassRunOutcome::Cancelled { pass_index, submitted_bytes });
        }
        TransmitOutcome::Completed => {}
    }
    match resolve_pass_completion(driver, transport, cancel_flag)? {
        PassCompletion::Cancelled => Ok(PassRunOutcome::Cancelled { pass_index, submitted_bytes: bytes.len() }),
        PassCompletion::NeedsConfirm => {
            emit_for(job_id, state, DeviceState::AwaitingCompletion { job_id, pass_index }, events);
            Ok(PassRunOutcome::AwaitingConfirm { pass_index })
        }
        PassCompletion::Ready => finish_pass(job_id, pass_index, passes.len(), driver, transport, state, events),
    }
}

/// Recompute the byte length of an already-fully-transmitted pass, mirroring
/// `run_from_pass`'s own encode step — used by `Command::Cancel` to report
/// `submitted_bytes` for a job parked in `AwaitingCompletion`. Errors fall
/// back to 0: encoding already succeeded once to get here.
fn pass_byte_len(driver: &(dyn Driver + Send), passes: &[CutPass], pass_index: usize) -> usize {
    let mut bytes = if pass_index == 0 { driver.session_begin() } else { Vec::new() };
    if let Ok(pass_bytes) = driver.encode_pass(&passes[pass_index].job) {
        bytes.extend(pass_bytes);
    }
    bytes.len()
}

/// Run once cancellation has been observed (either the worker noticed the
/// flag mid-transmit/mid-poll, or a `Command::Cancel` arrived while parked):
/// emit `CancelRequested` and `Stopping`, best-effort abort the device
/// (`abort_bytes`, failure here doesn't block cancellation), resolve whether
/// the device's readiness is actually known, then emit `Cancelled` and leave
/// it as the resting state — `Cancelled` is what `snapshot()`/the next
/// `Command::Cut` sees until a fresh job starts and transitions to `Idle`.
fn perform_cancel(
    job_id: u64,
    pass_index: usize,
    submitted_bytes: usize,
    driver: &(dyn Driver + Send),
    transport: &mut dyn Transport,
    state: &mut DeviceState,
    events: &mpsc::Sender<DeviceEvent>,
    cancel_flag: &AtomicBool,
) {
    emit_for(job_id, state, DeviceState::CancelRequested { job_id }, events);
    emit_for(job_id, state, DeviceState::Stopping { job_id }, events);
    if let Some(abort) = driver.abort_bytes() {
        let _ = write_all(transport, &abort); // best-effort: failure here doesn't block cancellation
    }
    let completion_known = cancel_completion_known(driver, transport);
    emit_for(job_id, state, DeviceState::Cancelled { job_id, pass_index, submitted_bytes, completion_known }, events);
    // Cancelled stays the resting state (no auto-Idle) so a snapshot/event
    // drain can actually observe it; Command::Cut clears it back to Idle.
    cancel_flag.store(false, Ordering::SeqCst); // consumed: don't poison the next job
}

/// After a cancel, best-effort check whether the device is actually ready
/// (confirmed via a short bounded ENQ poll, distinct from the full 60s
/// pass-completion budget) — never true for `needs_operator_pass_confirm`
/// machines, which can't be polled at all. Paced like `resolve_pass_completion`
/// so a device that replies "not ready" promptly still gets real wall-clock
/// time between polls to actually become ready.
fn cancel_completion_known(driver: &(dyn Driver + Send), transport: &mut dyn Transport) -> bool {
    if driver.caps().needs_operator_pass_confirm {
        return false;
    }
    const ATTEMPTS: u8 = 3;
    let interval = Duration::from_millis(250);
    for _ in 0..ATTEMPTS {
        let iter_start = Instant::now();
        if write_all(transport, &[0x05]).is_err() {
            return false;
        }
        let mut buf = [0u8; 8];
        if let Ok(n) = transport.read(&mut buf, interval) {
            if n > 0 && buf[0] == b'0' {
                return true;
            }
        }
        thread::sleep(interval.saturating_sub(iter_start.elapsed()));
    }
    false
}

/// A pass/job failed: report `Failed` + transition to `Error`, returning the
/// same error so the caller can send it back as the command's reply.
fn fail(job_id: u64, e: DeviceError, state: &mut DeviceState, events: &mpsc::Sender<DeviceEvent>) -> DeviceError {
    let _ = events.send(DeviceEvent { job_id, kind: DeviceEventKind::Failed(e.clone()) });
    emit_for(job_id, state, DeviceState::Error(e.clone()), events);
    e
}

fn worker_loop(
    cmd_rx: mpsc::Receiver<Command>,
    events: mpsc::Sender<DeviceEvent>,
    factory: Arc<dyn DeviceBackendFactory>,
    cancel_flag: Arc<AtomicBool>,
) {
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
                // Idle is the normal rest state; Cancelled is also a valid
                // start state so a job can be resubmitted with no manual
                // reset — see perform_cancel's doc comment.
                if !matches!(state, DeviceState::Idle | DeviceState::Cancelled { .. }) {
                    let _ = reply.send(Err(DeviceError::Busy));
                    continue;
                }
                if passes.is_empty() {
                    let _ = reply.send(Err(DeviceError::Io("cut: no passes".into())));
                    continue;
                }
                if matches!(state, DeviceState::Cancelled { .. }) {
                    emit(&mut state, DeviceState::Idle, &events); // leave the cancelled rest state behind
                }
                let job_id = next_job_id;
                next_job_id += 1;
                cancel_flag.store(false, Ordering::SeqCst); // fresh job: clear any stale cancel from a prior one
                // Idle/Cancelled both imply a successful prior connect, so both are Some.
                let drv = driver.as_deref().expect("driver present while Idle");
                let tr = transport.as_deref_mut().expect("transport present while Idle");
                match run_from_pass(job_id, 0, &passes, drv, tr, &mut state, &events, &cancel_flag) {
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
                    Ok(PassRunOutcome::Cancelled { pass_index, submitted_bytes }) => {
                        perform_cancel(job_id, pass_index, submitted_bytes, drv, tr, &mut state, &events, &cancel_flag);
                        let _ = reply.send(Ok(job_id));
                    }
                    Err(e) => {
                        let e = fail(job_id, e, &mut state, &events);
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Command::Cancel => {
                if let Some(job) = active_job.take() {
                    let JobProgress { job_id, passes, pass_index } = job;
                    let drv = driver.as_deref().expect("driver present while job active");
                    let tr = transport.as_deref_mut().expect("transport present while job active");
                    // submitted_bytes: AwaitingCompletion means this pass's transmit
                    // already fully completed (same physical situation as the
                    // mid-poll Cancelled arm in run_from_pass, so recompute the same
                    // way it does). WaitingForColorSwap means pass_index is the
                    // *next*, not-yet-started pass, so 0 is the true count there,
                    // not a placeholder.
                    let submitted_bytes = if matches!(state, DeviceState::AwaitingCompletion { .. }) {
                        pass_byte_len(drv, &passes, pass_index)
                    } else {
                        0
                    };
                    perform_cancel(job_id, pass_index, submitted_bytes, drv, tr, &mut state, &events, &cancel_flag);
                }
                // else: nothing active, safe no-op.
            }
            Command::Resume { reply } => match (&state, active_job.take()) {
                (DeviceState::WaitingForColorSwap { .. }, Some(job)) => {
                    let JobProgress { job_id, passes, pass_index } = job;
                    let drv = driver.as_deref().expect("driver present while job active");
                    let tr = transport.as_deref_mut().expect("transport present while job active");
                    match run_from_pass(job_id, pass_index, &passes, drv, tr, &mut state, &events, &cancel_flag) {
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
                        Ok(PassRunOutcome::Cancelled { pass_index, submitted_bytes }) => {
                            perform_cancel(job_id, pass_index, submitted_bytes, drv, tr, &mut state, &events, &cancel_flag);
                            let _ = reply.send(Ok(()));
                        }
                        Err(e) => {
                            // pass_index >= 1 here, so session_begin already went out.
                            // This is a transport failure, not a cancel, so we
                            // deliberately skip the best-effort abort_bytes write
                            // (that belongs to perform_cancel's cancel path) — a
                            // write that just failed is unlikely to accept an abort.
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
                        Ok(PassRunOutcome::Cancelled { .. }) => unreachable!("finish_pass never cancels"),
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

    struct FakeDriver {
        profile: MachineProfile,
        caps: MachineCaps,
        abort: Option<Vec<u8>>,
        payload_len: usize,
        park_bytes: Vec<u8>,
    }
    impl Driver for FakeDriver {
        fn profile(&self) -> &MachineProfile { &self.profile }
        fn caps(&self) -> MachineCaps { self.caps }
        fn session_begin(&self) -> Vec<u8> { vec![0x1b, 0x04] }
        fn encode_pass(&self, _pass: &Job) -> Result<Vec<u8>, DriverError> { Ok(vec![0xAA; self.payload_len]) }
        fn pass_park(&self) -> Vec<u8> { self.park_bytes.clone() }
        fn session_end(&self) -> Vec<u8> { b"SO0".to_vec() }
        fn abort_bytes(&self) -> Option<Vec<u8>> { self.abort.clone() }
    }

    fn fake_driver_with_caps(profile: MachineProfile, caps: MachineCaps) -> Box<dyn Driver + Send> {
        fake_driver_custom(profile, caps, None, 0, Vec::new())
    }

    fn fake_driver_custom(
        profile: MachineProfile,
        caps: MachineCaps,
        abort: Option<Vec<u8>>,
        payload_len: usize,
        park_bytes: Vec<u8>,
    ) -> Box<dyn Driver + Send> {
        Box::new(FakeDriver { profile, caps, abort, payload_len, park_bytes })
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
        mgr.cancel(); // no active job: safe no-op, must not panic or hang
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

    // --- cancel + failure-path test support -------------------------------

    /// Blocks the worker inside `write()` exactly once (on the `block_on`th
    /// call, 1-indexed) so a test can deterministically catch it "mid-transmit"
    /// before releasing it — avoids sleep-based timing races.
    struct GateTransport {
        inner: MockTransport,
        mirror: Arc<Mutex<Vec<u8>>>,
        call_index: usize,
        block_on: usize,
        sync: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    }
    impl Transport for GateTransport {
        fn write(&mut self, b: &[u8]) -> Result<usize, TransportError> {
            self.call_index += 1;
            if self.call_index == self.block_on {
                if let Some((ready, proceed)) = self.sync.take() {
                    let _ = ready.send(());
                    let _ = proceed.recv();
                }
            }
            let n = self.inner.write(b)?;
            self.mirror.lock().unwrap().extend_from_slice(&b[..n]);
            Ok(n)
        }
        fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError> {
            self.inner.read(buf, timeout)
        }
    }

    /// Driver + a queue of pre-scripted transports (one per `open_transport`
    /// call) — covers every cut-flow test below, including scenarios needing
    /// a fresh transport after a prior one failed.
    struct ScriptedFactory {
        info: DeviceInfo,
        profile: MachineProfile,
        caps: MachineCaps,
        abort: Option<Vec<u8>>,
        payload_len: usize,
        park_bytes: Vec<u8>,
        transports: Mutex<VecDeque<Box<dyn Transport>>>,
    }
    impl DeviceBackendFactory for ScriptedFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> { vec![self.info.clone()] }
        fn driver_for(&self, _machine_id: &str) -> Option<Box<dyn Driver + Send>> {
            Some(fake_driver_custom(self.profile.clone(), self.caps, self.abort.clone(), self.payload_len, self.park_bytes.clone()))
        }
        fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            self.transports.lock().unwrap().pop_front().ok_or(TransportError::NotFound)
        }
    }

    /// Cuts a large single-pass job on a `GateTransport` gated at the second
    /// chunk write, releases it after calling `cancel()` mid-transmit, then
    /// asserts: no further payload bytes land, exactly one `abort_bytes` write
    /// went out, and the final `Cancelled` event carries `expect_completion_known`.
    fn assert_cancel_mid_transmit(caps: MachineCaps, ready_reads: Vec<Result<Vec<u8>, TransportError>>, expect_completion_known: bool) {
        let mirror = Arc::new(Mutex::new(Vec::new()));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (proceed_tx, proceed_rx) = mpsc::channel();
        let inner = MockTransport { reads: ready_reads.into(), ..Default::default() };
        let gate = GateTransport { inner, mirror: mirror.clone(), call_index: 0, block_on: 2, sync: Some((ready_tx, proceed_rx)) };
        let factory = ScriptedFactory {
            info: cameo_info(),
            profile: MachineProfile { id: "cameo5".into(), name: "Cameo 5".into(), width_mm: 305.0, height_mm: 1000.0 },
            caps,
            abort: Some(b"PU;".to_vec()),
            payload_len: WRITE_CHUNK * 3,
            park_bytes: Vec::new(),
            transports: Mutex::new(VecDeque::from(vec![Box::new(gate) as Box<dyn Transport>])),
        };
        let (mgr, events) = DeviceManager::spawn(Arc::new(factory));
        mgr.connect(cameo_info()).unwrap();
        drain(&events);

        thread::scope(|scope| {
            let cut_thread = scope.spawn(|| mgr.cut(one_pass_job()).unwrap());
            ready_rx.recv().unwrap(); // worker is blocked mid-write on chunk 2
            mgr.cancel();
            proceed_tx.send(()).unwrap();
            let job_id = cut_thread.join().unwrap();

            // Cancelled is now the resting state: wait for it directly, and
            // confirm a live snapshot() actually observes it (not just the
            // event trail) per the review fix.
            wait_for_state(&mgr, |s| matches!(s, DeviceState::Cancelled { .. }));
            assert!(matches!(
                mgr.snapshot(),
                DeviceState::Cancelled { completion_known, .. } if completion_known == expect_completion_known
            ));
            let evs = drain(&events);
            assert!(evs.iter().any(|e| e.job_id == job_id && matches!(e.kind, DeviceEventKind::StateChanged(DeviceState::CancelRequested { .. }))));
            assert!(evs.iter().any(|e| e.job_id == job_id && matches!(e.kind, DeviceEventKind::StateChanged(DeviceState::Stopping { .. }))));
            let cancelled = evs.iter().find_map(|e| match &e.kind {
                DeviceEventKind::StateChanged(DeviceState::Cancelled { completion_known, .. }) if e.job_id == job_id => Some(*completion_known),
                _ => None,
            });
            assert_eq!(cancelled, Some(expect_completion_known));
        });

        let written = mirror.lock().unwrap();
        // Chunk 1 carries the 2-byte session_begin header plus payload, so the
        // exact 0xAA count after 2 full chunks land is 2 * WRITE_CHUNK - 2.
        assert_eq!(written.iter().filter(|&&b| b == 0xAA).count(), 2 * WRITE_CHUNK - 2);
        assert_eq!(count_subseq(&written, b"PU;"), 1, "abort bytes written exactly once");
        mgr.shutdown();
    }

    #[test]
    fn cancel_mid_transmit_stops_writes_sends_abort_and_confirms_stop() {
        let cameo_caps = MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: false };
        assert_cancel_mid_transmit(cameo_caps, vec![Ok(b"0\x03".to_vec())], true);

        let puma_caps = MachineCaps { supports_speed: false, supports_force: false, needs_operator_pass_confirm: true };
        assert_cancel_mid_transmit(puma_caps, Vec::new(), false);
    }

    #[test]
    fn transport_write_error_mid_job_fails_loudly() {
        let cameo_caps = MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: false };
        let write_results = VecDeque::from(vec![Ok(WRITE_CHUNK), Ok(WRITE_CHUNK), Err(TransportError::Io("cable pulled".into()))]);
        let inner = MockTransport { write_results, ..Default::default() };
        let factory = ScriptedFactory {
            info: cameo_info(),
            profile: MachineProfile { id: "cameo5".into(), name: "Cameo 5".into(), width_mm: 305.0, height_mm: 1000.0 },
            caps: cameo_caps,
            abort: None,
            payload_len: WRITE_CHUNK * 3,
            park_bytes: Vec::new(),
            transports: Mutex::new(VecDeque::from(vec![Box::new(inner) as Box<dyn Transport>])),
        };
        let (mgr, events) = DeviceManager::spawn(Arc::new(factory));
        mgr.connect(cameo_info()).unwrap();
        drain(&events);

        let err = mgr.cut(one_pass_job()).unwrap_err();
        assert_eq!(err, DeviceError::Io("cable pulled".into()));
        assert!(matches!(mgr.snapshot(), DeviceState::Error(DeviceError::Io(_))));

        let evs = drain(&events);
        assert!(evs.iter().any(|e| matches!(&e.kind, DeviceEventKind::Failed(DeviceError::Io(_)))));
        assert!(!evs.iter().any(|e| matches!(e.kind, DeviceEventKind::JobComplete)));
        mgr.shutdown();
    }

    #[test]
    fn write_zero_maps_to_typed_error() {
        let cameo_caps = MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: false };
        let write_results = VecDeque::from(vec![Ok(0)]);
        let inner = MockTransport { write_results, ..Default::default() };
        let factory = ScriptedFactory {
            info: cameo_info(),
            profile: MachineProfile { id: "cameo5".into(), name: "Cameo 5".into(), width_mm: 305.0, height_mm: 1000.0 },
            caps: cameo_caps,
            abort: None,
            payload_len: 0,
            park_bytes: Vec::new(),
            transports: Mutex::new(VecDeque::from(vec![Box::new(inner) as Box<dyn Transport>])),
        };
        let (mgr, events) = DeviceManager::spawn(Arc::new(factory));
        mgr.connect(cameo_info()).unwrap();
        drain(&events);

        let err = mgr.cut(one_pass_job()).unwrap_err();
        assert_eq!(err, DeviceError::WriteZero);

        let evs = drain(&events);
        assert!(evs.iter().any(|e| matches!(&e.kind, DeviceEventKind::Failed(DeviceError::WriteZero))));
        mgr.shutdown();
    }

    /// Runs `drive` against a `ScriptedFactory` built from the given script,
    /// asserts it fails with `expect`, that the manager lands in `Error`, and
    /// that `shutdown` still joins cleanly (proving the worker isn't wedged).
    fn assert_unplug_surfaces_typed_error(
        caps: MachineCaps,
        write_results: VecDeque<Result<usize, TransportError>>,
        reads: VecDeque<Result<Vec<u8>, TransportError>>,
        payload_len: usize,
        drive: impl FnOnce(&DeviceManager) -> Result<(), DeviceError>,
        expect: DeviceError,
    ) {
        let inner = MockTransport { write_results, reads, ..Default::default() };
        let factory = ScriptedFactory {
            info: cameo_info(),
            profile: MachineProfile { id: "cameo5".into(), name: "Cameo 5".into(), width_mm: 305.0, height_mm: 1000.0 },
            caps,
            abort: None,
            payload_len,
            park_bytes: Vec::new(),
            transports: Mutex::new(VecDeque::from(vec![Box::new(inner) as Box<dyn Transport>])),
        };
        let (mgr, _events) = DeviceManager::spawn(Arc::new(factory));
        mgr.connect(cameo_info()).unwrap();

        let err = drive(&mgr).unwrap_err();
        assert_eq!(err, expect);
        assert!(matches!(mgr.snapshot(), DeviceState::Error(_)));
        mgr.shutdown();
    }

    #[test]
    fn unplug_during_each_active_state_reports_disconnected() {
        let cameo_caps = MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: false };
        let puma_caps = MachineCaps { supports_speed: false, supports_force: false, needs_operator_pass_confirm: true };

        // Transmitting: the very first write (session_begin) fails.
        assert_unplug_surfaces_typed_error(
            cameo_caps,
            VecDeque::from(vec![Err(TransportError::NotFound)]),
            VecDeque::new(),
            0,
            |mgr| mgr.cut(one_pass_job()).map(|_| ()),
            DeviceError::Disconnected,
        );

        // AwaitingCompletion: session_begin + pass bytes land, then confirm_pass_done's
        // session_end write fails.
        assert_unplug_surfaces_typed_error(
            puma_caps,
            VecDeque::from(vec![Ok(3), Err(TransportError::Io("cable pulled".into()))]),
            VecDeque::new(),
            1,
            |mgr| {
                mgr.cut(one_pass_job())?;
                mgr.confirm_pass_done()
            },
            DeviceError::Io("cable pulled".into()),
        );

        // WaitingForColorSwap: pass 1 completes and parks (empty pass_park is a
        // no-op write), then resume's pass-2 transmit fails.
        assert_unplug_surfaces_typed_error(
            cameo_caps,
            VecDeque::from(vec![Ok(3), Ok(1), Err(TransportError::NotFound)]),
            VecDeque::from(vec![Ok(b"0\x03".to_vec())]),
            1,
            |mgr| {
                mgr.cut(two_pass_job())?;
                mgr.resume()
            },
            DeviceError::Disconnected,
        );
    }

    #[test]
    fn shutdown_mid_job_cancels_and_joins() {
        let (factory, _written) = factory_with_ready_reads(1);
        let (mgr, events) = DeviceManager::spawn(factory);
        mgr.connect(cameo_info()).unwrap();
        let job_id = mgr.cut(two_pass_job()).unwrap();
        wait_for_state(&mgr, |s| matches!(s, DeviceState::WaitingForColorSwap { .. }));
        drain(&events);

        let start = std::time::Instant::now();
        mgr.shutdown();
        assert!(start.elapsed() < Duration::from_secs(2), "shutdown should cancel and join promptly");

        let evs = drain(&events);
        assert!(evs.iter().any(|e| e.job_id == job_id && matches!(e.kind, DeviceEventKind::StateChanged(DeviceState::CancelRequested { .. }))));
        // Cancelled is the resting state post-shutdown (no further Cut arrives
        // to lazily flip it back to Idle), so that's the terminal state here.
        assert!(evs.iter().any(|e| e.job_id == job_id && matches!(e.kind, DeviceEventKind::StateChanged(DeviceState::Cancelled { .. }))));
    }
}
