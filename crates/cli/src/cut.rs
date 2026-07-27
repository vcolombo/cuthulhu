// SPDX-License-Identifier: GPL-3.0-or-later
//! Driving a planned cut on a real device: connect, submit, and answer the
//! machine's pauses until the job ends.
//!
//! Takes its `DeviceBackendFactory` as a parameter rather than building one, so
//! the same code runs against hardware and against `MockTransport`.
use std::sync::Arc;

use driver_core::manager::DeviceManager;
use driver_core::{CutStatus, DeviceBackendFactory, DeviceInfo, Phase};

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

/// What to tell the operator once the job has ended.
///
/// The two terminal phases are different endings, not one: a job that ran to the
/// end rests on `Idle`, while `Phase::Done` is reached by nothing but a cancel and
/// carries where that cancel stopped. Returned rather than printed so the wording
/// is assertable — `run` itself cannot be, since it owns the manager and installs a
/// process-wide Ctrl-C handler.
fn ended_message(status: &CutStatus, total: usize) -> String {
    if status.phase == Phase::Idle {
        return format!("done: {total} passes cut");
    }
    match (status.pass, status.sent) {
        (Some(p), Some(b)) => format!("cancelled at pass {} ({} bytes sent)", p.index, b.sent),
        // Off the normal path — the cancelled state populates both — but a missing
        // number must not downgrade a cancellation into a report of a finished cut.
        _ => "cancelled".to_string(),
    }
}

/// Connect, cut, and drive the job to its end. `Ok(())` covers a completed cut
/// and a cancelled one; a device fault is an `Err`.
pub fn run(
    plan: &cutplan::CutPlan,
    info: DeviceInfo,
    factory: Arc<dyn DeviceBackendFactory>,
    operator: Operator,
) -> Result<(), String> {
    let total = plan.passes.len();
    let (mgr, _events) = DeviceManager::spawn(factory);
    let mgr = Arc::new(mgr);
    mgr.connect(info).map_err(|e| format!("connect: {e:?}"))?;

    // ponytail: the handler holds a permanent Arc clone for the life of the
    // process, so `mgr` is never uniquely owned again — skip a graceful
    // `shutdown()` and let the (short-lived CLI) process exit reap the worker.
    let ctrlc_mgr = mgr.clone();
    ctrlc::set_handler(move || ctrlc_mgr.cancel()).map_err(|e| format!("ctrlc: {e}"))?;

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
            // Both terminal phases: the operator has nothing left to answer, and
            // `ended_message` says which ending it was.
            Phase::Idle | Phase::Done => {
                println!("{}", ended_message(&status, total));
                return Ok(());
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
        // A cancelled job rests on `Done` — the operator is no longer being asked for
        // anything, so stop waiting on them.
        if mgr.status().phase == Phase::Done {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use driver_core::{Actions, ByteProgress, PassPosition};

    fn terminal(phase: Phase, pass: Option<PassPosition>, sent: Option<ByteProgress>) -> CutStatus {
        CutStatus { phase, actions: Actions { cut: true, ..Actions::default() }, pass, sent, error: None }
    }

    /// The regression this pins: a cancelled job reports `Phase::Done`, a completed one
    /// `Phase::Idle`. Collapsing the two told an operator who hit Ctrl-C that every pass
    /// had been cut.
    #[test]
    fn a_cancelled_job_is_not_reported_as_a_finished_one() {
        let done = terminal(
            Phase::Done,
            Some(PassPosition { index: 1, total: 3 }),
            Some(ByteProgress { sent: 4096, total: 4096 }),
        );
        assert_eq!(ended_message(&done, 3), "cancelled at pass 1 (4096 bytes sent)");

        let idle = terminal(Phase::Idle, None, None);
        assert_eq!(ended_message(&idle, 3), "done: 3 passes cut");
    }

    /// Off the normal path, but a cancellation missing its numbers must still read as a
    /// cancellation — the one thing worse than a vague message is a wrong one.
    #[test]
    fn a_cancellation_without_numbers_still_says_cancelled() {
        assert_eq!(ended_message(&terminal(Phase::Done, None, None), 2), "cancelled");
    }
}
