// SPDX-License-Identifier: GPL-3.0-or-later
//! Driving a planned cut on a real device: connect, submit, and answer the
//! machine's pauses until the job ends.
//!
//! Takes its `DeviceBackendFactory` as a parameter rather than building one, so
//! the same code runs against hardware and against `MockTransport`.
use std::sync::Arc;

use driver_core::manager::{DeviceManager, DeviceState};
use driver_core::{DeviceBackendFactory, DeviceInfo};

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
        match mgr.snapshot() {
            DeviceState::WaitingForColorSwap { next_pass_index, .. } => {
                let prompt = format!("Pass {}/{}: swap tool, press Enter to resume", next_pass_index + 1, total);
                if !operator.wait_ack(&prompt, &mgr) {
                    continue; // re-check snapshot: cancel() already landed
                }
                mgr.resume().map_err(|e| format!("resume: {e:?}"))?;
            }
            DeviceState::AwaitingCompletion { pass_index, .. } => {
                let prompt = format!("Pass {}/{} cutting; press Enter once the machine finishes", pass_index + 1, total);
                if !operator.wait_ack(&prompt, &mgr) {
                    continue;
                }
                mgr.confirm_pass_done().map_err(|e| format!("confirm: {e:?}"))?;
            }
            DeviceState::Idle => {
                println!("done: {total} passes cut");
                return Ok(());
            }
            DeviceState::Cancelled { pass_index, submitted_bytes, .. } => {
                println!("cancelled at pass {pass_index} ({submitted_bytes} bytes sent)");
                return Ok(());
            }
            DeviceState::Error(e) => return Err(format!("device error: {e:?}")),
            _ => return Err("unexpected device state".into()),
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
        if matches!(mgr.snapshot(), DeviceState::Cancelled { .. }) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
