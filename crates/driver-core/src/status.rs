// SPDX-License-Identifier: GPL-3.0-or-later
//! What a caller is told about a cut: where it has got to, and what may be done
//! next.
//!
//! This is the whole of `DeviceManager`'s reporting interface. The internal
//! state machine is not part of it — callers that branch on which phase permits
//! which call end up re-deriving the machine, which is what `actions` exists to
//! prevent.
use serde::Serialize;

use crate::manager::{DeviceError, DeviceState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Phase {
    Disconnected,
    Connecting,
    Disconnecting,
    Idle,
    Sending,
    /// The machine cannot be polled, so a human confirms the pass finished.
    AwaitingConfirmation,
    AwaitingColorSwap,
    Cancelling,
    Done,
    Failed,
}

/// Which calls are legal right now. A caller renders its controls from this and
/// never needs to know the phase-to-permission rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Actions {
    pub cut: bool,
    pub cancel: bool,
    pub resume: bool,
    pub confirm: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PassPosition {
    pub index: usize,
    pub total: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ByteProgress {
    pub sent: usize,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutStatus {
    pub phase: Phase,
    pub actions: Actions,
    pub pass: Option<PassPosition>,
    pub sent: Option<ByteProgress>,
    pub error: Option<DeviceError>,
}

impl CutStatus {
    /// True while a cut is mid-flight — what the window-close guard asks.
    pub fn is_active(&self) -> bool {
        matches!(
            self.phase,
            Phase::Sending | Phase::AwaitingConfirmation | Phase::AwaitingColorSwap | Phase::Cancelling
        )
    }
}

pub(crate) fn status_of(state: &DeviceState, total_passes: usize) -> CutStatus {
    let pass = |index: usize| Some(PassPosition { index, total: total_passes });
    let (phase, actions, pass, sent, error) = match state {
        DeviceState::Disconnected => (Phase::Disconnected, Actions::default(), None, None, None),
        DeviceState::Connecting => (Phase::Connecting, Actions::default(), None, None, None),
        DeviceState::Disconnecting => (Phase::Disconnecting, Actions::default(), None, None, None),
        DeviceState::Idle => (Phase::Idle, Actions { cut: true, ..Actions::default() }, None, None, None),
        DeviceState::Transmitting { pass_index, submitted_bytes, total_bytes, .. } => (
            Phase::Sending,
            Actions { cancel: true, ..Actions::default() },
            pass(*pass_index),
            Some(ByteProgress { sent: *submitted_bytes, total: *total_bytes }),
            None,
        ),
        DeviceState::AwaitingCompletion { pass_index, .. } => (
            Phase::AwaitingConfirmation,
            Actions { cancel: true, confirm: true, ..Actions::default() },
            pass(*pass_index),
            None,
            None,
        ),
        DeviceState::WaitingForColorSwap { next_pass_index, .. } => (
            Phase::AwaitingColorSwap,
            Actions { cancel: true, resume: true, ..Actions::default() },
            pass(*next_pass_index),
            None,
            None,
        ),
        DeviceState::CancelRequested { .. } | DeviceState::Stopping { .. } => {
            (Phase::Cancelling, Actions::default(), None, None, None)
        }
        // A cancelled job has ended. `cut` is legal again, exactly as
        // `manager.rs`'s cut guard already allows.
        DeviceState::Cancelled { pass_index, submitted_bytes, .. } => (
            Phase::Done,
            Actions { cut: true, ..Actions::default() },
            pass(*pass_index),
            Some(ByteProgress { sent: *submitted_bytes, total: *submitted_bytes }),
            None,
        ),
        DeviceState::Error(e) => (Phase::Failed, Actions::default(), None, None, Some(e.clone())),
    };
    CutStatus { phase, actions, pass, sent, error }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{DeviceError, DeviceState};

    /// A caller must be able to render its buttons from `actions` alone, without
    /// knowing which phase permits which call. These four cases are the guards at
    /// manager.rs's `cut`, `resume` and `confirm_pass_done`.
    #[test]
    fn actions_state_which_calls_are_legal() {
        let idle = status_of(&DeviceState::Idle, 0);
        assert_eq!(idle.phase, Phase::Idle);
        assert!(idle.actions.cut && !idle.actions.cancel && !idle.actions.resume && !idle.actions.confirm);

        let swap = status_of(&DeviceState::WaitingForColorSwap { job_id: 1, next_pass_index: 1 }, 3);
        assert_eq!(swap.phase, Phase::AwaitingColorSwap);
        assert!(swap.actions.resume && swap.actions.cancel && !swap.actions.cut && !swap.actions.confirm);

        let await_done = status_of(&DeviceState::AwaitingCompletion { job_id: 1, pass_index: 0 }, 2);
        assert_eq!(await_done.phase, Phase::AwaitingConfirmation);
        assert!(await_done.actions.confirm && await_done.actions.cancel && !await_done.actions.resume);

        let sending = status_of(
            &DeviceState::Transmitting { job_id: 1, pass_index: 0, submitted_bytes: 40, total_bytes: 100 }, 2);
        assert_eq!(sending.phase, Phase::Sending);
        assert!(sending.actions.cancel && !sending.actions.cut);
    }

    /// Pass position and byte progress ride along with the phase, so a caller never
    /// correlates a progress event against a separate state read.
    #[test]
    fn sending_carries_pass_and_byte_position() {
        let s = status_of(
            &DeviceState::Transmitting { job_id: 7, pass_index: 1, submitted_bytes: 4096, total_bytes: 20480 }, 3);
        assert_eq!(s.pass, Some(PassPosition { index: 1, total: 3 }));
        assert_eq!(s.sent, Some(ByteProgress { sent: 4096, total: 20480 }));
    }

    /// A cut that ended is `Done` whether it finished or was cancelled; only a fault
    /// is `Failed`, and it carries the reason.
    #[test]
    fn terminal_phases_are_distinguishable() {
        assert_eq!(status_of(&DeviceState::Idle, 0).phase, Phase::Idle);
        let cancelled = status_of(
            &DeviceState::Cancelled { job_id: 1, pass_index: 0, submitted_bytes: 10, completion_known: false }, 1);
        assert_eq!(cancelled.phase, Phase::Done);
        let failed = status_of(&DeviceState::Error(DeviceError::Timeout), 1);
        assert_eq!(failed.phase, Phase::Failed);
        assert_eq!(failed.error, Some(DeviceError::Timeout));
    }

    /// Replaces `apps/desktop/src/device.rs`'s five-variant `is_active` match, which
    /// the window-close guard uses to decide whether to block a quit.
    #[test]
    fn is_active_covers_every_mid_flight_phase() {
        for state in [
            DeviceState::Transmitting { job_id: 1, pass_index: 0, submitted_bytes: 0, total_bytes: 1 },
            DeviceState::AwaitingCompletion { job_id: 1, pass_index: 0 },
            DeviceState::WaitingForColorSwap { job_id: 1, next_pass_index: 1 },
            DeviceState::CancelRequested { job_id: 1 },
            DeviceState::Stopping { job_id: 1 },
        ] {
            assert!(status_of(&state, 2).is_active(), "{state:?} is mid-flight");
        }
        for state in [DeviceState::Idle, DeviceState::Disconnected] {
            assert!(!status_of(&state, 0).is_active(), "{state:?} is not mid-flight");
        }
    }
}
