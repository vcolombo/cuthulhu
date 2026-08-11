// SPDX-License-Identifier: GPL-3.0-or-later
//! Device lifecycle: a worker thread owns the transport/driver and a bounded
//! command channel serializes access. `cut` drives a job through session
//! framing (`session_begin` once, per-pass `encode_pass`/`pass_park`,
//! `session_end` once) and a per-pass completion policy; `resume` and
//! `confirm_pass_done` continue a job parked mid-flight. `cancel` is
//! cooperative/best-effort: a shared flag the worker checks between transmit
//! chunks and ENQ polls (for a busy worker), plus a queued `Command::Cancel`
//! for a worker parked at `recv()` — see `DeviceManager::cancel`.

use crate::status::{status_of, CutStatus, Ended};
use crate::{close_pass, open_pass, write_all, DeviceBackendFactory, DeviceInfo, Driver, Job, Transport, TransportError};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Serialize)]
/// The machine, in full detail. Deliberately crate-private: callers read
/// `CutStatus`, so there is only ever one derivation of phase and legal actions.
pub(crate) enum DeviceState {
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DeviceError { Disconnected, Busy, Timeout, WriteZero, Io(String) }

impl From<TransportError> for DeviceError {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::NotFound | TransportError::Disconnected => DeviceError::Disconnected,
            TransportError::Timeout => DeviceError::Timeout,
            TransportError::WriteZero => DeviceError::WriteZero,
            TransportError::Io(s) => DeviceError::Io(s),
        }
    }
}

/// An event and the status that held when it was sent, so a listener renders from
/// what it just received rather than polling for a value that may already have
/// moved on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceEvent { pub job_id: u64, pub kind: DeviceEventKind, pub status: CutStatus }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeviceEventKind {
    /// Carries no payload: the event's `status` is what changed. A state the
    /// caller cannot act on differently is not a distinction worth publishing.
    StateChanged,
    Progress { pass_index: usize, submitted_bytes: usize, total_bytes: usize },
    PassComplete(usize),
    JobComplete,
    Failed(DeviceError),
}

/// One per color, in configured order (Task 7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CutPass { pub job: Job }

/// Lifecycle events (connect/disconnect) aren't scoped to a job; they're
/// reported with this sentinel job_id. Job ids start at 1 — 0 is reserved
/// for lifecycle events, so Task 7's job-id counter must begin at 1.
const NO_JOB: u64 = 0;

enum Command {
    Connect { info: DeviceInfo, reply: mpsc::Sender<Result<(), DeviceError>> },
    Disconnect { reply: mpsc::Sender<Result<(), DeviceError>> },
    #[cfg(test)]
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
    /// Published by the worker on every state change and progress tick. Read
    /// without touching the command channel, so a caller is never blocked by a
    /// busy worker. It may lag the worker by one event — that is the single
    /// documented lag rule, and every caller shares it.
    status: Arc<Mutex<CutStatus>>,
}

impl DeviceManager {
    pub fn spawn(factory: Arc<dyn DeviceBackendFactory>) -> (DeviceManager, mpsc::Receiver<DeviceEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::sync_channel::<Command>(16);
        let (event_tx, event_rx) = mpsc::channel::<DeviceEvent>();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let worker_flag = cancel_flag.clone();
        let status = Arc::new(Mutex::new(CutStatus::disconnected()));
        let reporter = Reporter { events: event_tx, status: status.clone(), ended: Cell::new(None) };
        let handle = thread::spawn(move || worker_loop(cmd_rx, reporter, factory, worker_flag));
        (DeviceManager { cmd_tx, handle, cancel_flag, status }, event_rx)
    }

    /// Where the cut has got to, and what may be done next. Never blocks: unlike
    /// `snapshot()`, which round-trips through the worker's command channel and so
    /// waits out whatever transport write the worker is inside, this reads the cell
    /// the worker publishes.
    pub fn status(&self) -> CutStatus {
        // A dead worker publishes nothing ever again, so the cell would freeze on
        // whatever it last held — a UI left offering a cancel button for a cut that
        // no longer has a thread behind it. `snapshot()` gets this for free from its
        // channel error; this read path has to ask.
        if self.handle.is_finished() {
            return status_of(&DeviceState::Error(DeviceError::Disconnected), 0, None);
        }
        self.status.lock().unwrap().clone()
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

    /// The raw state behind the reported one. `actions.cut` is derived from
    /// `completion_known`, and a test asserting only the derived value cannot tell a
    /// correct derivation from a lucky one — so the fact itself needs an observation
    /// point. Nothing shipping reads it; it exists for the tests in this file.
    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> DeviceState {
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
    /// until a `cut` or a reconnect transitions back to `Idle`.
    /// `completion_known` is only `true` when an ENQ poll actually confirmed the
    /// device is ready (never for `needs_operator_pass_confirm` machines), and it
    /// is what decides whether `actions.cut` may offer another Job. A no-op when
    /// no job is active.
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

/// The worker's two ways of reporting: the event stream, and the status cell
/// `DeviceManager::status()` reads. One type owns both so no site can send an
/// event without publishing — a site that forgot would leave the published
/// status silently frozen while events kept flowing.
struct Reporter {
    events: mpsc::Sender<DeviceEvent>,
    status: Arc<Mutex<CutStatus>>,
    /// How the last job finished. Kept here rather than threaded through every
    /// emit because this is the only thing that reads it, and because a job that
    /// ran to the end rests on `DeviceState::Idle` — the outcome has nowhere else
    /// to live. Touched only by the worker thread, hence `Cell`.
    ended: Cell<Option<Ended>>,
}

impl Reporter {
    /// A new job, or a lifecycle transition, has nothing to do with how the last
    /// job ended: pass `None` so a fresh cut cannot report the previous outcome.
    fn set_ended(&self, ended: Option<Ended>) {
        self.ended.set(ended);
    }

    /// Publishes the status, then sends the event. That order is what callers
    /// rely on: one woken by an event and calling `status()` cannot then see a
    /// value older than the event that woke it.
    fn emit(&self, state: &DeviceState, total_passes: usize, job_id: u64, kind: DeviceEventKind) {
        let status = status_of(state, total_passes, self.ended.get());
        // The guard dies at the end of this statement, so the lock is not held
        // across the send below — nor, anywhere in this worker, across a
        // transport write or read.
        *self.status.lock().unwrap() = status.clone();
        // Dropped receiver must not panic the worker.
        let _ = self.events.send(DeviceEvent { job_id, kind, status });
    }
}

/// A lifecycle transition (connect/disconnect, or clearing a cancelled rest
/// state) belongs to no job, so it reports no pass count and no outcome — every
/// state reachable this way has no pass position of its own, and a freshly
/// connected device has cut nothing.
fn emit(state: &mut DeviceState, new: DeviceState, rep: &Reporter) {
    rep.set_ended(None);
    emit_for(NO_JOB, state, new, 0, rep);
}

/// Like `emit`, but tags the `StateChanged` event with the job it belongs to
/// instead of `NO_JOB`, so listeners can filter a job's own state transitions.
fn emit_for(job_id: u64, state: &mut DeviceState, new: DeviceState, total_passes: usize, rep: &Reporter) {
    *state = new;
    rep.emit(state, total_passes, job_id, DeviceEventKind::StateChanged);
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
/// single `StateChanged` event up front (not one per chunk) so listeners — the
/// GUI, in particular — see the device enter `Phase::Sending` and can offer a
/// cancel control for the whole pass. Checks `cancel_flag`
/// before each chunk so a cancel mid-transmit stops promptly.
fn transmit_bytes(
    transport: &mut dyn Transport,
    bytes: &[u8],
    job_id: u64,
    pass_index: usize,
    total_passes: usize,
    state: &mut DeviceState,
    rep: &Reporter,
    cancel_flag: &AtomicBool,
) -> Result<TransmitOutcome, DeviceError> {
    let total_bytes = bytes.len();
    let mut submitted_bytes = 0usize;
    let sending = DeviceState::Transmitting { job_id, pass_index, submitted_bytes, total_bytes };
    emit_for(job_id, state, sending, total_passes, rep);
    for chunk in bytes.chunks(WRITE_CHUNK) {
        if cancel_flag.load(Ordering::SeqCst) {
            return Ok(TransmitOutcome::Cancelled { submitted_bytes });
        }
        write_all(transport, chunk).map_err(DeviceError::from)?;
        submitted_bytes += chunk.len();
        *state = DeviceState::Transmitting { job_id, pass_index, submitted_bytes, total_bytes };
        rep.emit(
            state,
            total_passes,
            job_id,
            DeviceEventKind::Progress { pass_index, submitted_bytes, total_bytes },
        );
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
        write_all(transport, &driver.status_query()).map_err(DeviceError::from)?;
        let mut buf = [0u8; 8];
        match transport.read(&mut buf, interval) {
            Ok(n) if n > 0 && buf[0] == b'0' => return Ok(PassCompletion::Ready),
            // Unloaded media can never become ready on its own — fail with a
            // real reason instead of silently polling out the 60s deadline.
            Ok(n) if n > 0 && buf[0] == b'2' => {
                return Err(DeviceError::Io("media unloaded".to_string()))
            }
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

/// How long a candidate device gets to answer the connect-time status query.
// ponytail: one fixed budget for every candidate. A real cutter answers an ENQ in
// milliseconds, so this is generous; a device slower than this reads as "not a cutter" and
// the operator sees a refusal they can retry. Make it per-driver if a real machine ever
// needs longer.
const PROBE_TIMEOUT: Duration = Duration::from_millis(750);

/// Confirms an operator-picked candidate is actually a cutter before reporting it connected.
///
/// Serial ports carry no VID/PID, so a candidate is a guess: any device that accepts bytes
/// can be selected, and connect otherwise "succeeds" against a label printer or a debug
/// console. The failure then surfaces much later, as a cut that transmits in full and times
/// out waiting for a status reply — or, once registration exists, as an optical-registration
/// failure the operator will blame on marks, paper or lighting.
///
/// The status query the completion poll already relies on doubles as the identity check: a
/// cutter answers it with a status char (`0` ready / `1` moving / `2` unloaded), and nothing
/// else is accepted as proof.
fn probe_is_cutter(
    transport: &mut dyn Transport,
    driver: &dyn Driver,
    info: &DeviceInfo,
) -> Result<(), DeviceError> {
    let where_ = &info.instance_id;
    write_all(transport, &driver.status_query()).map_err(DeviceError::from)?;
    let mut buf = [0u8; 8];
    match transport.read(&mut buf, PROBE_TIMEOUT) {
        Ok(n) if n > 0 && (b'0'..=b'2').contains(&buf[0]) => Ok(()),
        Ok(n) if n > 0 => Err(DeviceError::Io(format!(
            "{where_} answered a status query with something other than a status: not a cutter, or not one this driver speaks"
        ))),
        Ok(_) | Err(TransportError::Timeout) => Err(DeviceError::Io(format!(
            "{where_} did not answer a status query, so it cannot be confirmed as a cutter"
        ))),
        Err(e) => Err(DeviceError::from(e)),
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
    rep: &Reporter,
) -> Result<PassRunOutcome, DeviceError> {
    rep.emit(state, total_passes, job_id, DeviceEventKind::PassComplete(pass_index));
    write_all(transport, &close_pass(driver, pass_index, total_passes)).map_err(DeviceError::from)?;
    if pass_index + 1 < total_passes {
        let next_pass_index = pass_index + 1;
        emit_for(job_id, state, DeviceState::WaitingForColorSwap { job_id, next_pass_index }, total_passes, rep);
        Ok(PassRunOutcome::Paused { next_pass_index })
    } else {
        rep.emit(state, total_passes, job_id, DeviceEventKind::JobComplete);
        // Set after `JobComplete` goes out, not before: that event still carries the
        // mid-flight status, and a caller renders it — a "completed" outcome attached
        // to a `Sending` phase would read as a cut both finishing and still running.
        rep.set_ended(Some(Ended::Completed));
        // The job is over, so `Idle` reports no pass count. `total_passes` is a
        // parameter, not worker state, so nothing stale can outlive this call.
        emit_for(job_id, state, DeviceState::Idle, 0, rep);
        Ok(PassRunOutcome::Done)
    }
}

/// Run `passes[pass_index]` through transmit + completion policy, then either park or finish.
fn run_from_pass(
    job_id: u64,
    pass_index: usize,
    passes: &[CutPass],
    driver: &(dyn Driver + Send),
    transport: &mut dyn Transport,
    state: &mut DeviceState,
    rep: &Reporter,
    cancel_flag: &AtomicBool,
) -> Result<PassRunOutcome, DeviceError> {
    let total_passes = passes.len();
    let bytes = open_pass(driver, &passes[pass_index].job, pass_index)
        .map_err(|e| DeviceError::Io(format!("{e:?}")))?;
    match transmit_bytes(transport, &bytes, job_id, pass_index, total_passes, state, rep, cancel_flag)? {
        TransmitOutcome::Cancelled { submitted_bytes } => {
            return Ok(PassRunOutcome::Cancelled { pass_index, submitted_bytes });
        }
        TransmitOutcome::Completed => {}
    }
    match resolve_pass_completion(driver, transport, cancel_flag)? {
        PassCompletion::Cancelled => Ok(PassRunOutcome::Cancelled { pass_index, submitted_bytes: bytes.len() }),
        PassCompletion::NeedsConfirm => {
            let awaiting = DeviceState::AwaitingCompletion { job_id, pass_index };
            emit_for(job_id, state, awaiting, total_passes, rep);
            Ok(PassRunOutcome::AwaitingConfirm { pass_index })
        }
        PassCompletion::Ready => finish_pass(job_id, pass_index, total_passes, driver, transport, state, rep),
    }
}

/// The byte length of an already-fully-transmitted Pass, from the same function
/// that produced those bytes — used by `Command::Cancel` to report
/// `submitted_bytes` for a job parked in `AwaitingCompletion`. Errors fall
/// back to 0: encoding already succeeded once to get here, and a cancel must not fail
/// because a byte count could not be recomputed.
fn pass_byte_len(driver: &(dyn Driver + Send), passes: &[CutPass], pass_index: usize) -> usize {
    open_pass(driver, &passes[pass_index].job, pass_index).map_or(0, |b| b.len())
}

/// Run once cancellation has been observed (either the worker noticed the
/// flag mid-transmit/mid-poll, or a `Command::Cancel` arrived while parked):
/// emit `CancelRequested` and `Stopping`, best-effort abort the device
/// (`abort_bytes`, failure here doesn't block cancellation), resolve whether
/// the device's readiness is actually known, then emit `Cancelled` and leave
/// it as the resting state — `Cancelled` is what `snapshot()`/the next
/// `Command::Cut` sees until a fresh job starts and transitions to `Idle`.
/// Resting on a state of its own is why a cancel needs no remembered outcome:
/// `status_of` reads the ending straight off `Cancelled`.
fn perform_cancel(
    job_id: u64,
    pass_index: usize,
    submitted_bytes: usize,
    total_passes: usize,
    driver: &(dyn Driver + Send),
    transport: &mut dyn Transport,
    state: &mut DeviceState,
    rep: &Reporter,
    cancel_flag: &AtomicBool,
) {
    emit_for(job_id, state, DeviceState::CancelRequested { job_id }, total_passes, rep);
    emit_for(job_id, state, DeviceState::Stopping { job_id }, total_passes, rep);
    if let Some(abort) = driver.abort_bytes() {
        let _ = write_all(transport, &abort); // best-effort: failure here doesn't block cancellation
    }
    let completion_known = cancel_completion_known(driver, transport);
    let cancelled = DeviceState::Cancelled { job_id, pass_index, submitted_bytes, completion_known };
    emit_for(job_id, state, cancelled, total_passes, rep);
    // Cancelled stays the resting state (no auto-Idle) so a snapshot/event
    // drain can actually observe it; Command::Cut clears it back to Idle — but
    // only when `completion_known` said the machine was seen to stop. When it did
    // not, `Command::Disconnect` is the way out, and it is the honest one: it drops
    // the transport an operator then re-opens, having gone and looked at the
    // machine. Nothing here decides on their behalf that the blade must have
    // stopped by now.
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
        if write_all(transport, &driver.status_query()).is_err() {
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
fn fail(
    job_id: u64,
    e: DeviceError,
    total_passes: usize,
    state: &mut DeviceState,
    rep: &Reporter,
) -> DeviceError {
    // `Failed` still reports the state the job died in, so it needs the real pass
    // count; `Error` itself has no pass position.
    rep.emit(state, total_passes, job_id, DeviceEventKind::Failed(e.clone()));
    emit_for(job_id, state, DeviceState::Error(e.clone()), 0, rep);
    e
}

fn worker_loop(
    cmd_rx: mpsc::Receiver<Command>,
    rep: Reporter,
    factory: Arc<dyn DeviceBackendFactory>,
    cancel_flag: Arc<AtomicBool>,
) {
    let rep = &rep;
    let mut state = DeviceState::Disconnected;
    let mut transport: Option<Box<dyn Transport>> = None;
    let mut driver: Option<Box<dyn Driver + Send>> = None;
    let mut next_job_id: u64 = 1; // 0 (NO_JOB) is reserved for lifecycle events
    let mut active_job: Option<JobProgress> = None;

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            Command::Shutdown => break,
            #[cfg(test)]
            Command::Snapshot { reply } => {
                let _ = reply.send(state.clone());
            }
            Command::Connect { info, reply } => {
                if !matches!(state, DeviceState::Disconnected | DeviceState::Error(_)) {
                    let _ = reply.send(Err(DeviceError::Busy));
                    continue;
                }
                emit(&mut state, DeviceState::Connecting, rep);
                let outcome = factory
                    .open_transport(&info)
                    .map_err(DeviceError::from)
                    .and_then(|t| {
                        factory
                            .driver_for(&info.machine_id)
                            .ok_or_else(|| DeviceError::Io(format!("no driver for machine `{}`", info.machine_id)))
                            .map(|d| (t, d))
                    })
                    .and_then(|(mut t, d)| {
                        if info.candidate {
                            probe_is_cutter(t.as_mut(), d.as_ref(), &info)?;
                        }
                        Ok((t, d))
                    });
                match outcome {
                    Ok((t, d)) => {
                        transport = Some(t);
                        driver = Some(d);
                        emit(&mut state, DeviceState::Idle, rep);
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        emit(&mut state, DeviceState::Error(e.clone()), rep);
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Command::Disconnect { reply } => {
                emit(&mut state, DeviceState::Disconnecting, rep);
                transport = None;
                driver = None;
                active_job = None; // invariant: active_job.is_some() <=> a job is parked
                emit(&mut state, DeviceState::Disconnected, rep);
                let _ = reply.send(Ok(()));
            }
            Command::Cut { passes, reply } => {
                // Idle is the normal rest state. A cancel whose stop was actually
                // observed is also a valid start state, so a job can be resubmitted
                // with no manual reset — see perform_cancel's doc comment. One that
                // was not is refused rather than started: `Busy` is literally what it
                // may still be, and the machine is the only thing that can say
                // otherwise. `status_of` withholds `actions.cut` for the same reason,
                // so this guard refuses nothing a caller was told it could do.
                if !matches!(state, DeviceState::Idle | DeviceState::Cancelled { completion_known: true, .. }) {
                    let _ = reply.send(Err(DeviceError::Busy));
                    continue;
                }
                if passes.is_empty() {
                    let _ = reply.send(Err(DeviceError::Io("cut: no passes".into())));
                    continue;
                }
                if matches!(state, DeviceState::Cancelled { .. }) {
                    emit(&mut state, DeviceState::Idle, rep); // leave the cancelled rest state behind
                }
                let job_id = next_job_id;
                next_job_id += 1;
                // The reported pass count is this Vec's length, for this job only —
                // read from the job in hand at every emit, never cached on the worker.
                let total_passes = passes.len();
                cancel_flag.store(false, Ordering::SeqCst); // fresh job: clear any stale cancel from a prior one
                rep.set_ended(None); // and the prior job's ending, or this one reports it as its own
                // Idle/Cancelled both imply a successful prior connect, so both are Some.
                let drv = driver.as_deref().expect("driver present while Idle");
                let tr = transport.as_deref_mut().expect("transport present while Idle");
                match run_from_pass(job_id, 0, &passes, drv, tr, &mut state, rep, &cancel_flag) {
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
                        perform_cancel(job_id, pass_index, submitted_bytes, total_passes, drv, tr, &mut state, rep, &cancel_flag);
                        let _ = reply.send(Ok(job_id));
                    }
                    Err(e) => {
                        let e = fail(job_id, e, total_passes, &mut state, rep);
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
                    perform_cancel(job_id, pass_index, submitted_bytes, passes.len(), drv, tr, &mut state, rep, &cancel_flag);
                }
                // else: nothing active, safe no-op.
            }
            Command::Resume { reply } => match (&state, active_job.take()) {
                (DeviceState::WaitingForColorSwap { .. }, Some(job)) => {
                    let JobProgress { job_id, passes, pass_index } = job;
                    let total_passes = passes.len();
                    let drv = driver.as_deref().expect("driver present while job active");
                    let tr = transport.as_deref_mut().expect("transport present while job active");
                    match run_from_pass(job_id, pass_index, &passes, drv, tr, &mut state, rep, &cancel_flag) {
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
                            perform_cancel(job_id, pass_index, submitted_bytes, total_passes, drv, tr, &mut state, rep, &cancel_flag);
                            let _ = reply.send(Ok(()));
                        }
                        Err(e) => {
                            // pass_index >= 1 here, so session_begin already went out.
                            // This is a transport failure, not a cancel, so we
                            // deliberately skip the best-effort abort_bytes write
                            // (that belongs to perform_cancel's cancel path) — a
                            // write that just failed is unlikely to accept an abort.
                            let e = fail(job_id, e, total_passes, &mut state, rep);
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
                    match finish_pass(job_id, pass_index, total_passes, drv, tr, &mut state, rep) {
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
                            let e = fail(job_id, e, total_passes, &mut state, rep);
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
    use crate::{
        DriverError, Ended, MachineCaps, MachineProfile, MockTransport, PassPosition, Phase, Settings, TransportKind,
    };
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// `FakeDriver`'s prologue. Named because two tests count it on the wire, and
    /// one of them subtracts its length from a reported byte total.
    const FAKE_SESSION_BEGIN: [u8; 2] = [0x1b, 0x04];

    /// A payload big enough to need several chunk writes, so a gate on the second
    /// write catches the worker mid-pass. How large a chunk the manager writes is
    /// its own business; a test only needs a job that outgrows it.
    const MULTI_CHUNK_PAYLOAD: usize = 64 * 1024;

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
        fn session_begin(&self) -> Vec<u8> { FAKE_SESSION_BEGIN.to_vec() }
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
            host: None,
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
        assert_eq!(mgr.status().phase, Phase::Disconnected);
        mgr.connect(cameo_info()).unwrap();
        assert_eq!(mgr.status().phase, Phase::Idle);
        let evs: Vec<_> = events.try_iter().collect();
        let changed_to = |phase| {
            evs.iter().any(|e| matches!(e.kind, DeviceEventKind::StateChanged) && e.status.phase == phase)
        };
        assert!(changed_to(Phase::Connecting));
        assert!(changed_to(Phase::Idle));
        mgr.shutdown();
    }

    #[test]
    fn connect_failure_yields_error_state_and_reconnect_recovers() {
        let (mgr, _events) = DeviceManager::spawn(Arc::new(FlakyOpenFactory::new()));
        assert!(mgr.connect(cameo_info()).is_err());
        assert_eq!(mgr.status().phase, Phase::Failed);
        mgr.connect(cameo_info()).unwrap(); // recovery: a later successful connect clears the failure
        assert_eq!(mgr.status().phase, Phase::Idle);
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
        assert_eq!(mgr.status().phase, Phase::Idle);
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
        assert_eq!(mgr.status().phase, Phase::Disconnected);
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
            host: None,
        }
    }

    /// A machine that needs an operator to confirm each pass by hand — no
    /// status polling. The one scripted read is the connect-time probe reply, which a
    /// candidate device owes before it is accepted; reads stay empty after that on purpose,
    /// so a bug that polled anyway would hit an immediate `TransportError::Timeout` and
    /// surface fast.
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
            Ok(Box::new(MockTransport { reads: VecDeque::from(vec![Ok(b"0\r".to_vec())]), ..Default::default() }))
        }
    }

    fn empty_job() -> Job { Job { polylines: Vec::new(), settings: Settings::default() } }
    fn one_pass_job() -> Vec<CutPass> { vec![CutPass { job: empty_job() }] }
    fn two_pass_job() -> Vec<CutPass> { vec![CutPass { job: empty_job() }, CutPass { job: empty_job() }] }

    /// Polls `status()` — which never blocks, so this cannot itself be held up by
    /// the worker it is waiting on — until the phase arrives.
    fn wait_for_phase(mgr: &DeviceManager, want: Phase) -> CutStatus {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let s = mgr.status();
            if s.phase == want {
                return s;
            }
            assert!(Instant::now() < deadline, "never reached {want:?}, last was {:?}", s.phase);
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Like `wait_for_phase`, but for an ending. Every ending rests on `Phase::Idle`,
    /// so the phase alone cannot say that a cancel has landed.
    fn wait_for_ended(mgr: &DeviceManager, want: Ended) -> CutStatus {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let s = mgr.status();
            if s.ended == Some(want) {
                return s;
            }
            assert!(Instant::now() < deadline, "never ended {want:?}, last was {:?}/{:?}", s.phase, s.ended);
            thread::sleep(Duration::from_millis(10));
        }
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
        let parked = wait_for_phase(&mgr, Phase::AwaitingColorSwap);
        assert_eq!(parked.pass, Some(PassPosition { index: 1, total: 2 }), "parked before pass 2 of 2");
        mgr.resume().unwrap();
        wait_for_phase(&mgr, Phase::Idle);

        let evs = drain(&events);
        assert!(!evs.is_empty());
        assert!(evs.iter().all(|e| e.job_id == job_id));
        assert_eq!(evs.iter().filter(|e| matches!(e.kind, DeviceEventKind::PassComplete(_))).count(), 2);
        assert!(evs.iter().any(|e| matches!(e.kind, DeviceEventKind::JobComplete)));

        let written = written.lock().unwrap();
        assert_eq!(count_subseq(&written, &FAKE_SESSION_BEGIN), 1, "one prologue for the whole job");
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
        mgr2.cut(two_pass_job()).unwrap(); // pauses for the color swap after pass 1
        wait_for_phase(&mgr2, Phase::AwaitingColorSwap);
        assert_eq!(mgr2.cut(one_pass_job()).unwrap_err(), DeviceError::Busy); // job still active

        mgr.shutdown();
        mgr2.shutdown();
    }

    #[test]
    fn operator_confirm_path_for_puma_caps() {
        let (mgr, events) = DeviceManager::spawn(Arc::new(PumaFactory));
        mgr.connect(puma_info()).unwrap();
        let job_id = mgr.cut(one_pass_job()).unwrap();
        let awaiting = wait_for_phase(&mgr, Phase::AwaitingConfirmation);
        assert!(awaiting.actions.confirm, "the operator is the only way this pass completes");
        mgr.confirm_pass_done().unwrap();
        wait_for_phase(&mgr, Phase::Idle);

        let evs = drain(&events);
        assert!(evs.iter().any(|e| e.job_id == job_id && matches!(e.kind, DeviceEventKind::JobComplete)));
        mgr.shutdown();
    }

    /// A factory whose single transport answers `reads` — enough to script what a device
    /// says (or refuses to say) when the manager probes it on connect.
    fn probe_factory(info: DeviceInfo, reads: Vec<Result<Vec<u8>, TransportError>>) -> Arc<ScriptedFactory> {
        Arc::new(ScriptedFactory {
            info,
            profile: MachineProfile { id: "puma".into(), name: "Puma".into(), width_mm: 300.0, height_mm: 1000.0 },
            caps: MachineCaps { supports_speed: false, supports_force: false, needs_operator_pass_confirm: true },
            abort: None,
            payload_len: 8,
            park_bytes: Vec::new(),
            transports: Mutex::new(VecDeque::from(vec![
                Box::new(MockTransport { reads: reads.into(), ..Default::default() }) as Box<dyn Transport>,
            ])),
        })
    }

    #[test]
    fn candidate_device_answering_status_connects() {
        let (mgr, _events) = DeviceManager::spawn(probe_factory(puma_info(), vec![Ok(b"0\r".to_vec())]));
        mgr.connect(puma_info()).unwrap();
        assert_eq!(mgr.status().phase, Phase::Idle);
        mgr.shutdown();
    }

    #[test]
    fn silent_candidate_device_is_refused_instead_of_connected() {
        // A serial port that accepts bytes and answers nothing could be any device at all:
        // a label printer, a debug console. Connecting would let a cut job transmit into it.
        let (mgr, _events) = DeviceManager::spawn(probe_factory(puma_info(), Vec::new()));
        let err = mgr.connect(puma_info()).unwrap_err();
        assert!(matches!(err, DeviceError::Io(_)), "refusal must carry a reason, got {err:?}");
        let s = mgr.status();
        assert_eq!(s.phase, Phase::Failed);
        assert!(matches!(s.error, Some(DeviceError::Io(_))), "and status must repeat the reason");
        mgr.shutdown();
    }

    #[test]
    fn candidate_device_answering_junk_is_refused() {
        let (mgr, _events) = DeviceManager::spawn(probe_factory(puma_info(), vec![Ok(b"BROTHER".to_vec())]));
        assert!(mgr.connect(puma_info()).is_err(), "a reply that is not a status char proves nothing");
        mgr.shutdown();
    }

    #[test]
    fn vid_pid_matched_device_connects_without_being_probed() {
        // USB identity is settled by enumeration, so no bytes are owed on connect. This
        // transport scripts no reads at all: a probe would time out and fail the connect.
        let (mgr, _events) = DeviceManager::spawn(probe_factory(cameo_info(), Vec::new()));
        mgr.connect(cameo_info()).unwrap();
        mgr.shutdown();
    }

    #[test]
    fn stale_job_events_are_distinguishable() {
        let (factory, _written) = factory_with_ready_reads(2);
        let (mgr, events) = DeviceManager::spawn(factory);
        mgr.connect(cameo_info()).unwrap();

        let job1 = mgr.cut(one_pass_job()).unwrap();
        wait_for_phase(&mgr, Phase::Idle);
        let job2 = mgr.cut(one_pass_job()).unwrap();
        wait_for_phase(&mgr, Phase::Idle);

        assert_ne!(job1, job2);
        let evs = drain(&events);
        assert!(evs.iter().any(|e| e.job_id == job1 && matches!(e.kind, DeviceEventKind::JobComplete)));
        assert!(evs.iter().any(|e| e.job_id == job2 && matches!(e.kind, DeviceEventKind::JobComplete)));
        // Distinct ids alone would still let a listener see job 1's tail after job 2
        // had started, which is why the desktop UI used to filter on job id at all.
        // One worker sending every event in order down one channel is what let that
        // filtering go, so it is asserted here rather than left to a comment.
        let first2 = evs.iter().position(|e| e.job_id == job2).expect("job2 reported");
        assert!(evs[first2..].iter().all(|e| e.job_id != job1), "no job-1 event may follow job 2's first");
        mgr.shutdown();
    }

    // --- what a caller used to have to work out for itself ----------------
    // The three tests below were `acceptEvent` and `terminalTransition` in
    // apps/desktop/ui/src/cut/viewmodel.ts: rules a caller re-derived because the
    // manager did not state them. They belong here, against the manager, or the
    // next caller re-derives them too.

    /// Was `acceptEvent`. A caller should not have to track job ids to know an
    /// event belongs to the job it is watching.
    #[test]
    fn events_from_a_finished_job_do_not_reopen_it() {
        let (factory, _written) = factory_with_ready_reads(1);
        let (mgr, events) = DeviceManager::spawn(factory);
        mgr.connect(cameo_info()).unwrap();

        let job = mgr.cut(one_pass_job()).expect("cut");
        let seen: Vec<DeviceEvent> = events.try_iter().collect();
        assert!(
            seen.iter().all(|e| e.job_id == job || e.job_id == NO_JOB),
            "no event may carry a foreign job id: {:?}",
            seen.iter().map(|e| e.job_id).collect::<Vec<_>>()
        );
        assert!(seen.iter().any(|e| e.job_id == job), "the job did report events of its own");
        assert_eq!(mgr.status().phase, Phase::Idle, "the job is over");
        mgr.shutdown();
    }

    /// The worker's half of the ending. A job that ran to the end rests on `Idle`,
    /// which says nothing by itself, so the worker remembers `Completed` — and has to
    /// forget it in the two places it would otherwise be read as this job's own: a
    /// reconnect (a fresh device has cut nothing) and the next `cut`.
    #[test]
    fn a_completed_job_reports_completion_until_something_supersedes_it() {
        let (factory, _written) = factory_with_ready_reads(2);
        let (mgr, _events) = DeviceManager::spawn(factory);
        mgr.connect(cameo_info()).unwrap();
        assert_eq!(mgr.status().ended, None, "nothing has run yet");

        mgr.cut(one_pass_job()).expect("cut");
        let done = wait_for_ended(&mgr, Ended::Completed);
        assert_eq!(done.phase, Phase::Idle, "phase says what is happening now: nothing");
        assert_eq!(done.pass, None, "and a finished job leaves no pass position behind");

        mgr.disconnect().expect("disconnect");
        mgr.connect(cameo_info()).expect("reconnect");
        assert_eq!(mgr.status().ended, None, "a fresh connection has cut nothing");

        mgr.cut(two_pass_job()).expect("cut"); // parks for the swap, so it is still in flight
        let parked = wait_for_phase(&mgr, Phase::AwaitingColorSwap);
        assert_eq!(parked.ended, None, "a job in flight has not ended");
        mgr.shutdown();
    }

    /// Was `terminalTransition`'s Cancelled case. A cancel ends the job, but it
    /// arrives as a resting state rather than a terminal event kind — a caller
    /// watching only for `JobComplete`/`Failed` waits forever. `ended`/`actions`
    /// say it outright.
    #[test]
    fn a_cancelled_job_reports_cancelled_and_allows_another_cut() {
        // Reads: pass 1's completion poll, the post-cancel readiness check, then
        // the replacement job's own poll.
        let (factory, _written) = factory_with_ready_reads(3);
        let (mgr, _events) = DeviceManager::spawn(factory);
        mgr.connect(cameo_info()).unwrap();

        mgr.cut(two_pass_job()).expect("cut"); // parks for the color swap, so there is a job to cancel
        wait_for_phase(&mgr, Phase::AwaitingColorSwap);
        mgr.cancel();

        let s = wait_for_ended(&mgr, Ended::Cancelled);
        assert!(s.actions.cut, "another cut is legal after a cancel");
        mgr.cut(one_pass_job()).expect("and the manager honours what actions promised");
        mgr.shutdown();
    }

    /// Was the `NO_JOB` release case: the viewmodel's job filter outlived the job
    /// and swallowed lifecycle events, so a disconnect went unnoticed.
    #[test]
    fn lifecycle_events_survive_a_finished_job() {
        let (factory, _written) = factory_with_ready_reads(1);
        let (mgr, events) = DeviceManager::spawn(factory);
        mgr.connect(cameo_info()).unwrap();

        mgr.cut(one_pass_job()).expect("cut");
        drain(&events); // the job's own events are not what this is about

        mgr.disconnect().expect("disconnect");
        let after = drain(&events);
        assert!(after.iter().any(|e| e.job_id == NO_JOB), "disconnect must be reported");
        assert_eq!(mgr.status().phase, Phase::Disconnected);
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
    /// asserts: transmission advanced by exactly one more chunk and then stopped,
    /// the reported byte total matches what actually landed on the wire, exactly
    /// one `abort_bytes` write went out, and the final `Cancelled` event carries
    /// `expect_completion_known`.
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
            payload_len: MULTI_CHUNK_PAYLOAD,
            park_bytes: Vec::new(),
            transports: Mutex::new(VecDeque::from(vec![Box::new(gate) as Box<dyn Transport>])),
        };
        let (mgr, events) = DeviceManager::spawn(Arc::new(factory));
        mgr.connect(cameo_info()).unwrap();
        drain(&events);

        // Read inside the gate but assert outside it: an assertion that fired
        // before the gate is released would leave the cut thread parked forever
        // and `thread::scope` would hang the suite instead of failing it.
        let (mid_transmit, job_id) = thread::scope(|scope| {
            let cut_thread = scope.spawn(|| mgr.cut(one_pass_job()).unwrap());
            // Timed out, not blocking: the sender lives in a transport the still-running
            // worker owns, so a gate the worker never reaches (a changed chunk size, a
            // smaller payload) must fail this test rather than hang the suite.
            ready_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("worker reached the gated write");
            let mid_transmit = mgr.status();
            mgr.cancel();
            proceed_tx.send(()).unwrap();
            (mid_transmit, cut_thread.join().unwrap())
        });

        // Cancelled is the resting state, and it reports its ending — so waiting on
        // the published status is enough to know the cancel landed.
        let at_cancel = wait_for_ended(&mgr, Ended::Cancelled);
        let evs = drain(&events);

        let mid = mid_transmit.sent.expect("a transmitting cut reports bytes");
        let stopped = at_cancel.sent.expect("a cancelled cut reports how far it got");
        assert_eq!(mid_transmit.phase, Phase::Sending, "the gate caught the worker mid-transmit");
        assert!(mid.sent > 0 && mid.sent < mid.total, "partial progress: {mid:?}");
        // The gate held chunk 2, so releasing it lands exactly that one chunk before
        // the cancel flag is next checked. Equal-sized chunks is the cadence under
        // test; what size they are is the manager's business.
        assert_eq!(stopped.sent, 2 * mid.sent, "one further chunk landed, then writes stopped");
        assert!(stopped.sent < mid.total, "the pass did not finish transmitting");

        let (payload_written, aborts) = {
            let written = mirror.lock().unwrap();
            (written.iter().filter(|&&b| b == 0xAA).count(), count_subseq(&written, b"PU;"))
        };
        // Chunk 1 carries the prologue plus payload, so the payload bytes on the
        // wire are the reported total less that prologue. A reported byte count
        // that no transport actually accepted would be worse than no count.
        assert_eq!(
            payload_written,
            stopped.sent - FAKE_SESSION_BEGIN.len(),
            "reported progress must match the payload bytes that landed"
        );
        assert_eq!(aborts, 1, "abort bytes written exactly once");

        // The stop is announced through the event stream before the terminal phase
        // lands. `CancelRequested` and `Stopping` both report as `Cancelling` — a
        // caller can do nothing different in one than the other, which is why the
        // status collapses them.
        assert!(evs.iter().any(|e| e.job_id == job_id
            && matches!(e.kind, DeviceEventKind::StateChanged)
            && e.status.phase == Phase::Cancelling));

        // What a caller does differs on `completion_known`: it does not start another
        // Job into a machine nothing saw stop. Asserted through `actions` — the only
        // thing a caller reads — with the raw state alongside it, so a mapping that
        // stopped reading the field would fail here rather than pass by coincidence.
        assert!(matches!(
            mgr.snapshot(),
            DeviceState::Cancelled { completion_known, .. } if completion_known == expect_completion_known
        ));
        assert_eq!(
            at_cancel.actions.cut, expect_completion_known,
            "actions must not offer a cut the machine never confirmed it had stopped for"
        );
        if !expect_completion_known {
            assert_eq!(
                mgr.cut(one_pass_job()).unwrap_err(),
                DeviceError::Busy,
                "and the guard refuses what actions withheld, so a caller that ignored actions gains nothing"
            );
        }
        mgr.shutdown();
    }

    /// The way out of a stop nobody confirmed, and the reason no "mark it ready"
    /// control is needed: the operator goes to the machine, sees it at rest, and
    /// reconnects. Disconnect leaves any state, so the cancelled rest state goes with
    /// the transport — and for a serial candidate the reconnect re-probes real
    /// hardware rather than taking anyone's word for it.
    #[test]
    fn reconnecting_clears_a_cancel_whose_stop_was_never_confirmed() {
        // A Puma: `needs_operator_pass_confirm`, so no cancel of one can ever confirm.
        let (mgr, _events) = DeviceManager::spawn(Arc::new(PumaFactory));
        mgr.connect(puma_info()).unwrap();
        mgr.cut(one_pass_job()).unwrap();
        wait_for_phase(&mgr, Phase::AwaitingConfirmation);
        mgr.cancel();

        let stopped = wait_for_ended(&mgr, Ended::Cancelled);
        assert!(!stopped.actions.cut, "a Puma's stop is never confirmed, so no Job may follow it");
        assert_eq!(mgr.cut(one_pass_job()).unwrap_err(), DeviceError::Busy);

        mgr.disconnect().unwrap();
        mgr.connect(puma_info()).unwrap();
        assert!(mgr.status().actions.cut, "a reconnected cutter accepts a Job again");
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
        // MockTransport clamps a scripted count to the buffer it is handed, so
        // usize::MAX is "accept whatever you are offered" — two writes through,
        // then the cable goes, whatever size the manager writes in.
        let write_results =
            VecDeque::from(vec![Ok(usize::MAX), Ok(usize::MAX), Err(TransportError::Io("cable pulled".into()))]);
        let inner = MockTransport { write_results, ..Default::default() };
        let factory = ScriptedFactory {
            info: cameo_info(),
            profile: MachineProfile { id: "cameo5".into(), name: "Cameo 5".into(), width_mm: 305.0, height_mm: 1000.0 },
            caps: cameo_caps,
            abort: None,
            payload_len: MULTI_CHUNK_PAYLOAD,
            park_bytes: Vec::new(),
            transports: Mutex::new(VecDeque::from(vec![Box::new(inner) as Box<dyn Transport>])),
        };
        let (mgr, events) = DeviceManager::spawn(Arc::new(factory));
        mgr.connect(cameo_info()).unwrap();
        drain(&events);

        let err = mgr.cut(one_pass_job()).unwrap_err();
        assert_eq!(err, DeviceError::Io("cable pulled".into()));
        let s = mgr.status();
        assert_eq!(s.phase, Phase::Failed);
        assert_eq!(s.error, Some(DeviceError::Io("cable pulled".into())));

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
        let s = mgr.status();
        assert_eq!(s.phase, Phase::Failed);
        assert_eq!(s.error, Some(expect), "status names the same failure the call returned");
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
    fn unloaded_media_fails_fast_instead_of_polling_out_the_deadline() {
        struct UnloadedFactory;
        impl DeviceBackendFactory for UnloadedFactory {
            fn list_devices(&self) -> Vec<DeviceInfo> { vec![cameo_info()] }
            fn driver_for(&self, _m: &str) -> Option<Box<dyn Driver + Send>> { Some(fake_driver()) }
            fn open_transport(&self, _i: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
                let mut reads = VecDeque::new();
                reads.push_back(Ok(b"2\x03".to_vec())); // status: unloaded
                Ok(Box::new(MockTransport { reads, ..Default::default() }))
            }
        }
        let (mgr, _events) = DeviceManager::spawn(Arc::new(UnloadedFactory));
        mgr.connect(cameo_info()).unwrap();
        let start = std::time::Instant::now();
        let err = mgr.cut(two_pass_job()).unwrap_err();
        assert_eq!(err, DeviceError::Io("media unloaded".to_string()));
        // The whole point: an unloaded reply must not silently poll out the 60s cap.
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    /// The desktop cannot use `snapshot()`: it round-trips through the worker's
    /// command channel, so it blocks for as long as the worker is inside a
    /// transport write — which is why `apps/desktop/src/device.rs` grew a second
    /// state cache of its own. `status()` reads published memory instead, so it
    /// answers while the worker is busy. The gate parks the worker inside the
    /// second chunk write, so this needs no polling: reaching that point means
    /// `Transmitting` was already published.
    #[test]
    fn status_answers_while_the_worker_is_transmitting() {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (proceed_tx, proceed_rx) = mpsc::channel();
        let inner = MockTransport { reads: VecDeque::from(vec![Ok(b"0\x03".to_vec())]), ..Default::default() };
        let gate = GateTransport {
            inner,
            mirror: Arc::new(Mutex::new(Vec::new())),
            call_index: 0,
            block_on: 2,
            sync: Some((ready_tx, proceed_rx)),
        };
        let factory = ScriptedFactory {
            info: cameo_info(),
            profile: MachineProfile { id: "cameo5".into(), name: "Cameo 5".into(), width_mm: 305.0, height_mm: 1000.0 },
            caps: MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: false },
            abort: None,
            payload_len: MULTI_CHUNK_PAYLOAD,
            park_bytes: Vec::new(),
            transports: Mutex::new(VecDeque::from(vec![Box::new(gate) as Box<dyn Transport>])),
        };
        let (mgr, _events) = DeviceManager::spawn(Arc::new(factory));
        mgr.connect(cameo_info()).unwrap();

        // Read the status inside the gate, but assert outside it: an assertion that
        // fired before releasing the gate would leave the cut thread parked forever
        // and `thread::scope` would hang instead of failing.
        let s = thread::scope(|scope| {
            scope.spawn(|| {
                let _ = mgr.cut(one_pass_job());
            });
            // Timed out for the same reason as `assert_cancel_mid_transmit`'s gate: an
            // unreached gate must fail this test rather than hang the suite.
            ready_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("worker reached the gated write");
            let s = mgr.status();
            proceed_tx.send(()).unwrap();
            s
        });
        assert_eq!(s.phase, Phase::Sending, "status() must answer from published memory, not the worker");
        assert!(s.actions.cancel, "a sending cut can be cancelled");
        assert_eq!(s.pass, Some(PassPosition { index: 0, total: 1 }), "pass count comes from the submitted Vec");
        // Chunk 1 landed before the gate caught chunk 2, so its per-chunk publish must
        // already be in the cell — proving the in-loop publish, not just its event.
        let progress = s.sent.expect("a sending cut reports bytes");
        assert!(progress.sent > 0 && progress.sent < progress.total, "partial progress: {progress:?}");

        assert_eq!(mgr.status().phase, Phase::Idle, "a finished job leaves no pass count behind");
        assert_eq!(mgr.status().pass, None);
        mgr.shutdown();
    }

    /// A published cell only moves while something is publishing. Once the worker
    /// is gone the last value would stand forever — `Idle` here, or worse a
    /// `Sending` with a live cancel button — so `status()` must report the dead
    /// worker instead. `snapshot()` gets this from its channel error; this is the
    /// same promise for the read path that replaces it.
    #[test]
    fn status_reports_a_dead_worker_instead_of_the_frozen_cell() {
        let (mgr, _events) = DeviceManager::spawn(Arc::new(test_factory()));
        mgr.connect(cameo_info()).unwrap();
        assert_eq!(mgr.status().phase, Phase::Idle, "the cell holds a healthy state to be overridden");

        // Kill the worker without consuming the manager, which `shutdown()` would.
        mgr.cmd_tx.try_send(Command::Shutdown).unwrap();
        for _ in 0..200 {
            if mgr.status().phase == Phase::Failed {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let s = mgr.status();
        assert_eq!(s.phase, Phase::Failed, "a finished worker must not leave the cell reading Idle");
        assert_eq!(s.error, Some(DeviceError::Disconnected), "and it must say why");
        assert!(!s.actions.cut && !s.actions.cancel, "nothing can be done to a device with no worker");
        mgr.shutdown();
    }

    #[test]
    fn shutdown_mid_job_cancels_and_joins() {
        let (factory, _written) = factory_with_ready_reads(1);
        let (mgr, events) = DeviceManager::spawn(factory);
        mgr.connect(cameo_info()).unwrap();
        let job_id = mgr.cut(two_pass_job()).unwrap();
        wait_for_phase(&mgr, Phase::AwaitingColorSwap);
        drain(&events);

        let start = std::time::Instant::now();
        mgr.shutdown();
        assert!(start.elapsed() < Duration::from_secs(2), "shutdown should cancel and join promptly");

        let evs = drain(&events);
        let own = |e: &DeviceEvent| e.job_id == job_id && matches!(e.kind, DeviceEventKind::StateChanged);
        assert!(evs.iter().any(|e| own(e) && e.status.phase == Phase::Cancelling));
        // Cancelled is the resting state post-shutdown (no further Cut arrives to
        // lazily flip it back to Idle), and it reports how the job ended.
        assert!(evs.iter().any(|e| own(e) && e.status.ended == Some(Ended::Cancelled)));
    }
}
