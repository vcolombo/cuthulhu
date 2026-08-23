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
use crate::protocol::{Admitted, DeviceSnapshot, DispatchId, Event, Refusal, Request, Response};

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
        let verifier = Arc::new(PinnedCert {
            fingerprint: pinned_fingerprint.to_string(),
            seen: Mutex::new(None),
        });
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone())
            .with_no_client_auth();

        // The deadline outlives the connect: the greeting read below shares it, so a peer that
        // answers slowly and then stalls cannot cost this call twice its budget.
        let deadline = Instant::now() + timeout;
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
        let mut stream = rustls::StreamOwned::new(conn, tcp);

        // Bounded by what is left of the connect budget, like the greeting read below: the
        // handshake happens inside this write, and a peer that completes the TCP connect and then
        // stops reading must not hold a short-budget caller open past it.
        let remaining = deadline.saturating_duration_since(Instant::now());
        write_frame(&mut stream, &token.to_string(), remaining).map_err(|e| {
            // A handshake that failed on the pin reaches here as an I/O error, so
            // the more useful message is the one the verifier can give.
            match verifier.seen.lock().unwrap().clone() {
                Some(found) if found != pinned_fingerprint =>
                    ClientError::Fingerprint { expected: pinned_fingerprint.to_string(), found },
                _ => ClientError::Transport(e.to_string()),
            }
        })?;
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
            Err(e) => {
                if let Some(found) = verifier.seen.lock().unwrap().clone() {
                    if found != pinned_fingerprint {
                        return Err(ClientError::Fingerprint {
                            expected: pinned_fingerprint.to_string(),
                            found,
                        });
                    }
                }
                return Err(ClientError::Transport(e.to_string()));
            }
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
    pub fn dispatch(
        &self,
        dispatch_id: DispatchId,
        device: &str,
        machine_id: &str,
        passes: Vec<CutPass>,
    ) -> Result<Admitted, ClientError> {
        match self.call(
            Request::Dispatch {
                dispatch_id,
                device: device.to_string(),
                machine_id: machine_id.to_string(),
                passes,
            },
            crate::frame::DEFAULT_BODY_TIMEOUT,
        )? {
            Response::Accepted { admitted, .. } => Ok(admitted),
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

/// A `Response`'s variant name, which is the whole of what a mismatch is worth saying: the value
/// behind it belongs to a reply this request cannot use, and `Devices` carries every field of every
/// cutter the host knows (#283). Private to this module, since naming a reply is only useful where
/// one turned out to be the wrong one.
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

/// A peer that answers a client with replies of the test's choosing, including ones no Cut Host
/// sends.
///
/// Public rather than `#[cfg(test)]` for the reason `host::testing` is: the desktop's own tests
/// compile as a separate crate and cannot reach test-only code. Nothing that serves a real `Host`
/// can stand in — `serve_client` answers every request through `handle_request`, which returns
/// only replies that request admits — so the mismatch branches need a peer built to break the
/// protocol.
pub mod testing {
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::time::Duration;

    use crate::frame::{read_frame, write_frame, DEFAULT_MAX_FRAME, SOCKET_POLL_INTERVAL};
    use crate::protocol::{Request, Response};

    /// Where a misbehaving host is listening, and the fingerprint a client must pin to reach it.
    pub struct MisbehavingHost {
        pub addr: String,
        pub fingerprint: String,
    }

    /// How long this peer waits on any one frame. Generous, because it bounds nothing a test is
    /// asserting on — a client that stops talking should not hold the thread for the whole run,
    /// and that is all this is for.
    const BUDGET: Duration = Duration::from_secs(10);

    /// Answers `replies` in order: the first as the greeting a token earns, then one per request.
    ///
    /// Binds port 0 so tests run concurrently, and replays the same script on every connection, so
    /// a client that redials is answered rather than left hanging.
    pub fn start_host_answering(replies: Vec<Response>) -> MisbehavingHost {
        let cert = rcgen::generate_simple_self_signed(vec!["cuthulhu-cutd".to_string()])
            .expect("a self-signed certificate for the fake host");
        let der = cert.cert.der().clone();
        let fingerprint = crate::serve::fingerprint(&der);
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(cert.key_pair.serialize_der().into());
        let tls = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![der], key)
                .expect("the generated certificate and its own key"),
        );

        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let addr = listener.local_addr().expect("the port the OS chose").to_string();
        let script = Arc::new(replies);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (tls, script) = (tls.clone(), script.clone());
                std::thread::spawn(move || answer(stream, tls, &script));
            }
        });
        MisbehavingHost { addr, fingerprint }
    }

    fn answer(stream: TcpStream, tls: Arc<rustls::ServerConfig>, replies: &[Response]) {
        // The frame layer re-checks its deadline whenever a read comes back empty, so the socket
        // has to come back empty rather than block — the pacing `serve_client` sets, for the same
        // reason.
        let _ = stream.set_read_timeout(Some(SOCKET_POLL_INTERVAL));
        let _ = stream.set_write_timeout(Some(SOCKET_POLL_INTERVAL));
        let Ok(conn) = rustls::ServerConnection::new(tls) else { return };
        let mut tls = rustls::StreamOwned::new(conn, stream);
        // The token is read and discarded: a client that got this far already pinned the
        // certificate, and nothing here is proving anything about tokens.
        if read_frame::<_, String>(&mut tls, 1024, Some(BUDGET), BUDGET).is_err() {
            return;
        }
        for reply in replies {
            if write_frame(&mut tls, reply, BUDGET).is_err() {
                return;
            }
            // Each reply after the greeting waits for the request it answers, so none can land
            // ahead of it and be read as the answer to something else.
            if read_frame::<_, Request>(&mut tls, DEFAULT_MAX_FRAME, Some(BUDGET), BUDGET).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
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

    /// The whole table at once: a new variant fails to compile `Display`'s match, and a reworded
    /// one fails here. These are the strings the desktop shows — `host_error` puts `to_string()`
    /// in the message unaltered.
    ///
    /// Two rows compute the sentence they expect from the value underneath instead of restating it,
    /// because those two forward a payload whole: writing the forwarded wording out here would
    /// compare a literal with itself and pin the wrong layer, when the claim being made is that the
    /// payload arrives unaltered. `Transport`'s row does restate its sentence, and rightly — the
    /// wrapping around the payload is this type's own, and the payload it wraps is one of the many
    /// the client and its resolver write rather than a sentence from a layer below.
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
                ClientError::Transport("the host closed the connection".into()),
                "the host could not be reached (the host closed the connection)".into(),
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

    /// Every verb that reads a reply, against a host that answers a variant the verb cannot use.
    ///
    /// All four on one connection, which is itself the assertion that a mismatch is not a broken
    /// socket: `call` writes one request and reads frames until the reply, so the stream is still
    /// aligned afterwards and the next verb is answered normally. Each verb passes its own
    /// `expected` name, so each of the four is a separate hand-written string to pin.
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
                .dispatch(DispatchId("d1".into()), "usb:1:4", "cameo5", Vec::new())
                .unwrap_err()
                .to_string(),
            "this host answered with `Ok` where `Accepted` was expected"
        );
        assert_eq!(
            client.cancel("usb:1:4").unwrap_err().to_string(),
            "this host answered with `Devices` where `Ok` was expected"
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
}
