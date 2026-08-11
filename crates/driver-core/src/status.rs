// SPDX-License-Identifier: GPL-3.0-or-later
//! What a caller is told about a cut: where it has got to, how the last one
//! ended, and what may be done next.
//!
//! This is the whole of `DeviceManager`'s reporting interface. The internal
//! state machine is not part of it — callers that branch on which phase permits
//! which call end up re-deriving the machine, which is what `actions` exists to
//! prevent.
use serde::{Deserialize, Serialize};

use crate::manager::{DeviceError, DeviceState};

/// What is happening now, and nothing about what happened before: a job that has
/// ended is not happening, so every ending rests on `Idle`. Which ending it was is
/// `CutStatus::ended`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    Failed,
}

/// How the last job finished, or `None` when none has. Without it `Idle` means
/// three things at once — nothing has run, a cut finished, a device just
/// connected — and every caller has to invent its own memory to tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ended {
    Completed,
    Cancelled,
}

/// Which calls are legal right now. A caller renders its controls from this and
/// never needs to know the phase-to-permission rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Actions {
    pub cut: bool,
    pub cancel: bool,
    pub resume: bool,
    pub confirm: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassPosition {
    pub index: usize,
    pub total: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteProgress {
    pub sent: usize,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutStatus {
    pub phase: Phase,
    pub ended: Option<Ended>,
    pub actions: Actions,
    pub pass: Option<PassPosition>,
    pub sent: Option<ByteProgress>,
    pub error: Option<DeviceError>,
}

impl CutStatus {
    /// What to report when there is no manager to ask — the desktop holds its
    /// `DeviceManager` in an `Option` it empties at shutdown, and a status is
    /// still owed after that.
    pub fn disconnected() -> CutStatus {
        status_of(&DeviceState::Disconnected, 0, None)
    }

    /// True while a cut is mid-flight — what the window-close guard asks.
    pub fn is_active(&self) -> bool {
        matches!(
            self.phase,
            Phase::Sending | Phase::AwaitingConfirmation | Phase::AwaitingColorSwap | Phase::Cancelling
        )
    }
}

/// `ended` is the outcome the worker remembers for a job that ran to the end. A
/// cancelled job needs no such memory: it rests on a state of its own, so it can
/// say how it ended from the state alone.
pub(crate) fn status_of(state: &DeviceState, total_passes: usize, ended: Option<Ended>) -> CutStatus {
    let ended = match state {
        DeviceState::Cancelled { .. } => Some(Ended::Cancelled),
        // A fault is not an ending, and callers render `ended` and `Failed`
        // independently — a remembered ending surviving into `Error` would have a
        // failed cut report itself complete at the same time.
        DeviceState::Error(_) => None,
        _ => ended,
    };
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
        // A cancelled job is over, so nothing is happening: `Idle`, with `ended`
        // saying which ending it was.
        //
        // Over is not stopped. `completion_known` is true only when a poll actually
        // saw the machine come to rest; a Puma cannot be polled at all and its abort
        // is queued behind whatever motion is already buffered, so false is the
        // ordinary case there rather than a rare one. Offering a cut then aims a
        // carriage at a new origin while the last path may still be draining — so
        // `cut` waits for a stop something actually observed. Reconnecting is how an
        // operator who can see the machine at rest gets moving again
        // (`manager.rs`'s `Command::Disconnect` leaves any state).
        DeviceState::Cancelled { pass_index, submitted_bytes, completion_known, .. } => (
            Phase::Idle,
            Actions { cut: *completion_known, ..Actions::default() },
            pass(*pass_index),
            Some(ByteProgress { sent: *submitted_bytes, total: *submitted_bytes }),
            None,
        ),
        DeviceState::Error(e) => (Phase::Failed, Actions::default(), None, None, Some(e.clone())),
    };
    CutStatus { phase, ended, actions, pass, sent, error }
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
        let idle = status_of(&DeviceState::Idle, 0, None);
        assert_eq!(idle.phase, Phase::Idle);
        assert!(idle.actions.cut && !idle.actions.cancel && !idle.actions.resume && !idle.actions.confirm);

        let swap = status_of(&DeviceState::WaitingForColorSwap { job_id: 1, next_pass_index: 1 }, 3, None);
        assert_eq!(swap.phase, Phase::AwaitingColorSwap);
        assert!(swap.actions.resume && swap.actions.cancel && !swap.actions.cut && !swap.actions.confirm);

        let await_done = status_of(&DeviceState::AwaitingCompletion { job_id: 1, pass_index: 0 }, 2, None);
        assert_eq!(await_done.phase, Phase::AwaitingConfirmation);
        assert!(await_done.actions.confirm && await_done.actions.cancel && !await_done.actions.resume);

        let sending = status_of(
            &DeviceState::Transmitting { job_id: 1, pass_index: 0, submitted_bytes: 40, total_bytes: 100 }, 2, None);
        assert_eq!(sending.phase, Phase::Sending);
        assert!(sending.actions.cancel && !sending.actions.cut);
    }

    /// Pass position and byte progress ride along with the phase, so a caller never
    /// correlates a progress event against a separate state read.
    #[test]
    fn sending_carries_pass_and_byte_position() {
        let s = status_of(
            &DeviceState::Transmitting { job_id: 7, pass_index: 1, submitted_bytes: 4096, total_bytes: 20480 }, 3, None);
        assert_eq!(s.pass, Some(PassPosition { index: 1, total: 3 }));
        assert_eq!(s.sent, Some(ByteProgress { sent: 4096, total: 20480 }));
    }

    /// The wart this task removes: a cut that finished and a cut that was cancelled both
    /// rested on `Idle`, so no caller could tell them apart without keeping its own
    /// memory of what it had seen.
    #[test]
    fn a_finished_cut_and_a_cancelled_one_are_distinguishable() {
        let fresh = status_of(&DeviceState::Idle, 0, None);
        assert_eq!(fresh.phase, Phase::Idle);
        assert_eq!(fresh.ended, None, "nothing has run yet");

        let finished = status_of(&DeviceState::Idle, 3, Some(Ended::Completed));
        assert_eq!(finished.phase, Phase::Idle, "phase says what is happening now");
        assert_eq!(finished.ended, Some(Ended::Completed));

        // Passed no remembered outcome on purpose: a cancelled job rests on a state of
        // its own, so it says how it ended without the worker having to remember.
        let cancelled = status_of(
            &DeviceState::Cancelled { job_id: 1, pass_index: 1, submitted_bytes: 40, completion_known: true }, 3, None);
        assert_eq!(cancelled.phase, Phase::Idle, "a cancelled job is no longer in flight");
        assert_eq!(cancelled.ended, Some(Ended::Cancelled));
        assert_eq!(cancelled.pass, Some(PassPosition { index: 1, total: 3 }));
    }

    /// A cancel says the job is over; it does not say the blade stopped. Only the poll
    /// behind `completion_known` can say that, and `actions.cut` is the one value a
    /// caller reads before starting another Job into the same machine — so it is where
    /// the difference has to show. Asserted through `actions`, not the phase: both
    /// cases rest on `Idle`, which is exactly why the phase cannot carry this.
    #[test]
    fn another_cut_is_offered_only_when_the_stop_was_confirmed() {
        let cancelled = |completion_known| {
            status_of(
                &DeviceState::Cancelled { job_id: 1, pass_index: 0, submitted_bytes: 40, completion_known },
                2,
                None,
            )
        };
        assert!(cancelled(true).actions.cut, "a poll saw the machine at rest, so another Job may start");

        let unconfirmed = cancelled(false);
        assert!(!unconfirmed.actions.cut, "nothing saw it stop, so nothing may be started into it");
        assert_eq!(unconfirmed.actions, Actions::default(), "and there is no Job left to cancel, resume or confirm");
        assert_eq!(unconfirmed.ended, Some(Ended::Cancelled), "it still says how the job ended");
    }

    /// A fault is not an ending: it has its own phase and carries the reason, and it
    /// must not leave `ended` reading like a cut that finished — a caller renders both,
    /// so a failed job would otherwise report itself complete as well.
    #[test]
    fn a_fault_is_not_an_ending() {
        // Handed a remembered completion on purpose: the fault has to clear it, not
        // merely fail to add one.
        let failed = status_of(&DeviceState::Error(DeviceError::Timeout), 1, Some(Ended::Completed));
        assert_eq!(failed.phase, Phase::Failed);
        assert_eq!(failed.ended, None);
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
            assert!(status_of(&state, 2, None).is_active(), "{state:?} is mid-flight");
        }
        for state in [DeviceState::Idle, DeviceState::Disconnected] {
            assert!(!status_of(&state, 0, None).is_active(), "{state:?} is not mid-flight");
        }
    }
}
