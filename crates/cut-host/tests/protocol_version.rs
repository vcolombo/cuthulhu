// SPDX-License-Identifier: GPL-3.0-or-later
// The daemon is exercised through a real client, so this file needs the `client` feature;
// the daemon-only shape (`--no-default-features`, the Pi cross build) skips it.
#![cfg(feature = "client")]
//! A connection that has not agreed `PROTOCOL_VERSION` admits nothing.
//!
//! Mixed versions, a frame that is not a version, a replay of the unversioned session, and a
//! peer that leaves mid-agreement. Each one is a way to get bytes that used to be a `Request`
//! onto the socket before the host should read one. None of them may leave a Job, a dispatch
//! id, or a cutter claimed.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use cut_host::client::testing::RawSession;
use cut_host::client::HostClient;
use cut_host::config::Config;
use cut_host::frame::FrameError;
use cut_host::host::testing::TwoCutterFactory;
use cut_host::host::Host;
use cut_host::protocol::{DispatchId, Request, VersionReply, PROTOCOL_VERSION, VERSION_FRAME_MAX};
use cut_host::serve::{fingerprint_of_cert_dir, serve_on};
use driver_core::Phase;

// The shared fixture also builds a host this file does not use. This binary keeps its own
// `Arc<Host>`, which is what the admission assertions read.
#[allow(dead_code)]
mod fixtures;
use fixtures::{square_pass, CAMEO, TOKEN};

struct Running {
    addr: String,
    fingerprint: String,
    host: Arc<Host>,
    _dir: tempfile::TempDir,
}

fn start() -> Running {
    let dir = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let config = Config {
        bind: listener.local_addr().unwrap(),
        tokens: [("test-client".to_string(), TOKEN.to_string())]
            .into_iter()
            .collect(),
        max_frame: cut_host::frame::DEFAULT_MAX_FRAME,
        cert_dir: dir.path().to_path_buf(),
    };
    let fingerprint = fingerprint_of_cert_dir(&config.cert_dir).unwrap();
    let host = Host::start(Arc::new(TwoCutterFactory));
    let serve_host = host.clone();
    thread::spawn(move || {
        let _ = serve_on(listener, serve_host, config);
    });
    Running {
        addr,
        fingerprint,
        host,
        _dir: dir,
    }
}

/// Long enough for a mock cut to park if one was admitted, short enough that a test which
/// wrongly waits on the body timeout fails for a different reason than this window.
fn assert_nothing_admitted(host: &Host) {
    let deadline = Instant::now() + Duration::from_millis(300);
    loop {
        assert!(
            host.claims().is_empty(),
            "{} cutter(s) still claimed",
            host.claims().len()
        );
        for snap in host.snapshots() {
            assert!(!snap.claimed, "{} claimed", snap.info.instance_id);
            assert!(
                snap.job_id.is_none(),
                "{} recorded a job",
                snap.info.instance_id
            );
            assert_eq!(snap.status.phase, Phase::Idle, "{}", snap.info.instance_id);
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn open(running: &Running) -> RawSession {
    RawSession::open(&running.addr, &running.fingerprint).expect("tls to the test host")
}

const BUDGET: Duration = Duration::from_secs(5);

#[test]
fn a_different_version_is_refused_by_name_and_admits_nothing() {
    let running = start();
    let mut raw = open(&running);
    raw.send(&(PROTOCOL_VERSION + 1), BUDGET).unwrap();
    let reply: VersionReply = raw.recv(VERSION_FRAME_MAX, BUDGET).unwrap();
    assert_eq!(
        reply,
        VersionReply::Unsupported {
            offered: PROTOCOL_VERSION + 1,
            supported: PROTOCOL_VERSION
        }
    );
    // The refusal is the last thing on the connection.
    let next: Result<VersionReply, FrameError> = raw.recv(VERSION_FRAME_MAX, BUDGET);
    assert!(
        matches!(next, Err(FrameError::Eof) | Err(FrameError::Io(_))),
        "{next:?}"
    );
    drop(raw);

    assert_nothing_admitted(&running.host);
    // Not wedged: the same cutter still takes a Job from a peer that speaks this version.
    let client = HostClient::connect(&running.addr, TOKEN, &running.fingerprint).unwrap();
    client
        .dispatch(
            &DispatchId("after-mismatch".into()),
            CAMEO,
            "cameo5",
            vec![square_pass()],
        )
        .unwrap();
}

#[test]
fn a_frame_that_is_not_a_version_admits_nothing() {
    let running = start();
    let mut raw = open(&running);
    // A dispatch-shaped object, small enough to fit the version cap, so the refusal is the
    // shape and not the length. Decoded as a `u32` it is not one; decoded as a `Request` it
    // would be how this test failed.
    raw.send(&serde_json::json!({"Dispatch": {"device": CAMEO}}), BUDGET)
        .unwrap();
    let reply: VersionReply = raw.recv(VERSION_FRAME_MAX, BUDGET).unwrap();
    assert_eq!(reply, VersionReply::Unreadable);
    drop(raw);
    assert_nothing_admitted(&running.host);
}

/// The unversioned session, replayed: a token frame, then a real dispatch. The dispatch is the
/// bytes that would cut. They are on the socket, and they must not be read as a `Request`.
#[test]
fn replaying_an_unversioned_session_admits_no_job() {
    let running = start();
    let mut raw = open(&running);
    raw.send(&TOKEN.to_string(), BUDGET).unwrap();
    let dispatch = Request::Dispatch {
        dispatch_id: DispatchId("replayed-1".into()),
        device: CAMEO.into(),
        machine_id: "cameo5".into(),
        passes: vec![square_pass()],
    };
    // The host may already have closed after the token frame. A write error here is the
    // refusal working; a write that lands is the replay this test exists for.
    let _ = raw.send(&dispatch, BUDGET);
    let reply: VersionReply = raw.recv(VERSION_FRAME_MAX, BUDGET).unwrap();
    assert_eq!(reply, VersionReply::Unreadable);
    drop(raw);
    assert_nothing_admitted(&running.host);
}

#[test]
fn a_peer_that_disconnects_after_agreeing_a_version_admits_nothing() {
    let running = start();
    let mut raw = open(&running);
    raw.send(&PROTOCOL_VERSION, BUDGET).unwrap();
    let reply: VersionReply = raw.recv(VERSION_FRAME_MAX, BUDGET).unwrap();
    assert_eq!(
        reply,
        VersionReply::Agreed {
            version: PROTOCOL_VERSION
        }
    );
    // Before the token, so the agreement is not a session.
    drop(raw);
    assert_nothing_admitted(&running.host);
}

#[test]
fn a_peer_that_disconnects_before_offering_a_version_admits_nothing() {
    let running = start();
    let raw = open(&running);
    drop(raw);
    assert_nothing_admitted(&running.host);
}
