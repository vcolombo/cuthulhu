// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::time::Instant;

/// A host that is silently unreachable — a dropped SYN, a firewall discarding rather than
/// refusing — must not hang past `CONNECT_TIMEOUT`. `10.255.255.1` is the conventional
/// black-holed address: routable, never answering.
#[test]
fn connect_to_a_silently_unreachable_host_fails_within_the_timeout() {
    let start = Instant::now();
    let err = match HostClient::connect("10.255.255.1:7878", "token", "aa:bb:cc") {
        Ok(_) => panic!("nothing should answer this address"),
        Err(e) => e,
    };
    assert!(
        start.elapsed() < CONNECT_TIMEOUT + Duration::from_secs(2),
        "took {:?}, longer than the timeout allows for: {err}",
        start.elapsed()
    );
}

/// A name that cannot be resolved must fail, and must leave nothing behind that stops
/// the next attempt. The old thread machinery could wedge an address for the process's
/// life if a claim was mis-released; the cancellable stack has no claims, and this test
/// is what notices if any state ever grows back.
///
/// `.invalid` never resolves, by RFC 2606, so this fails on the resolver's own answer
/// rather than on the deadline.
#[test]
fn a_name_that_cannot_be_resolved_does_not_wedge_later_attempts() {
    let unresolvable = "cuthulhu-does-not-exist.invalid:7878";
    for attempt in 1..=2 {
        let err = HostClient::connect(unresolvable, "token", "aa:bb:cc")
            .err()
            .expect("nothing answers a name that does not resolve");
        assert!(
            !err.to_string().contains("still being resolved"),
            "attempt {attempt} was refused by a claim the first attempt never released: {err}"
        );
    }
}

/// The probe shares `connect_by_deadline` with `connect`, so it inherits that bound — this
/// pins that it is actually reached, since the pairing dialog calls the probe first and a
/// mistyped address is the ordinary way to arrive here.
#[test]
fn a_probe_of_a_silently_unreachable_host_fails_within_the_timeout() {
    let start = Instant::now();
    let err = match probe_fingerprint("10.255.255.1:7878", CONNECT_TIMEOUT) {
        Ok(f) => panic!("nothing should answer this address, got {f}"),
        Err(e) => e,
    };
    assert!(
        start.elapsed() < CONNECT_TIMEOUT + Duration::from_secs(2),
        "took {:?}, longer than the timeout allows for: {err}",
        start.elapsed()
    );
}

/// The whole table at once: a new variant fails to compile `Display`'s match, and a rewording
/// that is `ClientError`'s own fails here. These are the strings the desktop shows —
/// `host_error` puts `to_string()` in the message unaltered.
///
/// Two rows compute the sentence they expect from the value underneath instead of restating it,
/// because those two forward a payload whole: writing the forwarded wording out here would
/// compare a literal with itself and pin the wrong layer, when the claim being made is that the
/// payload arrives unaltered. Those two rows therefore follow the payload's own `Display` and
/// stay green when it is reworded, which is the point of them. `Transport`'s row does restate
/// its sentence, and rightly — the wrapping around the payload is this type's own, and the
/// payload it wraps is one of the many the client and its resolver write rather than a sentence
/// from a layer below.
#[test]
fn every_client_failure_has_a_sentence() {
    let fault = crate::check::PassFault::Degenerate(2);
    let device = driver_core::manager::DeviceError::Busy;
    let cases: Vec<(ClientError, String)> = vec![
        (
            ClientError::Refused(Refusal::UnknownDevice("usb:1:4".into())),
            "this host has no cutter called `usb:1:4`".into(),
        ),
        (
            ClientError::Refused(Refusal::MachineMismatch {
                dispatched: "cameo5".into(),
                attached: "puma".into(),
            }),
            "the cut was planned for a `cameo5`, but a `puma` is attached".into(),
        ),
        (
            ClientError::Refused(Refusal::Preflight(crate::check::PassFault::Degenerate(2))),
            fault.to_string(),
        ),
        (
            ClientError::Refused(Refusal::Device(driver_core::manager::DeviceError::Busy)),
            device.to_string(),
        ),
        (
            ClientError::Refused(Refusal::DispatchIdTooLong { max: 128 }),
            "this host will not accept a dispatch id longer than 128 characters".into(),
        ),
        (
            ClientError::Fingerprint { expected: "aa:bb".into(), found: "cc:dd".into() },
            "this host presented a different certificate than the one paired \
             (expected aa:bb, found cc:dd)"
                .into(),
        ),
        (
            ClientError::Unauthorized,
            "this host refused the token; pair again with the one in its `cutd.toml`".into(),
        ),
        (
            ClientError::WrongReply { expected: "Snapshots", found: "Devices" },
            "this host answered with `Devices` where `Snapshots` was expected".into(),
        ),
        (
            ClientError::WrongDispatch,
            "this host answered about a different dispatch than the one it was sent".into(),
        ),
        (
            ClientError::Transport("the host closed the connection".into()),
            "the host could not be reached (the host closed the connection)".into(),
        ),
        (
            ClientError::Protocol {
                host_version: None,
                desktop_version: PROTOCOL_VERSION,
            },
            "This Cut Host is running an older version. Update your Cut Host to connect."
                .into(),
        ),
        (
            ClientError::Protocol {
                host_version: Some(PROTOCOL_VERSION + 1),
                desktop_version: PROTOCOL_VERSION,
            },
            "This version of Cuthulhu is older than the Cut Host. Update Cuthulhu to connect."
                .into(),
        ),
    ];
    for (error, sentence) in cases {
        assert_eq!(error.to_string(), sentence, "{error:?}");
    }
}

/// Every name a mismatch can report. `Refused` and `Unauthorized` are reachable here even
/// though each has a `ClientError` of its own: `call` turns a `Refused` into one only in a
/// reply slot, and the greeting decides `Unauthorized` only there — so either arriving in the
/// other place is a wrong reply and has to be nameable.
#[test]
fn every_response_variant_has_a_name() {
    let named: Vec<(Response, &str)> = vec![
        (Response::Devices(Vec::new()), "Devices"),
        (Response::Snapshots(Vec::new()), "Snapshots"),
        (
            Response::Accepted {
                dispatch_id: DispatchId("d1".into()),
                admitted: Admitted::Started,
            },
            "Accepted",
        ),
        (Response::Ok, "Ok"),
        (Response::Refused(Refusal::UnknownDevice("usb:1:4".into())), "Refused"),
        (Response::Unauthorized, "Unauthorized"),
    ];
    for (response, name) in named {
        assert_eq!(response_name(&response), name, "{response:?}");
    }
}

/// Every distinct reply a verb can expect, against a host that answers a variant it cannot use.
/// Four branches, not seven verbs: `cancel`, `resume`, `confirm_pass_done` and `reconnect` share
/// `expect_ok`, so `cancel` stands for all four. Each branch passes its own `expected` name, so
/// each of the four is a separate hand-written string to pin.
///
/// All four on one connection, which shows that this peer's mismatches leave the stream usable:
/// `call` writes one request and reads frames until the reply, and this fixture answers exactly
/// once per request, so nothing is left over for the next verb to misread. That is a fact about
/// the fixture and not about every off-protocol peer — one that volunteered a second complete
/// `Response` would leave the next call reading a stale one, which is why the desktop drops the
/// connection rather than trusting it (see `with_host_within`).
///
/// The old rendering put the reply's `Debug` in the message under a sentence about
/// reachability, so a single wrong reply printed every field of every cutter the host knew,
/// inside a claim that it could not be reached (#283).
#[test]
fn a_reply_a_verb_cannot_use_is_named_rather_than_rendered() {
    let host = testing::start_host_answering(vec![
        Response::Ok,
        // `Ok` where a device list was asked for.
        Response::Ok,
        // The reply that gave #283 its example: a `Vec<DeviceInfo>` in the wrong slot.
        Response::Devices(vec![DeviceInfo {
            instance_id: "usb:1:4".into(),
            machine_id: "cameo5".into(),
            transport: driver_core::TransportKind::Usb { locator: "1:4".into() },
            candidate: false,
            host: None,
        }]),
        Response::Ok,
        Response::Devices(Vec::new()),
    ]);
    let client = HostClient::connect(&host.addr, "token", &host.fingerprint)
        .expect("this host greets a client normally; only its replies are wrong");

    assert_eq!(
        client.devices().unwrap_err().to_string(),
        "this host answered with `Ok` where `Devices` was expected"
    );

    let listed = client.snapshots().unwrap_err().to_string();
    assert_eq!(listed, "this host answered with `Devices` where `Snapshots` was expected");
    assert!(!listed.contains("DeviceInfo"), "the reply's fields reached the operator: {listed}");
    assert!(
        !listed.contains("could not be reached"),
        "a host that answered was reported unreachable: {listed}"
    );

    assert_eq!(
        client
            .dispatch(&DispatchId("d1".into()), "usb:1:4", "cameo5", Vec::new())
            .unwrap_err()
            .to_string(),
        "this host answered with `Ok` where `Accepted` was expected"
    );
    assert_eq!(
        client.cancel("usb:1:4").unwrap_err().to_string(),
        "this host answered with `Devices` where `Ok` was expected"
    );
}

/// The one reply of the *right* variant that is still not this request's answer. `Accepted`
/// carries the dispatch it is about, and an `Admitted` read out of one that names another Job
/// reports a start this host was never asked for — while the entry a real retry needs is
/// dropped as settled (#285).
///
/// The mismatch is refused without either id reaching the sentence, which is what keeps a
/// peer's string out of an operator's message (#283); and the connection is left usable, so the
/// second dispatch proves the check refuses a mismatch and nothing else.
#[test]
fn an_accepted_naming_another_dispatch_is_not_this_dispatch_s_answer() {
    let host = testing::start_host_answering(vec![
        Response::Ok,
        Response::Accepted {
            dispatch_id: DispatchId("some-other-job".into()),
            admitted: Admitted::Started,
        },
        Response::Accepted {
            dispatch_id: DispatchId("d1".into()),
            admitted: Admitted::AlreadyAccepted,
        },
    ]);
    let client = HostClient::connect(&host.addr, "token", &host.fingerprint)
        .expect("this host greets a client normally; only its replies are wrong");

    let refused = client
        .dispatch(&DispatchId("d1".into()), "usb:1:4", "cameo5", Vec::new())
        .unwrap_err()
        .to_string();
    assert_eq!(
        refused,
        "this host answered about a different dispatch than the one it was sent"
    );
    assert!(!refused.contains("some-other-job"), "the peer's own id reached the operator: {refused}");

    assert_eq!(
        client
            .dispatch(&DispatchId("d1".into()), "usb:1:4", "cameo5", Vec::new())
            .expect("the id this dispatch sent is the id this reply names"),
        Admitted::AlreadyAccepted
    );
}

/// `connect`'s failure. `HostClient` is deliberately not `Debug` — it holds a TLS stream — so
/// `expect_err` cannot be used on its result.
fn connect_failure(host: &testing::MisbehavingHost) -> ClientError {
    match HostClient::connect(&host.addr, "token", &host.fingerprint) {
        Ok(_) => panic!("this host's greeting cannot open a session"),
        Err(e) => e,
    }
}

/// The greeting is its own construction site: it is answered before any `Request` is written,
/// so it cannot go through `call` and had a second hand-written payload of its own.
#[test]
fn a_greeting_that_is_not_ok_is_named_rather_than_rendered() {
    let host = testing::start_host_answering(vec![Response::Snapshots(Vec::new())]);
    assert_eq!(
        connect_failure(&host).to_string(),
        "this host answered with `Snapshots` where `Ok` was expected"
    );
}

/// A refused token is also "not `Ok`", and the greeting decides it before it reaches a
/// mismatch at all — reading it as a wrong reply would lose the one refusal on this path that
/// tells the operator what to do about it (#112).
#[test]
fn a_refused_token_is_still_a_refused_token_rather_than_a_wrong_reply() {
    let host = testing::start_host_answering(vec![Response::Unauthorized]);
    assert_eq!(
        connect_failure(&host).to_string(),
        "this host refused the token; pair again with the one in its `cutd.toml`"
    );
}

/// The host named a higher version than this build speaks. The sentence tells the operator
/// to update Cuthulhu, and no session is opened to send a token into.
#[test]
fn a_newer_host_is_named_as_a_desktop_to_update() {
    let host = testing::start_host_at_version(PROTOCOL_VERSION + 1, vec![Response::Ok]);
    match connect_failure(&host) {
        ClientError::Protocol {
            host_version,
            desktop_version,
        } => {
            assert_eq!(host_version, Some(PROTOCOL_VERSION + 1));
            assert_eq!(desktop_version, PROTOCOL_VERSION);
        }
        other => panic!("expected a protocol mismatch, got {other:?}"),
    }
    assert_eq!(
        connect_failure(&host).to_string(),
        "This version of Cuthulhu is older than the Cut Host. Update Cuthulhu to connect."
    );
}

/// The other direction: the host speaks a version this build has moved past.
#[test]
fn an_older_host_is_named_as_a_cut_host_to_update() {
    let host =
        testing::start_host_at_version(PROTOCOL_VERSION.saturating_sub(1), vec![Response::Ok]);
    // PROTOCOL_VERSION is 1, so saturating_sub is 0, which the host reports as what it speaks.
    assert_eq!(
        connect_failure(&host).to_string(),
        "This Cut Host is running an older version. Update your Cut Host to connect."
    );
}

/// The daemon this change replaces. It does not answer the offer, and the failure is still
/// the older host — a dropped connection here used to arrive as "could not be reached".
#[test]
fn an_unversioned_host_is_the_older_side() {
    let host = testing::start_unversioned_host();
    match connect_failure(&host) {
        ClientError::Protocol {
            host_version: None, ..
        } => {}
        other => panic!("expected a host that named no version, got {other:?}"),
    }
    assert_eq!(
        connect_failure(&host).to_string(),
        "This Cut Host is running an older version. Update your Cut Host to connect."
    );
}

/// A version reply is capped before its body is read. A length that would be legal for a
/// dispatch must not hold the connect open for the body, and must not be decoded as one.
#[test]
fn an_oversized_version_reply_is_refused_before_its_body() {
    let mut header = Vec::new();
    header.extend_from_slice(&(crate::frame::DEFAULT_MAX_FRAME as u32).to_be_bytes());
    let host = testing::start_host_writing_version_reply(header);
    let started = Instant::now();
    let err = connect_failure(&host);
    let sentence = err.to_string();
    assert!(
        matches!(
            err,
            ClientError::Protocol {
                host_version: None,
                ..
            }
        ),
        "got {sentence}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "waited for a body {:?}",
        started.elapsed()
    );
    assert_eq!(
        sentence,
        "This Cut Host is running an older version. Update your Cut Host to connect."
    );
}
