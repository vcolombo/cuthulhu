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

/// What the machine is waiting for the operator to do, or `None` if it is waiting
/// for nothing. Read from `status.actions`, never from `status.phase`: `driver-core`
/// owns which calls are legal (`status.rs`), and a caller that maps phases back to
/// permissions has to be re-audited every time a phase is added — which is the audit
/// `actions` exists to delete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pause {
    /// `resume()` is legal: parked between colours for a tool change.
    Swap,
    /// `confirm_pass_done()` is legal: the machine cannot be polled, so a human says
    /// when the pass finished.
    Confirm,
}

/// The two are never both legal — one parks before a pass, the other after — so the
/// order here decides nothing.
fn pause_of(status: &CutStatus) -> Option<Pause> {
    if status.actions.resume {
        Some(Pause::Swap)
    } else if status.actions.confirm {
        Some(Pause::Confirm)
    } else {
        None
    }
}

/// Which pass to name to the operator and how many there are, counting from 1. A
/// paused or cancelled job always reports both; `1/1` is only a fallback so a missing
/// position cannot panic a live cut.
fn pass_position(status: &CutStatus) -> (usize, usize) {
    status.pass.map(|p| (p.index + 1, p.total)).unwrap_or((1, 1))
}

/// What to ask the operator for. Returned rather than printed so the wording is
/// assertable, and taking the whole position from the status so the index and its
/// denominator cannot come from two different accounts of the job.
fn pause_prompt(pause: Pause, plan: &cutplan::CutPlan, status: &CutStatus) -> String {
    let (pass, total) = pass_position(status);
    let color = pass_color(plan, status);
    match pause {
        Pause::Swap => format!("Pass {pass}/{total} (color {color}): swap tool, press Enter to resume"),
        Pause::Confirm => {
            format!("Pass {pass}/{total} (color {color}) cutting; press Enter once the machine finishes")
        }
    }
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
    // The plan's own count, needed before a device exists (the guard below) and after
    // the job has stopped reporting a position — a finished cut rests on `Idle`, which
    // carries none. The prompts deliberately do not use it: while a job is running the
    // status reports both halves of the position itself.
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
        // What the operator may answer comes from `actions`; `phase` is only asked
        // what is happening once there is nothing to answer.
        if let Some(pause) = pause_of(&status) {
            if operator.wait_ack(&pause_prompt(pause, plan, &status), &mgr) {
                let (what, answered) = match pause {
                    Pause::Swap => ("resume", mgr.resume()),
                    Pause::Confirm => ("confirm", mgr.confirm_pass_done()),
                };
                answer_pause(what, answered)?;
            }
            continue;
        }
        match status.phase {
            // Nothing is happening, so the job is over and the operator has nothing
            // left to answer. `ended` is what says which ending it was.
            Phase::Idle => {
                return match status.ended {
                    Some(Ended::Cancelled) => Ok(Outcome::Cancelled {
                        pass: pass_position(&status).0,
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

#[cfg(test)]
mod tests {
    use super::*;
    use driver_core::{Actions, Job, PassPosition, Settings};

    fn status(actions: Actions, phase: Phase, pass: Option<PassPosition>) -> CutStatus {
        CutStatus { phase, ended: None, actions, pass, sent: None, error: None }
    }

    fn plan(colors: &[Option<u32>]) -> cutplan::CutPlan {
        cutplan::CutPlan {
            passes: colors
                .iter()
                .map(|&color| cutplan::PlannedPass {
                    color,
                    job: Job { polylines: vec![], settings: Settings::default() },
                })
                .collect(),
        }
    }

    /// The phases are deliberately wrong here: `driver-core` owns which calls are
    /// legal, and a loop that re-derives that from the phase has to be re-audited
    /// every time a phase is added. Reading `actions` is what deletes that audit, so
    /// a status that permits an answer must be answered whatever it calls itself.
    #[test]
    fn the_pause_is_read_from_actions_not_from_the_phase() {
        let resume = Actions { cancel: true, resume: true, ..Actions::default() };
        assert_eq!(pause_of(&status(resume, Phase::Sending, None)), Some(Pause::Swap));

        let confirm = Actions { cancel: true, confirm: true, ..Actions::default() };
        assert_eq!(pause_of(&status(confirm, Phase::Sending, None)), Some(Pause::Confirm));

        // Mid-flight: cancellable, but there is nothing for the operator to answer.
        let sending = Actions { cancel: true, ..Actions::default() };
        assert_eq!(pause_of(&status(sending, Phase::AwaitingConfirmation, None)), None);
        assert_eq!(pause_of(&status(Actions::default(), Phase::Failed, None)), None);
    }

    /// Both halves of "2/3" come from the status, so the number being counted and the
    /// number it counts towards cannot come from two different accounts of the job.
    #[test]
    fn a_prompt_takes_both_halves_of_the_position_from_the_status() {
        let plan = plan(&[Some(0xff0000ff), Some(0x0000ffff), None]);
        let at_second = status(
            Actions { cancel: true, resume: true, ..Actions::default() },
            Phase::AwaitingColorSwap,
            Some(PassPosition { index: 1, total: 3 }),
        );

        let swap = pause_prompt(Pause::Swap, &plan, &at_second);
        assert!(swap.contains("Pass 2/3"), "counts from 1, out of the job's own total: {swap}");
        assert!(swap.contains("#0000ff"), "names the colour being swapped to: {swap}");
        assert!(swap.contains("swap tool"), "says what to do: {swap}");

        let confirm = pause_prompt(Pause::Confirm, &plan, &at_second);
        assert!(confirm.contains("Pass 2/3"), "{confirm}");
        assert!(confirm.contains("once the machine finishes"), "waits on the blade, not the queue: {confirm}");
    }

    /// A plain cut's pass has no colour to name since #144 — it is one pass by request, not one
    /// colour's worth of shapes. The prompt used to read `#000000`, which was the invented stroke
    /// the plain path stamped on every path; nothing pinned it, so nothing would catch it
    /// changing.
    #[test]
    fn a_colourless_pass_is_named_none_in_the_prompt() {
        let plan = plan(&[None]);
        let parked = status(
            Actions { cancel: true, confirm: true, ..Actions::default() },
            Phase::AwaitingConfirmation,
            Some(PassPosition { index: 0, total: 1 }),
        );
        let confirm = pause_prompt(Pause::Confirm, &plan, &parked);
        assert!(confirm.contains("(color none)"), "{confirm}");
    }
}
