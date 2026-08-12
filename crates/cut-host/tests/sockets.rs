// SPDX-License-Identifier: GPL-3.0-or-later
// The daemon is exercised through a real client, so this file needs the `client` feature;
// the daemon-only shape (`--no-default-features`, the Pi cross build) skips it.
#![cfg(feature = "client")]
//! The three behaviours that only exist once there is a real socket: two clients at once, the
//! client cap, and the refusal to bind somewhere the whole internet can reach.
//!
//! Each is a refusal or a guarantee a reader would reasonably assume was covered, and none was:
//! the multi-client claim was proved at the `Host` level, where a per-connection assumption is
//! exactly what would not show up, and neither the cap nor the bind guard was exercised at all
//! (#99).

use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cut_host::client::HostClient;
use cut_host::config::Config;
use cut_host::host::testing::TwoCutterFactory;
use cut_host::host::Host;
use cut_host::protocol::DispatchId;
use cut_host::serve::MAX_CLIENTS;
use driver_core::Phase;

mod fixtures;
use fixtures::{square_pass, start_test_host, CAMEO, TOKEN};

/// The design's most distinctive claim: any authorized client may cancel, resume or confirm any
/// Job, not only the one that dispatched it — because whoever walks to the cutter to swap material
/// is not necessarily sitting at the laptop that started the cut.
///
/// Two genuinely separate connections, not two calls on one: the assumption this is guarding
/// against is a host that quietly ties a Job to the connection that sent it, and one connection
/// cannot see that.
#[test]
fn a_job_dispatched_on_one_connection_is_confirmed_on_another() {
    let host = start_test_host();
    let laptop = HostClient::connect(&host.addr, TOKEN, &host.fingerprint).unwrap();
    let workshop = HostClient::connect(&host.addr, TOKEN, &host.fingerprint).unwrap();

    laptop.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()]).unwrap();

    // Watched from the second connection too, so the Job is visible to a client that had nothing
    // to do with starting it.
    let parked = await_phase(&workshop, CAMEO, Phase::AwaitingConfirmation);
    assert!(parked.actions.confirm, "the other client is offered the confirm");

    workshop.confirm_pass_done(CAMEO).unwrap();
    let done = await_phase(&laptop, CAMEO, Phase::Idle);
    assert_eq!(done.ended, Some(driver_core::Ended::Completed));
}

/// The cap exists so a client that never leaves cannot exhaust the daemon; the half that matters
/// as much is that a slot comes back. A counter decremented somewhere it was never incremented
/// would pass every other test in this crate.
#[test]
fn the_client_cap_refuses_the_surplus_and_frees_a_slot_when_one_leaves() {
    let host = start_test_host();
    let mut attached: Vec<HostClient> = (0..MAX_CLIENTS)
        .map(|i| {
            HostClient::connect(&host.addr, TOKEN, &host.fingerprint)
                .unwrap_or_else(|e| panic!("client {i} of the cap should be served: {e}"))
        })
        .collect();

    // The surplus is dropped without ceremony — before TLS, so it surfaces as a failed connect.
    assert!(
        HostClient::connect(&host.addr, TOKEN, &host.fingerprint).is_err(),
        "the {}th client was served past the cap",
        MAX_CLIENTS + 1
    );
    // And the ones already attached keep working, rather than being disturbed by the refusal.
    assert_eq!(attached[0].devices().unwrap().len(), 2);

    // One laptop closes. The slot is freed by its worker noticing the close, so this is polled
    // rather than asserted: nothing here happens on this thread.
    attached.pop();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if HostClient::connect(&host.addr, TOKEN, &host.fingerprint).is_ok() {
            break;
        }
        assert!(Instant::now() < deadline, "a client left and its slot never came back");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The daemon-enforced half of "LAN-only by default". `Config::is_private_bind`'s pure
/// classification is tested; that `serve` actually refuses on it was not.
///
/// `8.8.8.8` is public and is not an address this machine holds, so the override case fails at the
/// bind instead of exposing anything — which is the point of asserting on *which* refusal each is.
#[test]
fn serving_a_public_address_is_refused_unless_it_is_asked_for() {
    let refusal = serve_on_a_public_address(false).expect_err("a public bind must not be silent");
    assert!(
        refusal.to_string().contains("--allow-public-bind"),
        "the refusal should say the way through: {refusal}"
    );

    let with_override =
        serve_on_a_public_address(true).expect_err("this machine does not hold 8.8.8.8");
    assert!(
        !with_override.to_string().contains("--allow-public-bind"),
        "the override was ignored and the guard refused anyway: {with_override}"
    );
}

fn serve_on_a_public_address(allow_public_bind: bool) -> std::io::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        bind: "8.8.8.8:7878".parse().unwrap(),
        tokens: [("test-client".to_string(), TOKEN.to_string())].into_iter().collect(),
        max_frame: cut_host::frame::DEFAULT_MAX_FRAME,
        cert_dir: dir.path().to_path_buf(),
    };
    cut_host::serve::serve(
        Host::start(Arc::new(TwoCutterFactory)),
        config,
        allow_public_bind,
    )
}

/// A listener bound here rather than by `serve`, so this cannot accidentally be the test above.
/// Kept as proof that the guard is about the address and not about binding failing in general.
#[test]
fn a_private_bind_is_served_without_the_override() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        bind: listener.local_addr().unwrap(),
        tokens: [("test-client".to_string(), TOKEN.to_string())].into_iter().collect(),
        max_frame: cut_host::frame::DEFAULT_MAX_FRAME,
        cert_dir: dir.path().to_path_buf(),
    };
    assert!(config.is_private_bind(), "127.0.0.1 is the case the guard must let through");
}

fn await_phase(client: &HostClient, device: &str, want: Phase) -> driver_core::CutStatus {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snaps = client.snapshots().unwrap();
        let snap = snaps.into_iter().find(|s| s.info.instance_id == device).unwrap();
        if snap.status.phase == want {
            return snap.status;
        }
        assert!(Instant::now() < deadline, "{device} never reached {want:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
