// SPDX-License-Identifier: GPL-3.0-or-later
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Duration;

use crate::frame::{
    read_frame, write_frame, FrameError, DEFAULT_MAX_FRAME, SOCKET_POLL_INTERVAL,
};
use crate::protocol::{Request, Response, VersionReply, PROTOCOL_VERSION, VERSION_FRAME_MAX};

/// Where a misbehaving host is listening, and the fingerprint a client must pin to reach it.
pub struct MisbehavingHost {
    pub addr: String,
    pub fingerprint: String,
}

/// The budget this peer hands each `read_frame` — twice over, in fact, since `read_frame` spends
/// one waiting for a frame to begin and a fresh one on the rest of it. Not a ceiling on how long
/// a frame can take: a deadline is only checked between reads, so a read already under way still
/// spends its `SOCKET_POLL_INTERVAL`, and decoding happens outside both. That is fine, because
/// this bounds nothing a test asserts on — it is only here so a client that stops talking does
/// not hold the thread for the whole run.
const BUDGET: Duration = Duration::from_secs(10);

/// Answers `replies` in order: the first as the greeting a token earns, then one per request.
///
/// Binds port 0 so tests run concurrently, and replays the same script on every connection, so
/// a client that redials is answered rather than left hanging. Speaks `PROTOCOL_VERSION`:
/// a client that has not agreed it never reaches the script.
pub fn start_host_answering(replies: Vec<Response>) -> MisbehavingHost {
    start_host_at_version(PROTOCOL_VERSION, replies)
}

/// A host that speaks `supported` and, when that is the client's version, the script.
///
/// The agreement is `establish_version`, the function the daemon runs, with the version
/// this peer is pretending to speak. A second copy of the handshake used to answer every
/// read error with `Unreadable`, including a timeout the real host leaves unanswered.
pub fn start_host_at_version(supported: u32, replies: Vec<Response>) -> MisbehavingHost {
    spawn_host(move |mut tls| {
        let deadline = std::time::Instant::now() + BUDGET;
        if crate::serve::establish_version(&mut tls, deadline, supported).is_err() {
            return;
        }
        answer_script(&mut tls, &replies);
    })
}

/// What a daemon from before protocol versions does with the offer: it reads the first
/// frame as a token, the offer is not one, and it closes without a reply.
pub fn start_unversioned_host() -> MisbehavingHost {
    spawn_host(|mut tls| {
        let _ = read_frame::<_, String>(&mut tls, 1024, Some(BUDGET), BUDGET);
    })
}

/// Reads the version offer and then holds the socket open. A close here is an older host;
/// silence is a stall, and the two are different failures for the operator.
pub fn start_host_silent_after_the_version_offer() -> MisbehavingHost {
    spawn_host(|mut tls| {
        if read_frame::<_, u32>(&mut tls, VERSION_FRAME_MAX, Some(BUDGET), BUDGET).is_err() {
            return;
        }
        // Until the client gives up. Returning would drop the socket, and a dropped
        // socket is `Eof`, which the client reads as an older protocol.
        let _ = read_frame::<_, String>(
            &mut tls,
            1024,
            Some(Duration::from_secs(30)),
            Duration::from_secs(30),
        );
    })
}

/// Writes `VersionReply::Unreadable` and reports whether a token frame followed it.
///
/// The client must not send the credential once the reply is not an agreement. The
/// flag is that observation: the error variant alone is also what a later, failed
/// greeting read produces, so a client that treated `Unreadable` as agreed and then
/// lost the connection would still look like a protocol mismatch.
pub fn start_host_replying_unreadable() -> (MisbehavingHost, Receiver<bool>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let host = spawn_host(move |mut tls| {
        let saw_token = read_frame::<_, u32>(&mut tls, VERSION_FRAME_MAX, Some(BUDGET), BUDGET)
            .is_ok()
            && write_frame(&mut tls, &VersionReply::Unreadable, BUDGET).is_ok()
            && read_frame::<_, String>(
                &mut tls,
                1024,
                Some(Duration::from_secs(2)),
                Duration::from_secs(2),
            )
            .is_ok();
        let _ = tx.send(saw_token);
    });
    (host, rx)
}

/// Reads the version offer, writes `bytes` and nothing else. How a reply that is not a
/// `VersionReply` — including one whose length is past the version cap — reaches the client.
pub fn start_host_writing_version_reply(bytes: Vec<u8>) -> MisbehavingHost {
    spawn_host(move |mut tls| {
        if read_frame::<_, u32>(&mut tls, VERSION_FRAME_MAX, Some(BUDGET), BUDGET).is_err() {
            return;
        }
        let _ = tls.write_all(&bytes);
    })
}

/// A TLS connection on which no frame has been sent yet. The real `HostClient` agrees a
/// version itself; tests that must send something else start here.
pub struct RawSession {
    stream: super::Tls,
}

impl RawSession {
    pub fn open(addr: &str, fingerprint: &str) -> Result<RawSession, super::ClientError> {
        let deadline = std::time::Instant::now() + super::CONNECT_TIMEOUT;
        // The pin is checked on the first write, the same as `connect_within`. This session
        // has not written yet, so the verifier has nothing to report.
        let (stream, _verifier) = super::dial(addr, fingerprint, deadline)?;
        Ok(RawSession { stream })
    }

    pub fn send(
        &mut self,
        value: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<(), FrameError> {
        write_frame(&mut self.stream, value, timeout)
    }

    pub fn recv<T: serde::de::DeserializeOwned>(
        &mut self,
        max: usize,
        timeout: Duration,
    ) -> Result<T, FrameError> {
        read_frame(&mut self.stream, max, Some(timeout), timeout)
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        self.stream.write_all(bytes)
    }
}

fn spawn_host<F>(handle: F) -> MisbehavingHost
where
    F: Fn(rustls::StreamOwned<rustls::ServerConnection, TcpStream>) + Send + Sync + 'static,
{
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
    let addr = listener
        .local_addr()
        .expect("the port the OS chose")
        .to_string();
    let handle = Arc::new(handle);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let (tls, handle) = (tls.clone(), handle.clone());
            std::thread::spawn(move || {
                // The frame layer re-checks its deadline whenever a read comes back empty, so
                // the socket has to come back empty rather than block — the pacing
                // `serve_client` sets, for the same reason. Unwrapped rather than ignored so
                // the cause is named: this worker is detached, so a fixture blocked in a read
                // does not hang the test — the client's own deadline fires and the test fails
                // as a timeout, with nothing to say why. The panic prints the reason instead.
                stream
                    .set_read_timeout(Some(SOCKET_POLL_INTERVAL))
                    .expect("a read timeout on a loopback socket");
                stream
                    .set_write_timeout(Some(SOCKET_POLL_INTERVAL))
                    .expect("a write timeout on a loopback socket");
                let Ok(conn) = rustls::ServerConnection::new(tls) else {
                    return;
                };
                handle(rustls::StreamOwned::new(conn, stream));
            });
        }
    });
    MisbehavingHost { addr, fingerprint }
}

/// The token and the scripted replies. Reached only after a version was agreed — a client
/// that skips that step never gets a greeting, which is the cutover.
///
/// The token is read and discarded: a client that got this far already pinned the
/// certificate, and nothing here is proving anything about tokens.
fn answer_script(
    tls: &mut rustls::StreamOwned<rustls::ServerConnection, TcpStream>,
    replies: &[Response],
) {
    if read_frame::<_, String>(tls, 1024, Some(BUDGET), BUDGET).is_err() {
        return;
    }
    for reply in replies {
        if write_frame(tls, reply, BUDGET).is_err() {
            return;
        }
        // Each reply after the greeting waits for the request it answers, so none can land
        // ahead of it and be read as the answer to something else.
        if read_frame::<_, Request>(tls, DEFAULT_MAX_FRAME, Some(BUDGET), BUDGET).is_err() {
            return;
        }
    }
}
