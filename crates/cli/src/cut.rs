// SPDX-License-Identifier: GPL-3.0-or-later
//! Driving a planned cut on a real device: connect, submit, and answer the
//! machine's pauses until the job ends.
//!
//! Takes its `DeviceBackendFactory` as a parameter rather than building one, so
//! the same code runs against hardware and against `MockTransport`.
use std::sync::Arc;

use driver_core::manager::{DeviceError, DeviceManager};
use driver_core::{CutStatus, DeviceBackendFactory, DeviceInfo, Ended, Phase};

/// Who answers the machine's pauses.
///
/// `Unattended` exists because a pass on a machine that cannot be polled
/// (`MachineCaps::needs_operator_pass_confirm`) otherwise blocks on stdin, and a
/// plain cut is often scripted. It acknowledges as soon as the bytes are sent,
/// which is exactly what the plain path did before it went through
/// `DeviceManager` — the host queue draining is not the machine finishing, so it
/// says so on stderr rather than pretending the cut is verified.
///
/// Answering a pause that way is only safe when nothing follows it, so `run` refuses
/// an unattended cut of more than one pass.
pub enum Operator {
    Interactive,
    Unattended,
}

impl Operator {
    /// Wait for acknowledgement. `false` means a cancel landed while waiting.
    fn wait_ack(&self, prompt: &str, mgr: &DeviceManager) -> bool {
        match self {
            Operator::Interactive => {
                println!("{prompt}");
                wait_for_enter_or_cancel(mgr)
            }
            Operator::Unattended => {
                eprintln!("{prompt}: assuming done (stdin is not a terminal; completion not verified)");
                true
            }
        }
    }
}

/// `#RRGGBB` for the operator prompt — drop the alpha byte.
pub fn format_pass_color(color: Option<u32>) -> String {
    match color {
        Some(c) => format!("#{:06x}", c >> 8),
        None => "none".into(),
    }
}

/// Which pass to name to the operator, counting from 1. A paused or cancelled job
/// always reports a position; `1` is only a fallback so a missing one cannot panic
/// a live cut.
fn pass_at(status: &CutStatus) -> usize {
    status.pass.map(|p| p.index + 1).unwrap_or(1)
}

/// The colour of the pass the job is paused on — the reason the operator is being
/// interrupted at all. A bad index can't happen on the normal path (the reported
/// position indexes the same plan), but the prompt is cosmetic, so a mismatch
/// degrades to "none" rather than panicking a process mid-cut.
fn pass_color(plan: &cutplan::CutPlan, status: &CutStatus) -> String {
    let index = status.pass.map(|p| p.index).unwrap_or(0);
    format_pass_color(plan.passes.get(index).and_then(|p| p.color))
}

/// How the cut ended. Returned rather than printed so the wording is assertable
/// against the real loop instead of a helper standing next to it.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed { passes: usize },
    /// `pass` counts from 1, like the prompts: an operator who was just told
    /// "Pass 2/2 cutting" and cancelled must not then read "cancelled at pass 1".
    /// Held 1-based in the type so no reader has to remember to add one.
    Cancelled { pass: usize, sent: usize },
}

/// What to tell the operator once the job has ended.
pub fn ended_message(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Completed { passes } => format!("done: {passes} passes cut"),
        Outcome::Cancelled { pass, sent } => format!("cancelled at pass {pass} ({sent} bytes sent)"),
    }
}

/// Answering the pause the loop last read. A cancel can land between reading the
/// status and answering it — Ctrl-C during a scripted cut, which never waits — and
/// the worker then refuses the answer with `Busy` because the job is already gone.
/// That is the state having moved, not a fault: the next turn of the loop re-reads
/// the status and reports the real ending. Every other `DeviceError` is a device
/// that stopped working, and stays an error.
fn answer_pause(what: &str, result: Result<(), DeviceError>) -> Result<(), String> {
    match result {
        Ok(()) | Err(DeviceError::Busy) => Ok(()),
        Err(e) => Err(format!("{what}: {e:?}")),
    }
}

/// Connect, cut, and drive the job to its end, reporting how it ended. A cancelled
/// cut is an `Outcome`, not an error; a device fault is an `Err`.
///
/// `with_cancel` is handed the connected device before any bytes go out, so the
/// caller can stop the cut from outside: `main` turns it into the Ctrl-C handler, and
/// a test uses it to cancel mid-job. Installing a process-wide signal handler is not
/// a library function's business, and while it lived in here no test binary could
/// drive this loop more than once.
pub fn run(
    plan: &cutplan::CutPlan,
    info: DeviceInfo,
    factory: Arc<dyn DeviceBackendFactory>,
    operator: Operator,
    with_cancel: impl FnOnce(Arc<DeviceManager>) -> Result<(), String>,
) -> Result<Outcome, String> {
    let total = plan.passes.len();
    // The invariant `Unattended` depends on, enforced where the dependency lives
    // rather than only in `check_interactive` upstream. On a machine that parks for
    // confirmation, "the host finished transmitting" is all the host knows — the
    // blade may still be moving — so answering that pause without an operator starts
    // the next pass into a machine that may still be cutting, and answering a colour
    // swap resumes as though someone had changed the tool. One pass has neither
    // hazard: nothing follows the pause it answers.
    if matches!(operator, Operator::Unattended) && total > 1 {
        return Err(format!(
            "unattended cut: {total} passes, but an unattended run cannot answer a pause, and a pass after a pause needs one answered before it may start"
        ));
    }
    let (mgr, _events) = DeviceManager::spawn(factory);
    let mgr = Arc::new(mgr);
    mgr.connect(info).map_err(|e| format!("connect: {e:?}"))?;

    with_cancel(mgr.clone())?;

    mgr.cut(plan.cut_passes()).map_err(|e| format!("cut: {e:?}"))?;

    loop {
        let status = mgr.status();
        match status.phase {
            Phase::AwaitingColorSwap => {
                let prompt = format!(
                    "Pass {}/{} (color {}): swap tool, press Enter to resume",
                    pass_at(&status),
                    total,
                    pass_color(plan, &status),
                );
                if operator.wait_ack(&prompt, &mgr) {
                    answer_pause("resume", mgr.resume())?;
                }
            }
            Phase::AwaitingConfirmation => {
                let prompt = format!(
                    "Pass {}/{} (color {}) cutting; press Enter once the machine finishes",
                    pass_at(&status),
                    total,
                    pass_color(plan, &status),
                );
                if operator.wait_ack(&prompt, &mgr) {
                    answer_pause("confirm", mgr.confirm_pass_done())?;
                }
            }
            // Nothing is happening, so the job is over and the operator has nothing
            // left to answer. `ended` is what says which ending it was.
            Phase::Idle => {
                return match status.ended {
                    Some(Ended::Cancelled) => Ok(Outcome::Cancelled {
                        pass: pass_at(&status),
                        sent: status.sent.map(|b| b.sent).unwrap_or(0),
                    }),
                    Some(Ended::Completed) => Ok(Outcome::Completed { passes: total }),
                    // Reading a bare `Idle` as success is the inference this task deleted
                    // from the dialog, so it is not reintroduced here: a job that reaches
                    // a rest state without saying how it ended is a `driver-core` bug.
                    None => Err("cut ended without reporting how".into()),
                }
            }
            Phase::Failed => return Err(format!("device error: {:?}", status.error)),
            // Sending / Cancelling / connection phases: nothing for the operator to do.
            _ => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
}

/// Block until the operator presses Enter (`true`) or a cancel lands via
/// Ctrl-C/`DeviceManager::cancel` (`false`). The reader thread is left parked on
/// stdin if cancel wins — fine for a process that's about to exit.
fn wait_for_enter_or_cancel(mgr: &DeviceManager) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = std::io::stdin().read_line(&mut buf);
        let _ = tx.send(());
    });
    loop {
        if rx.try_recv().is_ok() {
            return true;
        }
        // A cancelled job is over — the operator is no longer being asked for
        // anything, so stop waiting on them.
        if mgr.status().ended == Some(Ended::Cancelled) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
