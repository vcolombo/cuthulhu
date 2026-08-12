// SPDX-License-Identifier: GPL-3.0-or-later
// The daemon is exercised through a real client, so this file needs the `client` feature;
// the daemon-only shape (`--no-default-features`, the Pi cross build) skips it.
#![cfg(feature = "client")]
//! A whole cut, from a client through TLS to a Cut Host and back, with no
//! hardware. This is the test that says the phase works.

use std::time::{Duration, Instant};

use cut_host::client::{ClientError, HostClient};
use cut_host::protocol::DispatchId;
use driver_core::manager::CutPass;
use driver_core::{Job, Phase, Settings};
use geometry::Point;

mod fixtures;
use fixtures::{start_test_host, square_pass, CAMEO, TOKEN};

#[test]
fn a_client_lists_the_cutters_on_a_host() {
    let host = start_test_host();
    let client = HostClient::connect(&host.addr, TOKEN, &host.fingerprint).unwrap();
    let devices = client.devices().unwrap();
    assert_eq!(devices.len(), 2);
}

/// And refused *by name*. A daemon that closed the connection instead reached the operator as
/// "the host could not be reached", which is what an asleep Pi looks like — one calls for
/// re-pairing with the token from `cutd.toml`, the other for waiting (#112).
#[test]
fn a_bad_token_is_refused_as_a_rejected_token_rather_than_an_unreachable_host() {
    let host = start_test_host();
    match HostClient::connect(&host.addr, "not-the-token", &host.fingerprint) {
        Err(ClientError::Unauthorized) => {}
        Err(e) => panic!("expected the token to be named as the problem, got {e:?}"),
        Ok(_) => panic!("a token matching no client was accepted"),
    }
}

#[test]
fn a_certificate_that_is_not_the_pinned_one_is_refused() {
    let host = start_test_host();
    let wrong = "00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
                 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff";
    // `HostClient` is deliberately not `Debug` — it owns a live TLS session and a
    // token — so the arms name the outcome rather than printing the value.
    match HostClient::connect(&host.addr, TOKEN, wrong) {
        Err(ClientError::Fingerprint { .. }) => {}
        Err(e) => panic!("expected a fingerprint refusal, got {e:?}"),
        Ok(_) => panic!("a certificate that was not the pinned one was accepted"),
    }
}

/// The whole point of the phase: dispatch, watch it park, confirm it, watch it
/// finish — all across a socket.
#[test]
fn a_cut_runs_from_dispatch_to_completion_over_the_wire() {
    let host = start_test_host();
    let client = HostClient::connect(&host.addr, TOKEN, &host.fingerprint).unwrap();

    client
        .dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()])
        .unwrap();

    let parked = await_phase(&client, CAMEO, Phase::AwaitingConfirmation);
    assert!(parked.actions.confirm, "the client renders its controls from actions");

    client.confirm_pass_done(CAMEO).unwrap();
    let done = await_phase(&client, CAMEO, Phase::Idle);
    assert_eq!(done.ended, Some(driver_core::Ended::Completed));
}

/// A client that goes away mid-Job must not take the Job with it.
#[test]
fn a_job_outlives_the_client_that_started_it() {
    let host = start_test_host();
    {
        let client = HostClient::connect(&host.addr, TOKEN, &host.fingerprint).unwrap();
        client
            .dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![square_pass()])
            .unwrap();
        await_job(&client, CAMEO);
    } // the laptop closes

    let reattached = HostClient::connect(&host.addr, TOKEN, &host.fingerprint).unwrap();
    let snap = reattached.snapshots().unwrap().into_iter().find(|s| s.info.instance_id == CAMEO).unwrap();
    assert_eq!(snap.status.phase, Phase::AwaitingConfirmation, "the Job carried on without its client");
    assert!(snap.job_id.is_some(), "and a new client can tell which Job it is");

    reattached.confirm_pass_done(CAMEO).unwrap();
    assert_eq!(await_phase(&reattached, CAMEO, Phase::Idle).ended, Some(driver_core::Ended::Completed));
}

#[test]
fn a_refusal_reaches_the_client_as_its_sentence() {
    let host = start_test_host();
    let client = HostClient::connect(&host.addr, TOKEN, &host.fingerprint).unwrap();
    let off_the_bed = CutPass {
        job: Job {
            polylines: vec![vec![Point { x: 0.0, y: 0.0 }, Point { x: 400.0, y: 0.0 }]],
            settings: Settings::default(),
        },
    };
    match client.dispatch(DispatchId("d-1".into()), CAMEO, "cameo5", vec![off_the_bed]) {
        Err(ClientError::Refused(cut_host::protocol::Refusal::Preflight(fault))) => {
            let message = fault.to_string();
            assert!(message.contains("300 x 200"), "got: {message}");
        }
        other => panic!("expected a Preflight refusal, got {other:?}"),
    }
}

#[test]
fn a_pair_check_lists_the_cutters_and_leaves_no_connection_behind() {
    let host = start_test_host();
    let devices = HostClient::pair_check(&host.addr, TOKEN, &host.fingerprint).unwrap();
    assert_eq!(devices.len(), 2, "the test host has two cutters");
    assert!(devices.iter().all(|d| d.host.is_none()), "a daemon does not know its own id");
}

#[test]
fn a_pair_check_with_the_wrong_token_fails_before_anything_is_saved() {
    let host = start_test_host();
    assert!(HostClient::pair_check(&host.addr, "not-the-token", &host.fingerprint).is_err());
}

#[test]
fn a_pair_check_against_a_different_certificate_is_refused() {
    let host = start_test_host();
    let wrong = "00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
                 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff";
    match HostClient::pair_check(&host.addr, TOKEN, wrong) {
        Err(ClientError::Fingerprint { .. }) => {}
        Err(e) => panic!("expected a fingerprint refusal, got {e:?}"),
        Ok(_) => panic!("a certificate that was not the pinned one was accepted"),
    }
}

/// What a first pairing does before it has anything to pin, and the equality is the point: a
/// probe that returned a fingerprint the pinning path would not accept would be worse than no
/// probe, so this asserts both that it matches what the host's cert directory reports *and* that
/// pinning it actually connects.
///
/// It is also the assertion that a probe does not authenticate. `probe_fingerprint` takes no
/// token — there is no credential in its signature to present — so succeeding here against a host
/// that would refuse an unknown one is the observable half of that guarantee; the signature is
/// the other half.
#[test]
fn a_probe_learns_the_fingerprint_the_pinned_path_accepts_without_presenting_a_token() {
    let host = start_test_host();
    let probed = cut_host::client::probe_fingerprint(&host.addr, Duration::from_secs(5)).unwrap();
    assert_eq!(probed, host.fingerprint);

    let devices = HostClient::pair_check(&host.addr, TOKEN, &probed).unwrap();
    assert_eq!(devices.len(), 2, "the probed fingerprint is the one the pinned path accepts");
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

/// The socket-side of `host::testing::wait_for_job`: the pause phase is published before the
/// dispatch thread records the Job's id, so a snapshot taken at first sight of the phase can
/// honestly answer `job_id: None` (#129). Waited out here so later snapshots name the Job.
fn await_job(client: &HostClient, device: &str) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snaps = client.snapshots().unwrap();
        let snap = snaps.into_iter().find(|s| s.info.instance_id == device).unwrap();
        if let Some(job_id) = snap.job_id {
            return job_id;
        }
        assert!(Instant::now() < deadline, "{device}'s dispatch never landed a Job");
        std::thread::sleep(Duration::from_millis(20));
    }
}
