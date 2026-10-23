// SPDX-License-Identifier: GPL-3.0-or-later
//! What a desktop holds to reach a Cut Host.
//!
//! One TLS connection carries every cutter on the host: requests out, events in.
//! The certificate is pinned by fingerprint rather than validated by an authority,
//! because a Pi on a home network has no name an authority would sign.

use std::io;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use driver_core::manager::CutPass;
use driver_core::DeviceInfo;

use crate::frame::{read_frame, write_frame, FrameError};
use crate::protocol::{
    Admitted, DeviceSnapshot, DispatchId, Event, Refusal, Request, Response, VersionReply,
    PROTOCOL_VERSION, VERSION_FRAME_MAX,
};

#[derive(Debug)]
pub enum ClientError {
    Refused(Refusal),
    Fingerprint { expected: String, found: String },
    /// The host answered, and refused the token. Distinct from `Transport` because the operator's
    /// next move is: re-pair with the token from the Pi's `cutd.toml` — not wait for a host that
    /// is merely asleep (#112).
    Unauthorized,
    /// A reply that arrived and the request could not use, named rather than rendered. Distinct
    /// from `Transport` because a host that answered was reached — the answer is the proof — and
    /// the desktop's `host_unreachable` sent the operator to check a network that had just carried
    /// it. Nothing about that network or the pairing is the thing to change (#283).
    WrongReply { expected: &'static str, found: &'static str },
    /// A reply of the right variant about the wrong dispatch. Distinct from `WrongReply` because
    /// nothing about the variant is wrong — naming it would say `Accepted` where `Accepted` was
    /// expected — and the `Admitted` inside it is an answer about some other Job, so believing it
    /// reports a start this host was never asked for (#285).
    ///
    /// Carries neither id, and deliberately. The sent one is the desktop's own and tells an
    /// operator nothing they can act on; the echoed one is a string from a peer that this client
    /// never bounds — the 128-character cap is the *host's*, applied to what it receives — and
    /// putting a peer's payload in front of an operator is the defect #283 was.
    WrongDispatch,
    /// The two sides did not agree `PROTOCOL_VERSION`.
    ///
    /// `host_version` is what the host said it speaks, or `None` when it never got as far as a
    /// number — it closed, or the offer was not a version. That is an older host: the
    /// unversioned protocol is the one this replaced, and it ends the connection without a
    /// reply. `desktop_version` is what this build offered. The sentence says which side to
    /// update; the numbers stay here so a caller can, and so a peer's payload cannot.
    Protocol {
        host_version: Option<u32>,
        desktop_version: u32,
    },
    Transport(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Refused(Refusal::UnknownDevice(d)) =>
                write!(f, "this host has no cutter called `{d}`"),
            ClientError::Refused(Refusal::MachineMismatch { dispatched, attached }) =>
                write!(f, "the cut was planned for a `{dispatched}`, but a `{attached}` is attached"),
            ClientError::Refused(Refusal::Preflight(fault)) => write!(f, "{fault}"),
            // Forwarded, not prefixed: a `DeviceError` writes a whole sentence of its own, and
            // "the cutter refused: " in front of one read twice — the same call `CutError` makes
            // for `Preflight` above (#90). Which cutter it was is the caller's context, not this
            // string's: the desktop asked one host about one device.
            ClientError::Refused(Refusal::Device(e)) => write!(f, "{e}"),
            ClientError::Refused(Refusal::DispatchIdTooLong { max }) =>
                write!(f, "this host will not accept a dispatch id longer than {max} characters"),
            ClientError::Fingerprint { expected, found } =>
                write!(f, "this host presented a different certificate than the one paired \
                           (expected {expected}, found {found})"),
            ClientError::Unauthorized =>
                write!(f, "this host refused the token; pair again with the one in its `cutd.toml`"),
            ClientError::WrongReply { expected, found } =>
                write!(f, "this host answered with `{found}` where `{expected}` was expected"),
            ClientError::WrongDispatch =>
                write!(f, "this host answered about a different dispatch than the one it was sent"),
            ClientError::Protocol { host_version, desktop_version } => match host_version {
                Some(host) if host > desktop_version => write!(
                    f,
                    "This version of Cuthulhu is older than the Cut Host. Update Cuthulhu to connect."
                ),
                // `None` is a host that never named a version. Equal is a host that refused the
                // version it claims to speak. Both are the host to replace: this build is current
                // for the version it offered, and the peer is not speaking it.
                _ => write!(
                    f,
                    "This Cut Host is running an older version. Update your Cut Host to connect."
                ),
            },
            ClientError::Transport(m) => write!(f, "the host could not be reached ({m})"),
        }
    }
}
impl std::error::Error for ClientError {}

/// Accepts exactly one certificate: the one pinned at pairing. A change is a hard
/// refusal, not a prompt — the only honest reading of a changed key on a host that
/// can make a blade move.
#[derive(Debug)]
struct PinnedCert {
    fingerprint: String,
    seen: Mutex<Option<String>>,
}

impl rustls::client::danger::ServerCertVerifier for PinnedCert {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let found = crate::serve::fingerprint(end_entity);
        *self.seen.lock().unwrap() = Some(found.clone());
        if found == self.fingerprint {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("certificate fingerprint does not match the paired host".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message, cert, dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message, cert, dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}

/// Accepts any certificate, because a first pairing has nothing to compare one against — that
/// is the whole of trust-on-first-use, and `probe_fingerprint` is its only caller.
///
/// Safe here for two reasons that only hold together, and safe nowhere else because no other
/// call site has both. Nothing secret crosses a probe: it sends no token and no request, so a
/// host that lied about its identity is handed nothing it did not already have. And the
/// fingerprint a probe returns is not trusted, it is *shown* — what gets pinned is what the
/// operator confirmed against the Pi's own console, and every connection after that goes through
/// `PinnedCert` against the pinned value. Neither `connect` nor `connect_within` takes a
/// verifier — each builds its own `PinnedCert` from the fingerprint it was given — so there is
/// no parameter this type could be passed to and the misuse will not compile.
///
/// The signature checks below stay real rather than being waved through with the certificate:
/// they are what makes the reported fingerprint belong to a peer that holds the matching private
/// key, rather than to anyone who can replay somebody else's certificate.
#[derive(Debug)]
struct AnyCert;

impl rustls::client::danger::ServerCertVerifier for AnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message, cert, dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message, cert, dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// The first of `addr`'s resolved addresses that answers before `deadline`.
///
/// A single deadline covers the resolve and every resolved address, not a fresh budget per
/// address: one slow address must not spend what its siblings needed. The v6-first trap this
/// budget used to be the only defense against is now removed at the source — `resolve` orders
/// IPv4 ahead of IPv6 before this loop ever sees the list.
fn connect_by_deadline(addr: &str, deadline: Instant) -> Result<TcpStream, ClientError> {
    let addrs = crate::resolve::resolve_by_deadline(addr, deadline)?;
    let resolved_any = !addrs.is_empty();
    let mut last_err = None;
    for sock_addr in addrs {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&sock_addr, remaining) {
            Ok(s) => return Ok(s),
            // Named, not just the error: with several resolved addresses tried in turn, "the
            // host could not be reached (Connection refused)" alone leaves the operator no
            // way to tell which of them actually failed.
            Err(e) => last_err = Some((sock_addr, e)),
        }
    }
    Err(match last_err {
        Some((sock_addr, e)) => ClientError::Transport(format!("{sock_addr}: {e}")),
        // Distinct from "no address": the resolve answered and the budget ran out before a
        // single connect was tried — blaming the name here sends the operator to debug the
        // wrong layer.
        None if resolved_any => {
            ClientError::Transport(format!("no time was left to try `{addr}`'s addresses"))
        }
        None => ClientError::Transport(format!("`{addr}` resolved to no address")),
    })
}

/// The fingerprint of the certificate `addr` presents, learned by completing a TLS handshake and
/// doing nothing else with the connection.
///
/// The one entry point that does not already know the fingerprint, because it is what a *first*
/// pairing has: an address the operator has typed and nothing to compare against. The alternative
/// it replaces was to connect with a deliberately wrong fingerprint and read the real one back out
/// of the refusal's prose.
///
/// No token is sent and no `Request` is written — there is no credential in this signature to
/// send — and the connection is closed as soon as the certificate is in hand. See `AnyCert` for
/// why accepting any certificate is safe on this path and on no other.
pub fn probe_fingerprint(addr: &str, timeout: Duration) -> Result<String, ClientError> {
    let deadline = Instant::now() + timeout;
    let mut tcp = connect_by_deadline(addr, deadline)?;
    // Same pacing as every other read in this crate: the socket comes back empty every
    // `SOCKET_POLL_INTERVAL` so that the deadline bounds the whole handshake rather than each
    // read within it — `complete_io` on its own would grant a fresh wait per read.
    tcp.set_read_timeout(Some(crate::frame::SOCKET_POLL_INTERVAL))
        .map_err(|e| ClientError::Transport(e.to_string()))?;
    // The handshake writes as well as reads, and a peer that accepts the connection and then stops
    // reading blocks those writes with no error to notice. Same pacing, same reason as the read.
    tcp.set_write_timeout(Some(crate::frame::SOCKET_POLL_INTERVAL))
        .map_err(|e| ClientError::Transport(e.to_string()))?;

    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCert))
        .with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from("cuthulhu-cutd")
        .map_err(|e| ClientError::Transport(e.to_string()))?;
    let mut conn = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| ClientError::Transport(e.to_string()))?;

    while conn.is_handshaking() {
        if Instant::now() >= deadline {
            return Err(ClientError::Transport(
                "the host accepted a connection but did not finish a TLS handshake in time".into(),
            ));
        }
        match conn.complete_io(&mut tcp) {
            // rustls reports no progress only when it wants neither a read nor a write, which
            // part-way through a handshake means it will never finish: retrying that spins.
            Ok((0, 0)) => return Err(ClientError::Transport("the TLS handshake stalled".into())),
            Ok(_) => {}
            // The read timeout above surfaces as `WouldBlock` on unix and `TimedOut` on Windows,
            // and neither is a fault — it is this loop's cue to re-check the deadline.
            Err(e) if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
            ) => {}
            Err(e) => return Err(ClientError::Transport(e.to_string())),
        }
    }

    let found = conn
        .peer_certificates()
        .and_then(|certs| certs.first())
        .map(crate::serve::fingerprint)
        .ok_or_else(|| ClientError::Transport("the host presented no certificate".into()))?;
    // A close_notify rather than a bare drop: a Cut Host logs a client that vanishes as a fault,
    // and a probe that worked is not one — an operator reading the Pi's console while pairing
    // should not be shown an error for the step that succeeded.
    conn.send_close_notify();
    let _ = conn.write_tls(&mut tcp);
    Ok(found)
}

/// How long to wait for a Cut Host to accept a connection.
///
/// A host that refuses is instant; one that is silently unreachable — a dropped SYN, a
/// firewall discarding rather than refusing — would otherwise block for the OS default,
/// which is tens of seconds. The desktop holds *that host's* connection lock across this call
/// while listing devices — not the lock over all hosts — so an unbounded wait here is a frozen
/// device list, but no longer a frozen everything-else.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The server's half of the agreement. Read as `VersionReply` and nothing else, under the
/// version cap: a snapshot or a dispatch sent here is over the cap or the wrong shape, and
/// either way it is not decoded as one.
///
/// A timeout is reachability — the peer has not shown it speaks an old protocol, it has shown
/// nothing. Anything else after the offer was written is an older host. The unversioned daemon
/// reads this offer as a token, fails, and closes without a reply; a reset instead of a clean
/// close is the same end.
fn read_version_reply(stream: &mut Tls, deadline: Instant) -> Result<(), ClientError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    match read_frame::<_, VersionReply>(stream, VERSION_FRAME_MAX, Some(remaining), remaining) {
        Ok(VersionReply::Agreed { version }) if version == PROTOCOL_VERSION => Ok(()),
        Ok(VersionReply::Agreed { version })
        | Ok(VersionReply::Unsupported {
            supported: version, ..
        }) => Err(ClientError::Protocol {
            host_version: Some(version),
            desktop_version: PROTOCOL_VERSION,
        }),
        Ok(VersionReply::Unreadable) => Err(ClientError::Protocol {
            host_version: None,
            desktop_version: PROTOCOL_VERSION,
        }),
        Err(FrameError::Timeout) => Err(ClientError::Transport(
            "the host did not agree a protocol version in time".into(),
        )),
        Err(_) => Err(ClientError::Protocol {
            host_version: None,
            desktop_version: PROTOCOL_VERSION,
        }),
    }
}

/// TLS up to a stream the caller can write a frame on.
///
/// One dial for `connect_within` and for a test session that has to send something other than
/// the version this build speaks. The verifier comes back with the stream: a pin failure
/// arrives as an I/O error on the first write, and the certificate that was actually seen is
/// the only way to name it.
fn dial(
    addr: &str,
    pinned_fingerprint: &str,
    deadline: Instant,
) -> Result<(Tls, Arc<PinnedCert>), ClientError> {
    let verifier = Arc::new(PinnedCert {
        fingerprint: pinned_fingerprint.to_string(),
        seen: Mutex::new(None),
    });
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    let tcp = connect_by_deadline(addr, deadline)?;
    tcp.set_read_timeout(Some(crate::frame::SOCKET_POLL_INTERVAL))
        .map_err(|e| ClientError::Transport(e.to_string()))?;
    // Both directions bounded, not just the read: a host that stops draining freezes every
    // write on this connection — and they all happen under one mutex (#102).
    tcp.set_write_timeout(Some(crate::frame::SOCKET_POLL_INTERVAL))
        .map_err(|e| ClientError::Transport(e.to_string()))?;
    let server_name = rustls::pki_types::ServerName::try_from("cuthulhu-cutd")
        .map_err(|e| ClientError::Transport(e.to_string()))?;
    let conn = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| ClientError::Transport(e.to_string()))?;
    Ok((rustls::StreamOwned::new(conn, tcp), verifier))
}

/// A handshake that failed on the pin reaches the caller as an I/O error. The verifier has
/// the certificate that was actually seen, which is the message worth showing.
fn pin_or_transport(
    verifier: &PinnedCert,
    pinned: &str,
    err: &impl std::fmt::Display,
) -> ClientError {
    match verifier.seen.lock().unwrap().clone() {
        Some(found) if found != pinned => ClientError::Fingerprint {
            expected: pinned.to_string(),
            found,
        },
        _ => ClientError::Transport(err.to_string()),
    }
}

pub struct HostClient {
    /// Serialized: one request and its reply at a time. A Cut Host answers a
    /// handful of clients and a desktop makes one call at a time, so a lock is the
    /// whole of the concurrency story here.
    stream: Mutex<Tls>,
}

impl HostClient {
    pub fn connect(addr: &str, token: &str, pinned_fingerprint: &str) -> Result<HostClient, ClientError> {
        Self::connect_within(addr, token, pinned_fingerprint, CONNECT_TIMEOUT)
    }

    /// Same as `connect`, but the connect attempt is capped at `timeout` rather than always
    /// spending the full `CONNECT_TIMEOUT` — a caller with a short total budget for the whole
    /// call (a status poll behind a lock that must never block for long) must not have that
    /// budget eaten by a reconnect it did not choose the length of.
    pub fn connect_within(
        addr: &str,
        token: &str,
        pinned_fingerprint: &str,
        timeout: Duration,
    ) -> Result<HostClient, ClientError> {
        // The deadline outlives the connect: the greeting read below shares it, so a peer that
        // answers slowly and then stalls cannot cost this call twice its budget.
        let deadline = Instant::now() + timeout;
        let (mut stream, verifier) = dial(addr, pinned_fingerprint, deadline)?;

        // The version before the token. A peer that has not agreed one is not sent the
        // credential, and the offer is a `u32` — there is no version-specific payload in it.
        // Bounded by what is left of the connect budget: the handshake happens inside this
        // write, and a peer that completes the TCP connect and then stops reading must not
        // hold a short-budget caller open past it.
        let remaining = deadline.saturating_duration_since(Instant::now());
        write_frame(&mut stream, &PROTOCOL_VERSION, remaining)
            .map_err(|e| pin_or_transport(&verifier, pinned_fingerprint, &e))?;
        read_version_reply(&mut stream, deadline)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        write_frame(&mut stream, &token.to_string(), remaining)
            .map_err(|e| pin_or_transport(&verifier, pinned_fingerprint, &e))?;
        // The same `deadline` the connect loop used, not a fresh `timeout` and not
        // `DEFAULT_BODY_TIMEOUT`: a peer that finishes the TLS handshake and then stalls before
        // the greeting must not hold a caller's short budget open for the full 30s (what this
        // used to pass), and must not get a *second* full `timeout` on top of what the connect
        // loop already spent either — or `connect_within` as a whole could cost 2x its budget
        // instead of 1x.
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match read_frame::<_, Response>(&mut stream, 4096, Some(remaining), remaining) {
            Ok(Response::Ok) => {}
            // The host answered and said no. Before it did, this arrived as a dropped connection —
            // the same thing an asleep Pi looks like, and the two need opposite things from the
            // operator.
            Ok(Response::Unauthorized) => return Err(ClientError::Unauthorized),
            // Decided after `Unauthorized`, not instead of it: a refused token is also not `Ok`,
            // and it is the one answer on this path that tells the operator what to do next.
            Ok(other) => return Err(wrong_reply("Ok", &other)),
            Err(e) => return Err(pin_or_transport(&verifier, pinned_fingerprint, &e)),
        }

        Ok(HostClient { stream: Mutex::new(stream) })
    }

    /// Prove a host before anything about it is written down: connect, list its cutters, and
    /// drop the connection.
    ///
    /// Pairing that saves first and discovers later is how an operator ends up with an entry
    /// that has never worked. `connect` alone proves the fingerprint and the token; listing
    /// proves the daemon is actually serving.
    pub fn pair_check(
        addr: &str,
        token: &str,
        pinned_fingerprint: &str,
    ) -> Result<Vec<DeviceInfo>, ClientError> {
        HostClient::connect(addr, token, pinned_fingerprint)?.devices()
    }

    pub fn devices(&self) -> Result<Vec<DeviceInfo>, ClientError> {
        match self.call(Request::ListDevices, crate::frame::DEFAULT_BODY_TIMEOUT)? {
            Response::Devices(d) => Ok(d),
            other => Err(wrong_reply("Devices", &other)),
        }
    }

    pub fn snapshots(&self) -> Result<Vec<DeviceSnapshot>, ClientError> {
        self.snapshots_within(crate::frame::DEFAULT_BODY_TIMEOUT)
    }

    /// Same as `snapshots`, but bounded by `timeout` rather than the full
    /// `DEFAULT_BODY_TIMEOUT`. For a status poll a stale answer is fine — the next poll is a
    /// second away — but holding whatever lock guards this client for 30s while a Job-carrying
    /// call would rightly wait that long is not.
    pub fn snapshots_within(&self, timeout: Duration) -> Result<Vec<DeviceSnapshot>, ClientError> {
        match self.call(Request::Snapshot, timeout)? {
            Response::Snapshots(s) => Ok(s),
            other => Err(wrong_reply("Snapshots", &other)),
        }
    }

    /// `Admitted` is the answer, not `()`: a host that recognised this id started nothing, and the
    /// caller is the only one who can put that in front of the operator (#121).
    ///
    /// Borrowed rather than moved because the id has to outlive the request carrying it: `Accepted`
    /// echoes back the dispatch it is answering about, and this is the only reply on this
    /// connection whose subject is not settled by structure. Everywhere else one request is
    /// written and frames are read until a reply, so a reply of the right variant can only be this
    /// request's; here it can name another Job, and the id is the whole of what a retry rests
    /// on (#285). Cloning for the request costs what the caller's own clone used to.
    pub fn dispatch(
        &self,
        dispatch_id: &DispatchId,
        device: &str,
        machine_id: &str,
        passes: Vec<CutPass>,
    ) -> Result<Admitted, ClientError> {
        match self.call(
            Request::Dispatch {
                dispatch_id: dispatch_id.clone(),
                device: device.to_string(),
                machine_id: machine_id.to_string(),
                passes,
            },
            crate::frame::DEFAULT_BODY_TIMEOUT,
        )? {
            Response::Accepted { dispatch_id: answered, admitted } if &answered == dispatch_id =>
                Ok(admitted),
            Response::Accepted { .. } => Err(ClientError::WrongDispatch),
            other => Err(wrong_reply("Accepted", &other)),
        }
    }

    pub fn cancel(&self, device: &str) -> Result<(), ClientError> {
        self.expect_ok(Request::Cancel { device: device.to_string() })
    }

    pub fn resume(&self, device: &str) -> Result<(), ClientError> {
        self.expect_ok(Request::Resume { device: device.to_string() })
    }

    pub fn confirm_pass_done(&self, device: &str) -> Result<(), ClientError> {
        self.expect_ok(Request::ConfirmPassDone { device: device.to_string() })
    }

    pub fn reconnect(&self, device: &str) -> Result<(), ClientError> {
        self.expect_ok(Request::Reconnect { device: device.to_string() })
    }

    fn expect_ok(&self, request: Request) -> Result<(), ClientError> {
        match self.call(request, crate::frame::DEFAULT_BODY_TIMEOUT)? {
            Response::Ok => Ok(()),
            other => Err(wrong_reply("Ok", &other)),
        }
    }

    /// One request, then read until the reply. Frames that are events rather than
    /// responses are skipped: the host writes them on the same connection, and one
    /// can land ahead of a response — a Job's own Paused event flushed right after
    /// the Dispatch response that started it, or a connect-time event still queued
    /// from before this client's subscription began (`serve.rs`'s `serve_client`
    /// drains before it blocks on the next request, not after).
    ///
    /// `body_timeout` is a caller's choice, not a constant: a Job-carrying call (dispatch,
    /// cancel, resume) is rightly owed the full `DEFAULT_BODY_TIMEOUT`, but a status poll would
    /// rather see a stale answer next second than hold this client's lock for 30s — see
    /// `snapshots_within`.
    fn call(&self, request: Request, body_timeout: Duration) -> Result<Response, ClientError> {
        let mut stream = self.stream.lock().unwrap();
        // The write is bounded by the same budget as the reply, and for the same reason: a host
        // that stops draining mid-dispatch — megabytes of polylines over a weak link — would
        // otherwise hold this mutex, and every other call on this host, forever (#102).
        write_frame(&mut *stream, &request, body_timeout)
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        loop {
            // A reply is owed the moment the request above was written: a Pi that goes silent
            // from here must not hold this mutex, and every other call, forever.
            match read_frame::<_, Incoming>(
                &mut *stream,
                crate::frame::DEFAULT_MAX_FRAME,
                Some(body_timeout),
                body_timeout,
            ) {
                // ponytail: event frames are read only to be discarded — there is no
                // queue and no accessor, so a client that never calls anything sees no
                // events and a client that calls rarely misses whatever arrived between
                // calls. Phase 2's UI will need events pushed, not implied by a
                // Snapshot's absence; give them a connection of their own then.
                Ok(Incoming::Event(_event)) => continue,
                Ok(Incoming::Response(Response::Refused(r))) => return Err(ClientError::Refused(r)),
                Ok(Incoming::Response(response)) => return Ok(response),
                Err(FrameError::Eof) =>
                    return Err(ClientError::Transport("the host closed the connection".into())),
                Err(e) => return Err(ClientError::Transport(e.to_string())),
            }
        }
    }
}

/// A frame on this connection is either the reply to the request that was just
/// sent, or an event for some cutter that the host had queued. Untagged because
/// the two have no overlapping shape — `Response`'s variants are keyed by name,
/// `Event` is a plain `{device, event}` struct — so there is nothing to guess.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Incoming {
    Response(Response),
    Event(Event),
}

/// A `Response`'s variant name, which is what a reply of the *wrong variant* is worth saying and
/// all of it: the value behind it belongs to a reply this request cannot use, and `Devices` carries
/// every field of every cutter the host knows (#283). It says nothing about a reply of the right
/// variant carrying the wrong contents, and cannot: `dispatch` compares the `dispatch_id` echoed
/// back to it and raises `WrongDispatch`, which no variant name distinguishes from a correct
/// `Accepted` (#285). Private to this module, since naming a reply is only useful where one turned
/// out to be the wrong one.
fn response_name(response: &Response) -> &'static str {
    match response {
        Response::Devices(_) => "Devices",
        Response::Snapshots(_) => "Snapshots",
        Response::Accepted { .. } => "Accepted",
        Response::Ok => "Ok",
        Response::Refused(_) => "Refused",
        Response::Unauthorized => "Unauthorized",
    }
}

/// `expected` is the caller's, because nothing below it knows which reply was owed: `call` hands
/// back every `Response` it did not turn into a `Refused`, and the greeting is read before any
/// `Request` exists to have asked for one.
fn wrong_reply(expected: &'static str, found: &Response) -> ClientError {
    ClientError::WrongReply { expected, found: response_name(found) }
}

impl From<io::Error> for ClientError {
    fn from(e: io::Error) -> Self {
        ClientError::Transport(e.to_string())
    }
}

/// A peer that answers a client with replies of the test's choosing, including variants no real
/// Cut Host sends in the position it sends them in.
///
/// Public rather than `#[cfg(test)]` for the reason `host::testing` is: the desktop's own tests
/// compile as a separate crate and cannot reach test-only code. Nothing that serves a real `Host`
/// can stand in — `serve_client` answers every request through `handle_request`, which returns
/// only replies that request admits — so the mismatch branches need a peer built to break the
/// protocol.
pub mod testing;

#[cfg(test)]
mod tests;
