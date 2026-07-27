// SPDX-License-Identifier: GPL-3.0-or-later
//! Driving a planned cut on a real device: connect, submit, and answer the
//! machine's pauses until the job ends.
//!
//! Takes its `DeviceBackendFactory` as a parameter rather than building one, so
//! the same code runs against hardware and against `MockTransport`.
use std::sync::Arc;

use driver_core::manager::DeviceManager;
use driver_core::{DeviceBackendFactory, DeviceInfo, Phase};

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
        // A bad index can't happen on the normal path — the reported position indexes
        // the same plan — but the prompt is cosmetic, so a mismatch degrades to "none"
        // rather than panicking a process mid-cut.
        let pass_index = status.pass.map(|p| p.index).unwrap_or(0);
        let color = format_pass_color(plan.passes.get(pass_index).and_then(|p| p.color));
        match status.phase {
            Phase::AwaitingColorSwap => {
                let prompt = format!(
                    "Pass {}/{} (color {}): swap tool, press Enter to resume",
                    pass_index + 1,
                    total,
                    color,
                );
                if operator.wait_ack(&prompt, &mgr) {
                    mgr.resume().map_err(|e| format!("resume: {e:?}"))?;
                }
            }
            Phase::AwaitingConfirmation => {
                let prompt = format!(
                    "Pass {}/{} (color {}) cutting; press Enter once the machine finishes",
                    pass_index + 1,
                    total,
                    color,
                );
                if operator.wait_ack(&prompt, &mgr) {
                    mgr.confirm_pass_done().map_err(|e| format!("confirm: {e:?}"))?;
                }
            }
            // A job that ran to the end rests on `Idle`; a cancelled one on `Done`.
            // Either way the operator has nothing left to answer.
            Phase::Idle | Phase::Done => {
                println!("done: {total} passes cut");
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
