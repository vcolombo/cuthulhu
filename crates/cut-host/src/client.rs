// SPDX-License-Identifier: GPL-3.0-or-later
//! What a desktop holds to reach a Cut Host.
//!
//! One TLS connection carries every cutter on the host: requests out, events in.
//! The certificate is pinned by fingerprint rather than validated by an authority,
//! because a Pi on a home network has no name an authority would sign.

use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use driver_core::manager::CutPass;
use driver_core::DeviceInfo;

use crate::frame::{read_frame, write_frame, FrameError};
use crate::protocol::{DeviceSnapshot, DispatchId, Event, Refusal, Request, Response};

#[derive(Debug)]
pub enum ClientError {
    Refused(Refusal),
    Fingerprint { expected: String, found: String },
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
            ClientError::Fingerprint { expected, found } =>
                write!(f, "this host presented a different certificate than the one paired \
                           (expected {expected}, found {found})"),
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

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// How long to wait for a Cut Host to accept a connection.
///
/// A host that refuses is instant; one that is silently unreachable — a dropped SYN, a
/// firewall discarding rather than refusing — would otherwise block for the OS default,
/// which is tens of seconds. The desktop holds a lock across this call while listing
/// devices, so an unbounded wait here is a frozen device list.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct HostClient {
    /// Serialized: one request and its reply at a time. A Cut Host answers a
    /// handful of clients and a desktop makes one call at a time, so a lock is the
    /// whole of the concurrency story here.
    stream: Mutex<Tls>,
}

impl HostClient {
    pub fn connect(addr: &str, token: &str, pinned_fingerprint: &str) -> Result<HostClient, ClientError> {
        let verifier = Arc::new(PinnedCert {
            fingerprint: pinned_fingerprint.to_string(),
            seen: Mutex::new(None),
        });
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone())
            .with_no_client_auth();

        // ponytail: `to_socket_addrs()` resolves DNS/mDNS synchronously and std gives no knob to
        // bound it — a Cut Host is addressed by name (`cuthulhu-pi.local:7878`), not by literal
        // IP, so this is the common path, not an edge case. `CONNECT_TIMEOUT` below covers the
        // connect itself; a hung resolver is a real, if rarer, way this can still block. Bound
        // it too (a helper thread, or a crate with an async resolver) if that turns out to bite.
        let mut addrs = addr.to_socket_addrs().map_err(|e| ClientError::Transport(e.to_string()))?;
        let sock_addr = addrs.next()
            .ok_or_else(|| ClientError::Transport(format!("`{addr}` resolved to no address")))?;
        let tcp = TcpStream::connect_timeout(&sock_addr, CONNECT_TIMEOUT)
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        tcp.set_read_timeout(Some(crate::frame::SOCKET_POLL_INTERVAL))
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        let server_name = rustls::pki_types::ServerName::try_from("cuthulhu-cutd")
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        let conn = rustls::ClientConnection::new(Arc::new(config), server_name)
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        let mut stream = rustls::StreamOwned::new(conn, tcp);

        write_frame(&mut stream, &token.to_string()).map_err(|e| {
            // A handshake that failed on the pin reaches here as an I/O error, so
            // the more useful message is the one the verifier can give.
            match verifier.seen.lock().unwrap().clone() {
                Some(found) if found != pinned_fingerprint =>
                    ClientError::Fingerprint { expected: pinned_fingerprint.to_string(), found },
                _ => ClientError::Transport(e.to_string()),
            }
        })?;
        match read_frame::<_, Response>(
            &mut stream,
            4096,
            Some(crate::frame::DEFAULT_BODY_TIMEOUT),
            crate::frame::DEFAULT_BODY_TIMEOUT,
        ) {
            Ok(Response::Ok) => {}
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
        match self.call(Request::ListDevices)? {
            Response::Devices(d) => Ok(d),
            other => Err(unexpected(other)),
        }
    }

    pub fn snapshots(&self) -> Result<Vec<DeviceSnapshot>, ClientError> {
        match self.call(Request::Snapshot)? {
            Response::Snapshots(s) => Ok(s),
            other => Err(unexpected(other)),
        }
    }

    pub fn dispatch(
        &self,
        dispatch_id: DispatchId,
        device: &str,
        machine_id: &str,
        passes: Vec<CutPass>,
    ) -> Result<(), ClientError> {
        match self.call(Request::Dispatch {
            dispatch_id,
            device: device.to_string(),
            machine_id: machine_id.to_string(),
            passes,
        })? {
            Response::Accepted { .. } => Ok(()),
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

    fn expect_ok(&self, request: Request) -> Result<(), ClientError> {
        match self.call(request)? {
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
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        let mut stream = self.stream.lock().unwrap();
        write_frame(&mut *stream, &request).map_err(|e| ClientError::Transport(e.to_string()))?;
        loop {
            // A reply is owed the moment the request above was written: a Pi that goes silent
            // from here must not hold this mutex, and every other call, forever.
            match read_frame::<_, Incoming>(
                &mut *stream,
                crate::frame::DEFAULT_MAX_FRAME,
                Some(crate::frame::DEFAULT_BODY_TIMEOUT),
                crate::frame::DEFAULT_BODY_TIMEOUT,
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
}
