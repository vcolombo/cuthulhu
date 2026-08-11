// SPDX-License-Identifier: GPL-3.0-or-later

//! Refusing to exit while a blade is moving.
//!
//! The desktop already guards its window-close this way, and the Pi is where it
//! matters more: a Cut Host exists so a long Job can outlive the client that sent
//! it, so `systemctl stop`, a package upgrade or a plain `kill` is the one thing
//! left that can still abandon a cut nobody is watching.
//!
//! The guard defers, it does not refuse forever. A signalled daemon keeps asking
//! until the cut ends and then exits on its own, so `systemctl stop` during a Job
//! waits for the Job rather than failing — see `docs/cut-host.md` for the
//! `TimeoutStopSec` that has to be long enough to let it.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::host::Host;

/// Signals seen so far. Written only by the signal handler, which may do nothing
/// else: a `fetch_add` on a lock-free atomic is about the whole of what is
/// async-signal-safe, so every decision it drives is made on an ordinary thread.
static SIGNALS: AtomicU32 = AtomicU32::new(0);

/// Signals it takes to exit past an active cut. The first refuses, the second is
/// the operator saying they meant it. There is no flag for this because the force
/// has to be reachable by whoever is already holding the signal — someone at
/// `systemctl stop` cannot go back and add an argument to the unit file.
const FORCE: u32 = 2;

/// How often the watcher looks at the flag.
//
// ponytail: a poll rather than a self-pipe, because `pause()` sleeps through a
// signal that lands between the check and the call — which would leave the force
// needing a third signal, from an operator already told two would do. Write a
// byte to a pipe from the handler if 200ms ever costs anything.
const POLL: Duration = Duration::from_millis(200);

/// Whether a process signalled `signals` times may exit now.
///
/// `is_any_cut_active` is built on `driver-core`'s own predicate, the same one the
/// desktop's window-close guard asks, rather than a second reading of the phases —
/// and it also covers a dispatch admitted and not yet inside `manager.cut`, which
/// that predicate cannot see. Without that half, a signal landing in the window
/// exits past a Job whose client was already told `Accepted`.
pub fn may_exit(host: &Host, signals: u32) -> bool {
    signals >= FORCE || !host.is_any_cut_active()
}

extern "C" fn note_signal(_: libc::c_int) {
    SIGNALS.fetch_add(1, Ordering::SeqCst);
}

/// Install the handler and start the thread that acts on it, then return so the
/// caller can go on to serve.
pub fn guard(host: Arc<Host>) {
    // SAFETY: `note_signal` touches one lock-free atomic and nothing else, which
    // is what a handler is permitted to do. Replacing the default disposition for
    // these two is the entire point — SIGTERM's default is what abandons the cut.
    unsafe {
        let handler = note_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGINT, handler);
    }
    eprintln!("cut host: a stop while a cut is running waits for it; signal twice to stop anyway");
    thread::spawn(move || watch(&host));
}

fn watch(host: &Host) -> ! {
    let mut reported = 0;
    loop {
        thread::sleep(POLL);
        let signals = SIGNALS.load(Ordering::SeqCst);
        if signals == 0 {
            continue;
        }
        if may_exit(host, signals) {
            if host.is_any_cut_active() {
                eprintln!("cut host: signalled twice — exiting and abandoning the cut in progress");
            }
            eprintln!("cut host: shutting down");
            std::process::exit(0);
        }
        // Only when the count changes: the answer is re-asked every `POLL` until
        // the cut ends, and a line per tick would bury the cut's own progress.
        if signals != reported {
            report(host);
            reported = signals;
        }
    }
}

/// What is being waited on, and where to watch it — an operator who has just been
/// refused a stop needs to know which cutter, on which Job, and how long is left.
fn report(host: &Host) {
    eprintln!("cut host: not exiting yet, a cut is still running:");
    for snapshot in host.snapshots().iter().filter(|s| s.status.is_active()) {
        // `None` is the window between a dispatch being admitted and its worker
        // recording the id, not a Job without one.
        let job = match snapshot.job_id {
            Some(id) => format!("job {id}"),
            None => "job starting".to_string(),
        };
        eprintln!(
            "cut host:   {} is {:?} ({job})",
            snapshot.info.instance_id, snapshot.status.phase
        );
    }
    eprintln!(
        "cut host: it will exit when the cut ends. Follow it with \
         `journalctl -u cuthulhu-cutd -f`, cancel it from the desktop, or signal again to stop now."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testing::{TwoCutterFactory, CAMEO};
    use crate::protocol::DispatchId;
    use driver_core::manager::CutPass;
    use driver_core::{Job, Settings};
    use geometry::Point;

    fn square_pass() -> CutPass {
        CutPass {
            job: Job {
                polylines: vec![vec![
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 10.0, y: 0.0 },
                    Point { x: 10.0, y: 10.0 },
                    Point { x: 0.0, y: 0.0 },
                ]],
                settings: Settings::default(),
            },
        }
    }

    /// The worker is a thread of its own, so a bare assertion after `dispatch`
    /// would race it.
    fn wait_for(host: &Host, device: &str, want: driver_core::Phase) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let phase = host.slot(device).unwrap().manager.status().phase;
            if phase == want {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "{device} sat at {phase:?}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_daemon_with_nothing_cutting_exits_on_the_first_signal() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        assert!(may_exit(&host, 1));
    }

    /// The defect this exists for: a signal arriving mid-Job must not end the
    /// process.
    #[test]
    fn a_daemon_with_a_cut_in_flight_refuses_the_first_signal() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);

        assert!(!may_exit(&host, 1), "a signal must not end a Job the daemon is running");
    }

    /// The way through, so an operator who means it is not held by their own Pi.
    #[test]
    fn a_second_signal_exits_past_a_cut_in_flight() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);

        assert!(may_exit(&host, FORCE));
    }

    /// A stop is deferred, not refused: the daemon keeps asking, and the answer
    /// changes on its own once the Job ends. Without this a `systemctl stop`
    /// issued mid-cut would hang until systemd's SIGKILL rather than completing.
    #[test]
    fn a_refused_signal_becomes_an_exit_once_the_cut_ends() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::AwaitingConfirmation);
        assert!(!may_exit(&host, 1));

        host.confirm_pass_done(CAMEO).unwrap();
        wait_for(&host, CAMEO, driver_core::Phase::Idle);
        assert!(may_exit(&host, 1), "the same one signal exits once nothing is cutting");
    }

    /// One cutter busy is enough — the daemon owns several and a stop must clear
    /// all of them.
    #[test]
    fn a_cut_on_any_cutter_holds_the_daemon() {
        use crate::host::testing::PUMA;
        let host = Host::start(Arc::new(TwoCutterFactory));
        host.dispatch(DispatchId("d-1".into()), PUMA, "puma", vec![square_pass()]).unwrap();
        wait_for(&host, PUMA, driver_core::Phase::AwaitingConfirmation);

        assert_eq!(host.slot(CAMEO).unwrap().manager.status().phase, driver_core::Phase::Idle);
        assert!(!may_exit(&host, 1), "an idle cutter alongside a busy one is not permission to exit");
    }
}
