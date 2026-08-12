// SPDX-License-Identifier: GPL-3.0-or-later
//! What a desktop holds to reach a Cut Host.
//!
//! One TLS connection carries every cutter on the host: requests out, events in.
//! The certificate is pinned by fingerprint rather than validated by an authority,
//! because a Pi on a home network has no name an authority would sign.

use std::io;
use std::net::{TcpStream, ToSocketAddrs};
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
            ClientError::Refused(Refusal::Device(e)) => write!(f, "the cutter refused: {e:?}"),
            ClientError::Refused(Refusal::DispatchIdTooLong { max }) =>
                write!(f, "this host will not accept a dispatch id longer than {max} characters"),
            ClientError::Fingerprint { expected, found } =>
                write!(f, "this host presented a different certificate than the one paired \
                           (expected {expected}, found {found})"),
            ClientError::Unauthorized =>
                write!(f, "this host refused the token; pair again with the one in its `cutd.toml`"),
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

/// `addr`'s resolved addresses, or a failure once `deadline` passes.
///
/// `to_socket_addrs()` is synchronous and takes no timeout, so the only way to bound the *wait*
/// is to stop waiting: the resolve runs on a thread of its own and the answer is taken over a
/// channel. A Cut Host is addressed by name (`cuthulhu-pi.local:7878`), not by a literal IP, so
/// this is the common path — and mDNS on a flaky network is the ordinary way a resolver wedges.
///
/// One address may be resolving at a time; a second attempt is refused rather than given a thread
/// of its own. See `resolve_by_deadline` for why the thread cannot simply be cancelled.
static RESOLVING: Mutex<std::collections::BTreeSet<String>> =
    Mutex::new(std::collections::BTreeSet::new());

/// Resolver threads alive right now, whether or not anyone is still waiting on one.
///
/// The thing being bounded is threads, so threads are what this counts. Two narrower policies were
/// tried and both were wrong in their own direction: counting only *stalled* lookups cannot be
/// enforced exactly, because whether a resolve will be abandoned is not known when it is admitted,
/// so a burst of simultaneous timeouts sails past the limit together; counting in-flight ones with
/// a small limit refused a perfectly healthy name because other names were stuck.
///
/// One counter, reserved before the thread is spawned and released when it returns, is both exact
/// and generous — see `MAX_RESOLVER_THREADS`.
static RESOLVER_THREADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The ceiling on resolver threads this process will hold at once.
///
/// High on purpose. It is a backstop against unbounded growth, not a queue depth: a resolve is
/// only started for a name that no other resolve is already out for (`RESOLVING`), so reaching
/// this needs 128 *distinct* names being resolved simultaneously — which a desktop dialling a
/// handful of Cut Hosts cannot produce, and which a wedged resolver cannot inflate either, since
/// the repeat attempts a stuck host generates are refused before they get here. Refusing at that
/// point is the honest answer, and it clears as the threads return.
const MAX_RESOLVER_THREADS: usize = 128;

/// Take a slot from `counter`, or report that there is none.
///
/// One atomic step, because checking and then taking is what let simultaneous callers all see room
/// and all take it: with the limit one away, a burst of distinct names passed the check together
/// and left more threads out than the limit allows.
///
/// Takes the counter rather than reading the static one, so the rule can be tested on a counter of
/// the test's own — these tests run in parallel and share the real one, and a test that asserts on
/// a shared count is asserting on whatever else happened to be running.
fn claim_slot(counter: &std::sync::atomic::AtomicUsize, max: usize) -> bool {
    use std::sync::atomic::Ordering::SeqCst;
    counter.fetch_update(SeqCst, SeqCst, |out| (out < max).then_some(out + 1)).is_ok()
}

/// ponytail: on a genuine timeout the thread stays blocked in the resolver and leaks, exactly as
/// `driver-silhouette`'s `usb.rs` read does and for the same reason — the OS resolver takes no
/// cancellation. The upgrade is an async resolver crate, at the cost of a dependency.
///
/// What makes the leak survivable is `RESOLVING`, not the rarity of the path: this is *not* a
/// once-per-connect cost. The cut dialog polls every second, and a poll against a host with no
/// live connection redials — so a resolver that stays wedged would otherwise be handed a fresh
/// thread every second until the process ran out. Refusing a second attempt while the first is
/// still out bounds it at one thread per address, and the thread clears its own claim whenever the
/// resolver finally answers, so a name that resolves slowly once is not blacklisted afterwards.
///
/// Per-address is not a ceiling on its own — the number of addresses is not fixed, since an
/// operator retyping one in the pairing dialog produces a fresh string each time — so
/// `MAX_ABANDONED_RESOLVES` bounds the total. It counts only the threads nobody is waiting on any
/// more: a healthy name must not be refused because other names are stalled.
fn resolve_by_deadline(addr: &str, deadline: Instant) -> Result<Vec<std::net::SocketAddr>, ClientError> {
    use std::sync::atomic::Ordering::SeqCst;

    // An address that is already an address needs no resolver, no thread and no ceiling.
    // `to_socket_addrs` would answer this from the string itself too, but only after being handed
    // to a worker and counted against the machinery above — so a Cut Host paired by IP, which is
    // what `docs/cut-host.md` has the operator put in `bind`, stays reachable however badly this
    // machine's name resolution is behaving.
    if let Ok(literal) = addr.parse::<std::net::SocketAddr>() {
        return Ok(vec![literal]);
    }

    {
        let mut in_flight = RESOLVING.lock().unwrap_or_else(|e| e.into_inner());
        if in_flight.contains(addr) {
            return Err(ClientError::Transport(format!(
                "`{addr}` is still being resolved from an earlier attempt"
            )));
        }
        in_flight.insert(addr.to_string());
    }
    // Taken before the thread exists, so the count can never describe fewer threads than are out.
    if !claim_slot(&RESOLVER_THREADS, MAX_RESOLVER_THREADS) {
        RESOLVING.lock().unwrap_or_else(|e| e.into_inner()).remove(addr);
        return Err(ClientError::Transport(
            "too many host names are stuck being resolved; this machine's resolver is not answering"
                .into(),
        ));
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let owned = addr.to_string();
    // `Builder`, not `thread::spawn`: spawn panics when a thread cannot be created, and a client
    // that cannot resolve a name has to report that, not take the desktop down with it.
    let spawned = std::thread::Builder::new().name(format!("resolve {addr}")).spawn(move || {
        let resolved = owned.to_socket_addrs().map(|a| a.collect::<Vec<_>>());
        // Released here rather than by the waiter, which may have given up long ago — until the
        // resolver returns there is still a thread out for this address, and that is precisely
        // what a second attempt, and the ceiling, must keep counting.
        RESOLVING.lock().unwrap_or_else(|e| e.into_inner()).remove(&owned);
        RESOLVER_THREADS.fetch_sub(1, SeqCst);
        let _ = tx.send(resolved);
    });
    if let Err(e) = spawned {
        RESOLVING.lock().unwrap_or_else(|e| e.into_inner()).remove(addr);
        RESOLVER_THREADS.fetch_sub(1, SeqCst);
        return Err(ClientError::Transport(format!("could not resolve `{addr}`: {e}")));
    }

    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(addrs)) => Ok(addrs),
        Ok(Err(e)) => Err(ClientError::Transport(e.to_string())),
        // The thread keeps its slot from here: it is still out, and the count is about threads.
        Err(_) => Err(ClientError::Transport(format!("`{addr}` could not be resolved in time"))),
    }
}

/// The first of `addr`'s resolved addresses that answers before `deadline`.
///
/// A single deadline covers the resolve and every resolved address, not a fresh budget per
/// address: mDNS commonly returns an IPv6 link-local first, and on a network where IPv6 is dead
/// that is the one address that cannot work — trying it with the whole budget and only
/// then trying the IPv4 that would have worked turns "first contact" into a timeout.
fn connect_by_deadline(addr: &str, deadline: Instant) -> Result<TcpStream, ClientError> {
    let addrs = resolve_by_deadline(addr, deadline)?;
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
            Ok(other) => return Err(ClientError::Transport(format!("unexpected greeting: {other:?}"))),
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
            other => Err(unexpected(other)),
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
            other => Err(unexpected(other)),
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
            other => Err(unexpected(other)),
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
            other => Err(unexpected(other)),
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

fn unexpected(response: Response) -> ClientError {
    ClientError::Transport(format!("the host answered with {response:?}"))
}

impl From<io::Error> for ClientError {
    fn from(e: io::Error) -> Self {
        ClientError::Transport(e.to_string())
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

    /// A host paired by IP never touches the resolver, so nothing about the machine's name
    /// resolution — wedged, slow, or at the ceiling — can make it unreachable. The documented
    /// `bind` in `docs/cut-host.md` is a literal address, so this is the ordinary case, not a
    /// corner of one.
    #[test]
    fn an_address_that_is_already_an_address_is_not_resolved_at_all() {
        // A deadline already in the past, which every path through the resolver would fail on:
        // answering anyway is only possible without one. That is the assertion, and it needs no
        // global state — an earlier version of this test pinned the shared ceiling to prove the
        // same thing and starved every test running beside it of a slot.
        let resolved = resolve_by_deadline("192.168.1.50:7878", Instant::now())
            .expect("a literal address must not depend on the resolver at all");
        assert_eq!(resolved, vec!["192.168.1.50:7878".parse::<std::net::SocketAddr>().unwrap()]);

        // And it takes nothing that another attempt would then be refused for. This address
        // specifically, not the whole set, which other tests legitimately hold entries in.
        assert!(
            !RESOLVING.lock().unwrap_or_else(|e| e.into_inner()).contains("192.168.1.50:7878"),
            "a literal address must not occupy the resolver's dedup either"
        );
    }

    /// A slot is taken in one step, so simultaneous callers cannot all find the last one free.
    /// Checking and then taking let a burst of distinct names pass the check together and leave
    /// more threads out than the limit allows — a ceiling that a rush walks straight through is
    /// not a ceiling.
    #[test]
    fn resolver_slots_cannot_be_taken_twice_by_callers_that_looked_at_once() {
        use std::sync::atomic::AtomicUsize;

        // A counter of this test's own, one slot short of its limit, and far more callers than
        // that reaching for it together.
        let counter = AtomicUsize::new(7);
        let taken: usize = std::thread::scope(|s| {
            let racers: Vec<_> =
                (0..16).map(|_| s.spawn(|| usize::from(claim_slot(&counter, 8)))).collect();
            racers.into_iter().map(|r| r.join().unwrap()).sum()
        });

        assert_eq!(taken, 1, "{taken} callers were all handed the last slot");
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 8, "the limit was passed");
    }

    /// A slot comes back when the resolve returns, so a working machine cannot exhaust the
    /// ceiling by using it. Asserted by using it far more times than the ceiling allows rather
    /// than by reading the shared count, which every other test in this file also moves.
    ///
    /// `localhost` resolves from the hosts file, so none of these waits on a network.
    #[test]
    fn resolves_that_answer_give_their_slots_back() {
        for attempt in 0..MAX_RESOLVER_THREADS + 20 {
            resolve_by_deadline("localhost:7878", Instant::now() + Duration::from_secs(5))
                .unwrap_or_else(|e| panic!("attempt {attempt} was refused a slot: {e}"));
        }
    }

    /// A name that cannot be resolved must fail, and must leave nothing behind that stops the next
    /// attempt: the in-flight claim exists to bound leaked resolver threads, and a claim released
    /// only by the waiter would make one failed lookup wedge that address for the process's life.
    ///
    /// `.invalid` never resolves, by RFC 2606, so this fails on the resolver's own answer rather
    /// than on the deadline — which is what leaves the second attempt free to run.
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
}
