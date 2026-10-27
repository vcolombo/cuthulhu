// SPDX-License-Identifier: GPL-3.0-or-later

//! Serving a Cut Host over TLS.
//!
//! `handle_request` is deliberately separate from the socket: it is the whole of
//! what a request means, and it is what the tests drive. `serve` adds a listener,
//! a certificate and a token to it and nothing else.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use socket2::{SockRef, TcpKeepalive};

use crate::config::Config;
use crate::frame::{read_frame, write_frame, FrameError, DEFAULT_BODY_TIMEOUT, SOCKET_POLL_INTERVAL};
use crate::host::Host;
use crate::protocol::{Request, Response, VersionReply, PROTOCOL_VERSION, VERSION_FRAME_MAX};

/// How long a stream may sit silent before the OS sends the first probe. The
/// request loop's read (`serve_client`) waits with no deadline of its own — a
/// desktop that polls only while a dialog is open is *meant* to go quiet for as
/// long as the operator leaves it alone (see the amending comment on #103), so
/// nothing at the application layer may treat silence as absence. Only a kernel
/// probe can tell a suspended peer from a quiet one, since both look identical
/// on the wire until then. A minute keeps eight stacked lid-closes from
/// exhausting `MAX_CLIENTS` long before anyone notices, without probing a
/// healthy LAN connection often enough to be chatter.
const KEEPALIVE_IDLE: Duration = Duration::from_secs(60);
/// Gap between probes once idle time is exceeded.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
/// Failed probes tolerated before the OS reports the connection dead — worst
/// case `KEEPALIVE_IDLE + KEEPALIVE_RETRIES * KEEPALIVE_INTERVAL` (2 minutes)
/// before a slot is freed.
const KEEPALIVE_RETRIES: u32 = 4;

/// Turn on TCP keepalive so a peer that vanishes without closing (laptop
/// suspended, access point dropped — no FIN or RST ever arrives) is eventually
/// reaped instead of holding a `MAX_CLIENTS` slot forever. `SockRef` borrows the
/// stream's socket to set the option without taking ownership of the fd, so the
/// caller keeps using `stream` as normal afterwards.
///
/// A failure here is logged and swallowed, not propagated: refusing to serve a
/// client because a socket option did not apply would recreate the exhaustion
/// this exists to fix, only faster. `with_retries` needs socket2's `all`
/// feature to exist at all, but once enabled it — like `with_time` and
/// `with_interval` — is implemented for both Linux (`TCP_KEEPIDLE`/`TCP_KEEPCNT`)
/// and macOS (`TCP_KEEPALIVE`/`TCP_KEEPCNT`), so developing on a Mac exercises
/// the same code path the Pi runs.
fn enable_keepalive(stream: &TcpStream, peer: &str) {
    let keepalive = TcpKeepalive::new()
        .with_time(KEEPALIVE_IDLE)
        .with_interval(KEEPALIVE_INTERVAL)
        .with_retries(KEEPALIVE_RETRIES);
    if let Err(e) = SockRef::from(stream).set_tcp_keepalive(&keepalive) {
        eprintln!("cut host: could not enable keepalive for {peer}: {e}");
    }
}

/// The maximum clients served at once. A Cut Host answers a handful of desktops;
/// anything beyond this is a bug or an attempt to exhaust it.
pub const MAX_CLIENTS: usize = 8;

/// How long an unauthenticated peer has, from the connection being accepted, to agree a protocol
/// version and present its token.
///
/// Short on purpose, and much shorter than `DEFAULT_BODY_TIMEOUT`: a peer that has proved nothing
/// has earned less patience than a paired desktop mid-dispatch, and until it has, it is holding
/// one of `MAX_CLIENTS` slots. One deadline covers both frames, so the version offer does not
/// buy a second budget. Within a single frame the wait to begin and the rest still stack — that
/// is `read_frame`'s own bound — so the worst an unauthenticated peer can squat stays roughly
/// `2x` this (#118).
const PRE_AUTH_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a connection ended before a token was read. The refusal, when there is one, has already
/// been written; the caller closes.
#[derive(Debug)]
pub(crate) enum VersionFailure {
    Timeout,
    Closed,
    Unsupported { offered: u32, supported: u32 },
    Unreadable,
    TooLarge { len: usize },
    Io(String),
}

impl std::fmt::Display for VersionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionFailure::Timeout => write!(f, "offered no protocol version in time"),
            VersionFailure::Closed => write!(f, "closed before offering a protocol version"),
            VersionFailure::Unsupported { offered, supported } => write!(
                f,
                "offered protocol version {offered}, this host speaks {supported}"
            ),
            VersionFailure::Unreadable => {
                write!(f, "offered something that is not a protocol version")
            }
            VersionFailure::TooLarge { len } => {
                write!(f, "offered a protocol version of {len} bytes, over the {VERSION_FRAME_MAX} byte limit")
            }
            VersionFailure::Io(m) => write!(f, "failed during the protocol version ({m})"),
        }
    }
}

/// Agree `supported`, or write a named refusal and fail.
///
/// `supported` is the one version this side speaks. The daemon passes
/// `PROTOCOL_VERSION`; a scripted host passes the version it is pretending to speak,
/// so the refusal a desktop test sees is this function's and not a second handshake.
/// The offer is read as a `u32` and nothing else. A `Request` is not decoded from
/// this frame: a peer that sent one fails here, and the connection closes, before
/// `handle_request` exists on the path. `deadline` is the caller's whole
/// unauthenticated budget, shared with the token read that follows a success.
pub(crate) fn establish_version<S: Read + Write>(
    stream: &mut S,
    deadline: Instant,
    supported: u32,
) -> Result<(), VersionFailure> {
    let remaining = || deadline.saturating_duration_since(Instant::now());
    let offered =
        match read_frame::<_, u32>(stream, VERSION_FRAME_MAX, Some(remaining()), remaining()) {
            Ok(offered) => offered,
            Err(FrameError::Timeout) => return Err(VersionFailure::Timeout),
            Err(FrameError::Eof) => return Err(VersionFailure::Closed),
            Err(FrameError::TooLarge { len, .. }) => {
                // The body was not read: `read_frame` refuses the length first. Whatever
                // the peer still has to send dies with the socket.
                write_version_refusal(stream, &VersionReply::Unreadable, deadline);
                return Err(VersionFailure::TooLarge { len });
            }
            Err(FrameError::Malformed(_)) => {
                write_version_refusal(stream, &VersionReply::Unreadable, deadline);
                return Err(VersionFailure::Unreadable);
            }
            Err(FrameError::Io(m)) => return Err(VersionFailure::Io(m)),
        };

    if offered != supported {
        write_version_refusal(
            stream,
            &VersionReply::Unsupported {
                offered,
                supported,
            },
            deadline,
        );
        return Err(VersionFailure::Unsupported { offered, supported });
    }

    write_frame(
        stream,
        &VersionReply::Agreed {
            version: supported,
        },
        remaining(),
    )
    .map_err(|e| match e {
        FrameError::Timeout => VersionFailure::Timeout,
        FrameError::Io(m) => VersionFailure::Io(m),
        FrameError::Eof => VersionFailure::Closed,
        FrameError::TooLarge { len, .. } => VersionFailure::TooLarge { len },
        FrameError::Malformed(m) => VersionFailure::Io(m),
    })?;
    Ok(())
}

/// Best-effort: a peer that has already stopped reading learns nothing from a refusal it
/// cannot hear, and the connection is about to close either way. Bounded by whatever is
/// left of the unauthenticated budget so the write cannot outlive it.
fn write_version_refusal<W: Write>(stream: &mut W, reply: &VersionReply, deadline: Instant) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return;
    }
    let _ = write_frame(stream, reply, remaining);
}

/// The name of the token that matched, or `None`. Every candidate is compared in full, so the
/// time taken says nothing about how much of a token was right — and comparing all of them,
/// rather than stopping at the first match, keeps that true as clients are added.
pub fn match_token(presented: &str, tokens: &BTreeMap<String, String>) -> Option<String> {
    let mut matched: Option<String> = None;
    for (name, value) in tokens {
        if constant_time_eq(presented, value) {
            matched = Some(name.clone());
        }
    }
    matched
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// What a request means, with no socket involved.
pub fn handle_request(host: &Arc<Host>, request: Request) -> Response {
    match request {
        Request::ListDevices => Response::Devices(host.devices()),
        Request::Snapshot => Response::Snapshots(host.snapshots()),
        Request::Dispatch { dispatch_id, device, machine_id, passes } => {
            match host.dispatch(dispatch_id.clone(), &device, &machine_id, passes) {
                Ok(admitted) => Response::Accepted { dispatch_id, admitted },
                Err(refusal) => Response::Refused(refusal),
            }
        }
        Request::Cancel { device } => once(host.cancel(&device)),
        Request::Resume { device } => once(host.resume(&device)),
        Request::ConfirmPassDone { device } => once(host.confirm_pass_done(&device)),
        Request::Reconnect { device } => once(host.reconnect(&device)),
    }
}

fn once(result: Result<(), crate::protocol::Refusal>) -> Response {
    match result {
        Ok(()) => Response::Ok,
        Err(refusal) => Response::Refused(refusal),
    }
}

/// The certificate this host presents, generated on first run and reused after.
/// Self-signed on purpose: a client pins its fingerprint at pairing, which needs
/// no authority and no name that resolves.
fn load_or_make_cert(dir: &Path) -> io::Result<(Vec<rustls::pki_types::CertificateDer<'static>>, rustls::pki_types::PrivateKeyDer<'static>)> {
    // Prevent path traversal attacks by rejecting paths containing '..'.
    if dir.components().any(|c| c == std::path::Component::ParentDir) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid input: {}", dir.display())));
    }
    std::fs::create_dir_all(dir)?;
    let cert_path = dir.join("cutd.crt");
    let key_path = dir.join("cutd.key");

    if !cert_path.exists() || !key_path.exists() {
        let cert = rcgen::generate_simple_self_signed(vec!["cuthulhu-cutd".to_string()])
            .map_err(io::Error::other)?;
        std::fs::write(&cert_path, cert.cert.pem())?;
        std::fs::write(&key_path, cert.key_pair.serialize_pem())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }
        eprintln!("cut host: generated a certificate at {}", cert_path.display());
    }

    let certs = rustls_pemfile::certs(&mut io::BufReader::new(std::fs::File::open(&cert_path)?))
        .collect::<Result<Vec<_>, _>>()?;
    let key = rustls_pemfile::private_key(&mut io::BufReader::new(std::fs::File::open(&key_path)?))?
        .ok_or_else(|| io::Error::other("no private key in cutd.key"))?;
    Ok((certs, key))
}

/// The SHA-256 of the DER certificate, which is what a client pins. Printed at
/// startup so whoever is pairing can read it off the Pi's console.
pub fn fingerprint(cert: &rustls::pki_types::CertificateDer<'_>) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(cert.as_ref());
    digest.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

/// Serve until the process ends.
///
/// `allow_public_bind` is the explicit override for the LAN-only default. It is a
/// parameter rather than a config key so that reaching the internet takes a
/// deliberate act at the command line, not a line someone copied into a file.
pub fn serve(host: Arc<Host>, config: Config, allow_public_bind: bool) -> io::Result<()> {
    if !config.is_private_bind() && !allow_public_bind {
        return Err(io::Error::other(format!(
            "{} is not a private address. A Cut Host can make a blade move; pass --allow-public-bind if you \
             really mean to expose it.",
            config.bind
        )));
    }

    let (certs, _) = load_or_make_cert(&config.cert_dir)?;
    eprintln!("cut host: protocol version {PROTOCOL_VERSION}");
    eprintln!(
        "cut host: certificate fingerprint {}",
        fingerprint(&certs[0])
    );

    let listener = TcpListener::bind(config.bind)?;
    eprintln!("cut host: listening on {}", config.bind);
    serve_on(listener, host, config)
}

/// Serve on a listener somebody else bound, with no bind-address guard — the
/// caller already chose the address. Split out from `serve` so a test can take
/// port 0 and read back the port the OS gave it.
pub fn serve_on(listener: TcpListener, host: Arc<Host>, config: Config) -> io::Result<()> {
    let (certs, key) = load_or_make_cert(&config.cert_dir)?;
    let tls = Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(io::Error::other)?,
    );

    let clients = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let tokens = Arc::new(config.tokens);
    let max_frame = config.max_frame;

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if clients.load(std::sync::atomic::Ordering::SeqCst) >= MAX_CLIENTS {
            continue; // dropped without ceremony; a real client retries
        }
        let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_else(|_| "?".into());
        enable_keepalive(&stream, &peer);
        clients.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let (host, tls, tokens, clients) = (host.clone(), tls.clone(), tokens.clone(), clients.clone());
        thread::spawn(move || {
            if let Err(e) = serve_client(stream, tls, &host, &*tokens, max_frame) {
                eprintln!("cut host: client ended: {e}");
            }
            clients.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        });
    }
    Ok(())
}

/// The fingerprint of the certificate in `dir`, generating it if it is not there
/// yet. What a pairing client is shown, and what `cuthulhu-cutd` prints at
/// startup.
pub fn fingerprint_of_cert_dir(dir: &Path) -> io::Result<String> {
    let (certs, _) = load_or_make_cert(dir)?;
    Ok(fingerprint(&certs[0]))
}

fn serve_client(
    stream: std::net::TcpStream,
    tls: Arc<rustls::ServerConfig>,
    host: &Arc<Host>,
    tokens: &BTreeMap<String, String>,
    max_frame: usize,
) -> io::Result<()> {
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_else(|_| "?".into());

    // The frame layer re-checks its deadline whenever a read comes back empty, so the socket
    // needs to come back empty rather than block indefinitely. This value only sets how promptly
    // a stalled frame is noticed; `DEFAULT_BODY_TIMEOUT` decides how long one is tolerated.
    stream
        .set_read_timeout(Some(SOCKET_POLL_INTERVAL))
        .map_err(|e| io::Error::other(format!("could not set a read timeout: {e}")))?;
    // The same, for the direction this worker writes in. Without it a client that authenticates
    // and then stops reading pins this thread — and its `MAX_CLIENTS` slot — in `write_frame` as
    // soon as the queued events outgrow the socket buffer. Keepalive does not reach that peer
    // either: it is alive and acknowledging, it has just stopped draining.
    stream
        .set_write_timeout(Some(SOCKET_POLL_INTERVAL))
        .map_err(|e| io::Error::other(format!("could not set a write timeout: {e}")))?;

    let conn = rustls::ServerConnection::new(tls).map_err(io::Error::other)?;
    let mut tls_stream = rustls::StreamOwned::new(conn, stream);

    // The version before the token, and the token before anything else. An
    // unauthenticated frame must never reach a device. Both reads share one budget
    // from the moment the connection was accepted: the version offer is not a
    // second chance to hold the slot.
    let unauth = Instant::now() + PRE_AUTH_TIMEOUT;
    if let Err(e) = establish_version(&mut tls_stream, unauth, PROTOCOL_VERSION) {
        eprintln!("cut host: {peer} {e}");
        return Err(io::Error::other(e.to_string()));
    }
    let remaining = unauth.saturating_duration_since(Instant::now());
    let presented: String =
        read_frame(&mut tls_stream, 1024, Some(remaining), remaining).map_err(io::Error::other)?;
    let Some(client) = match_token(&presented, tokens) else {
        eprintln!("cut host: {peer} presented a token matching no client");
        thread::sleep(std::time::Duration::from_secs(2));
        // Said out loud rather than closed silently. A dropped connection reaches the operator as
        // "the host could not be reached", which is what an asleep Pi looks like too — and the two
        // call for opposite actions: re-pair with the token from `cutd.toml`, versus wait (#112).
        // After the delay, so naming the failure does not also make it quicker to work through.
        let _ = write_frame(&mut tls_stream, &Response::Unauthorized, PRE_AUTH_TIMEOUT);
        return Err(io::Error::other("bad token"));
    };
    eprintln!("cut host: {peer} authenticated as `{client}`");
    write_frame(&mut tls_stream, &Response::Ok, DEFAULT_BODY_TIMEOUT).map_err(io::Error::other)?;

    // Events for every cutter go out on this connection; requests come back on it.
    // ponytail: they are written just before each reply rather than pushed the
    // moment they happen, so a client that asks nothing hears nothing. That suits a
    // desktop that polls `Snapshot`; give events a connection of their own when a UI
    // wants them pushed.
    let events = host.subscribe();

    loop {
        // Drain whatever the cutters have said before waiting on the client again.
        while let Ok(event) = events.try_recv() {
            write_frame(&mut tls_stream, &event, DEFAULT_BODY_TIMEOUT).map_err(io::Error::other)?;
        }
        // `None` buys the client an unbounded wait for its next request to *begin*, and nothing
        // more: it is idle between polls, not stalled mid-frame, and must not be dropped for
        // waiting. From the first byte on, `read_frame` holds it to `DEFAULT_BODY_TIMEOUT` —
        // otherwise a client that sends one length byte and stops keeps this worker, and one of
        // the `MAX_CLIENTS` slots, until the daemon is restarted. Keepalive does not reach that
        // peer: it is alive and acknowledging, it just stopped talking.
        match read_frame::<_, Request>(&mut tls_stream, max_frame, None, DEFAULT_BODY_TIMEOUT) {
            Ok(request) => {
                if let Request::Dispatch { ref device, .. } = request {
                    eprintln!("cut host: `{client}` dispatched to {device}");
                }
                let response = handle_request(host, request);
                write_frame(&mut tls_stream, &response, DEFAULT_BODY_TIMEOUT)
                    .map_err(io::Error::other)?;
            }
            Err(FrameError::Eof) => return Ok(()),
            Err(e) => return Err(io::Error::other(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{read_frame, write_frame, SOCKET_POLL_INTERVAL};
    use crate::host::testing::{TwoCutterFactory, CAMEO};
    use crate::protocol::{VersionReply, PROTOCOL_VERSION, VERSION_FRAME_MAX};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn list_devices_answers_with_every_cutter() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        match handle_request(&host, Request::ListDevices) {
            Response::Devices(d) => assert_eq!(d.len(), 2),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn snapshot_answers_with_every_cutters_status() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        match handle_request(&host, Request::Snapshot) {
            Response::Snapshots(s) => {
                assert_eq!(s.len(), 2);
                assert!(s.iter().all(|s| s.status.phase == driver_core::Phase::Idle));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn a_refused_dispatch_answers_refused_rather_than_dropping_the_connection() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        let request = Request::Dispatch {
            dispatch_id: crate::protocol::DispatchId("d-1".into()),
            device: "usb:9:9".into(),
            machine_id: "cameo5".into(),
            passes: Vec::new(),
        };
        assert!(matches!(handle_request(&host, request), Response::Refused(_)));
    }

    #[test]
    fn an_unknown_cutter_refuses_every_verb_over_the_wire() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        for request in [
            Request::Cancel { device: "usb:9:9".into() },
            Request::Resume { device: "usb:9:9".into() },
            Request::ConfirmPassDone { device: "usb:9:9".into() },
            Request::Reconnect { device: "usb:9:9".into() },
        ] {
            assert!(matches!(handle_request(&host, request), Response::Refused(_)));
        }
    }

    #[test]
    fn a_cancel_on_a_known_cutter_answers_ok() {
        let host = Host::start(Arc::new(TwoCutterFactory));
        assert!(matches!(handle_request(&host, Request::Cancel { device: CAMEO.into() }), Response::Ok));
    }

    fn tokens() -> std::collections::BTreeMap<String, String> {
        [("workshop-laptop".to_string(), "aaa".to_string()),
         ("office-desktop".to_string(), "bbb".to_string())]
            .into_iter()
            .collect()
    }

    #[test]
    fn a_token_matches_only_itself_and_reports_which_name_it_was() {
        assert_eq!(match_token("aaa", &tokens()).as_deref(), Some("workshop-laptop"));
        assert_eq!(match_token("bbb", &tokens()).as_deref(), Some("office-desktop"));
        assert_eq!(match_token("aab", &tokens()), None);
        assert_eq!(match_token("aa", &tokens()), None, "a prefix is not a match");
        assert_eq!(match_token("", &tokens()), None);
    }

    /// Not a test that a dead peer is actually reaped — that needs a suspended machine or a
    /// firewall rule to fake convincingly, and a mock would only test the mock. This instead
    /// catches the regression that matters day to day: someone deleting the `enable_keepalive`
    /// call from the accept loop.
    #[test]
    fn an_accepted_stream_has_keepalive_enabled() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = std::net::TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();

        enable_keepalive(&server, "test");

        assert!(socket2::SockRef::from(&server).keepalive().unwrap());
    }

    /// A version both sides speak is agreed, and the reply is the agreement rather than a
    /// `Response` — nothing version-specific has a reason to be on this frame.
    #[test]
    fn the_version_this_host_speaks_is_agreed() {
        let (mut client, reply) = talk_version(&PROTOCOL_VERSION, Duration::from_secs(2));
        assert!(reply.is_ok(), "{reply:?}");
        let back: VersionReply = read_frame(
            &mut client,
            VERSION_FRAME_MAX,
            Some(Duration::from_secs(2)),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(
            back,
            VersionReply::Agreed {
                version: PROTOCOL_VERSION
            }
        );
    }

    /// Named, then done. The offer is a different version, not a payload, and the reply carries
    /// the two numbers and nothing a `Request` would.
    #[test]
    fn an_unsupported_version_is_refused_by_name() {
        let offered = PROTOCOL_VERSION + 1;
        let (mut client, reply) = talk_version(&offered, Duration::from_secs(2));
        assert!(matches!(
            reply,
            Err(VersionFailure::Unsupported { offered: n, .. }) if n == offered
        ));
        let back: VersionReply = read_frame(
            &mut client,
            VERSION_FRAME_MAX,
            Some(Duration::from_secs(2)),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(
            back,
            VersionReply::Unsupported {
                offered,
                supported: PROTOCOL_VERSION
            }
        );
    }

    /// What an unversioned client sent first: a token string. It is not a version, so it is
    /// refused without being read as a `Request` — a `Dispatch` in that frame would be how a
    /// replayed session started a Job.
    #[test]
    fn a_frame_that_is_not_a_version_is_refused_without_being_decoded_as_a_request() {
        let (mut client, reply) = talk_version(&"test-token".to_string(), Duration::from_secs(2));
        assert!(
            matches!(reply, Err(VersionFailure::Unreadable)),
            "{reply:?}"
        );
        let back: VersionReply = read_frame(
            &mut client,
            VERSION_FRAME_MAX,
            Some(Duration::from_secs(2)),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(back, VersionReply::Unreadable);
    }

    /// The length is the dispatch cap, which a later frame may use. The version frame may not,
    /// and the body is not read to find that out: the script has no body, so a reader that
    /// allocated it would end on `Io` rather than `TooLarge`.
    #[test]
    fn an_oversized_version_frame_is_refused_before_its_body_is_read() {
        let mut header = Vec::new();
        header.extend_from_slice(&(crate::frame::DEFAULT_MAX_FRAME as u32).to_be_bytes());
        let (mut client, mut server) = pair();
        let deadline = Instant::now() + Duration::from_secs(2);
        client.write_all(&header).unwrap();
        let err = establish_version(&mut server, deadline, PROTOCOL_VERSION).unwrap_err();
        assert!(
            matches!(err, VersionFailure::TooLarge { len } if len == crate::frame::DEFAULT_MAX_FRAME),
            "{err:?}"
        );
        let back: VersionReply = read_frame(
            &mut client,
            VERSION_FRAME_MAX,
            Some(Duration::from_secs(2)),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(back, VersionReply::Unreadable);
    }

    /// The unauthenticated budget is a deadline, not a suggestion. A peer that accepts the
    /// connection and then says nothing must not hold the reader.
    #[test]
    fn a_version_that_never_arrives_times_out() {
        let (client, mut server) = pair();
        // Kept open so the read blocks instead of seeing EOF.
        let _hold = client.try_clone().unwrap();
        let budget = Duration::from_millis(200);
        let started = Instant::now();
        let err =
            establish_version(&mut server, Instant::now() + budget, PROTOCOL_VERSION).unwrap_err();
        assert!(matches!(err, VersionFailure::Timeout), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "waited {:?}",
            started.elapsed()
        );
    }

    /// A peer that closes after one byte of the length has begun a frame and then left. That is
    /// a fault, not an agreed version, and it admits nothing because nothing past it runs.
    #[test]
    fn a_peer_that_disconnects_mid_version_is_not_agreed() {
        let (mut client, mut server) = pair();
        client.write_all(&[0u8]).unwrap();
        drop(client);
        let err =
            establish_version(&mut server, Instant::now() + Duration::from_secs(2), PROTOCOL_VERSION)
                .unwrap_err();
        assert!(matches!(err, VersionFailure::Io(_)), "{err:?}");
    }

    fn pair() -> (
        std::os::unix::net::UnixStream,
        std::os::unix::net::UnixStream,
    ) {
        let (client, server) = std::os::unix::net::UnixStream::pair().unwrap();
        client.set_read_timeout(Some(SOCKET_POLL_INTERVAL)).unwrap();
        client
            .set_write_timeout(Some(SOCKET_POLL_INTERVAL))
            .unwrap();
        server.set_read_timeout(Some(SOCKET_POLL_INTERVAL)).unwrap();
        server
            .set_write_timeout(Some(SOCKET_POLL_INTERVAL))
            .unwrap();
        (client, server)
    }

    /// Write `offer` as a frame from the client side and run the agreement on the server side.
    fn talk_version(
        offer: &impl serde::Serialize,
        budget: Duration,
    ) -> (std::os::unix::net::UnixStream, Result<(), VersionFailure>) {
        let (mut client, mut server) = pair();
        write_frame(&mut client, offer, budget).unwrap();
        let reply = establish_version(&mut server, Instant::now() + budget, PROTOCOL_VERSION);
        (client, reply)
    }

    /// Revoking one client must leave the others working — the property the whole change exists
    /// for, and the one a shared key cannot offer.
    #[test]
    fn removing_one_token_leaves_the_others_working() {
        let mut remaining = tokens();
        remaining.remove("workshop-laptop");
        assert_eq!(match_token("aaa", &remaining), None, "the revoked client is out");
        assert_eq!(match_token("bbb", &remaining).as_deref(), Some("office-desktop"));
    }
}
