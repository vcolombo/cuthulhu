// SPDX-License-Identifier: GPL-3.0-or-later
//! Driving a planned cut on a real device: connect, submit, and answer the
//! machine's pauses until the job ends.
//!
//! Takes its `DeviceBackendFactory` as a parameter rather than building one, so
//! the same code runs against hardware and against `MockTransport`.
use std::sync::Arc;

use driver_core::manager::DeviceManager;
use driver_core::{CutStatus, DeviceBackendFactory, DeviceInfo, Ended, Phase};

/// Who answers the machine's pauses.
///
/// `Unattended` exists because a pass on a machine that cannot be polled
/// (`MachineCaps::needs_operator_pass_confirm`) otherwise blocks on stdin, and a
/// plain cut is often scripted. It acknowledges as soon as the bytes are sent,
/// which is exactly what the plain path did before it went through
/// `DeviceManager` — the host queue draining is not the machine finishing, so it
/// says so on stderr rather than pretending the cut is verified.
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

/// Which pass to name in a prompt, counting from 1. A paused job always reports a
/// position; `1` is only a fallback so a missing one cannot panic a live cut.
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
    Cancelled { pass: usize, sent: usize },
}

/// What to tell the operator once the job has ended.
pub fn ended_message(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Completed { passes } => format!("done: {passes} passes cut"),
        Outcome::Cancelled { pass, sent } => format!("cancelled at pass {pass} ({sent} bytes sent)"),
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
                    mgr.resume().map_err(|e| format!("resume: {e:?}"))?;
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
                    mgr.confirm_pass_done().map_err(|e| format!("confirm: {e:?}"))?;
                }
            }
            // Nothing is happening, so the job is over and the operator has nothing
            // left to answer. `ended` is what says which ending it was.
            Phase::Idle => {
                return Ok(match status.ended {
                    Some(Ended::Cancelled) => Outcome::Cancelled {
                        pass: status.pass.map(|p| p.index).unwrap_or(0),
                        sent: status.sent.map(|b| b.sent).unwrap_or(0),
                    },
                    _ => Outcome::Completed { passes: total },
                })
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
