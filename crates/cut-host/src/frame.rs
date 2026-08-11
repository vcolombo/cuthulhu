// SPDX-License-Identifier: GPL-3.0-or-later

//! Length-prefixed JSON frames.
//!
//! The length is a big-endian `u32` and it is checked against the cap *before* the
//! body is read. A Cut Host runs on a Pi with a gigabyte of RAM, so a header
//! claiming more than the cap must cost nothing to refuse.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

/// 32 MiB. A large cut is megabytes of polylines as JSON text; this leaves room
/// for that and refuses anything that could only be an attack or a bug.
pub const DEFAULT_MAX_FRAME: usize = 32 * 1024 * 1024;

/// How long a frame has to finish once its header has arrived. Generous: a large cut is
/// megabytes of JSON over a home network, and this is a fault deadline rather than a
/// performance target.
pub const DEFAULT_BODY_TIMEOUT: Duration = Duration::from_secs(30);

/// How often a blocked read wakes to re-check its deadline. The socket's own `SO_RCVTIMEO`,
/// set by whoever owns the connection; the value only decides how promptly a stalled frame
/// is noticed, not how long it is tolerated.
pub const SOCKET_POLL_INTERVAL: Duration = Duration::from_secs(1);

const CLOSED_MID_FRAME: &str = "the peer closed part-way through a frame";

#[derive(Debug)]
pub enum FrameError {
    /// The peer closed cleanly between frames. Not a fault.
    Eof,
    /// A frame began and did not finish inside its deadline.
    Timeout,
    TooLarge { len: usize, max: usize },
    Io(String),
    Malformed(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Eof => write!(f, "the peer closed the connection"),
            FrameError::Timeout => write!(f, "a frame began and did not finish in time"),
            FrameError::TooLarge { len, max } =>
                write!(f, "a frame declared {len} bytes, over the {max} byte limit"),
            FrameError::Io(m) => write!(f, "the connection failed ({m})"),
            FrameError::Malformed(m) => write!(f, "a frame could not be read ({m})"),
        }
    }
}
impl std::error::Error for FrameError {}

pub fn write_frame<W: Write>(w: &mut W, value: &impl Serialize) -> io::Result<()> {
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    let len = u32::try_from(body.len())
        .map_err(|_| io::Error::other(format!("frame of {} bytes exceeds u32", body.len())))?;
    w.write_all(&len.to_be_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

/// Fill `buf` completely, or fail — retrying reads that merely found no data yet, and giving up
/// when `deadline` passes.
///
/// `read_exact` cannot be used here. On a socket carrying `SO_RCVTIMEO` a quiet moment surfaces
/// as `WouldBlock`, and `read_exact` does not say how much it consumed before failing, so a retry
/// would resume mid-frame and corrupt it. This tracks its own fill so a retry is safe.
///
/// `deadline` of `None` waits forever, which is what waiting for a frame to *begin* must do: a
/// client that polls once a second is idle in between and must not be dropped for it.
///
/// The retry is paced by the socket, not by this loop: callers set `SO_RCVTIMEO`
/// (`SOCKET_POLL_INTERVAL`), so a quiet read blocks for that long before returning `WouldBlock`.
/// On a reader with no timeout at all this would spin, which is why both call sites set one.
fn fill(r: &mut impl Read, buf: &mut [u8], deadline: Option<Instant>) -> Result<(), FrameError> {
    let mut filled = 0;
    while filled < buf.len() {
        // Checked before every read, not just a stalled one: a peer that trickles a byte at a
        // time, always just under `SOCKET_POLL_INTERVAL`, never sees `WouldBlock` — the deadline
        // has to bound the whole fill, not only the retries.
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(FrameError::Timeout);
        }
        match r.read(&mut buf[filled..]) {
            Ok(0) => {
                return Err(if filled == 0 {
                    FrameError::Eof
                } else {
                    // Not `Eof`: a caller loops on `Eof` meaning "the peer left between frames",
                    // and a peer that vanished mid-frame left something behind.
                    FrameError::Io(CLOSED_MID_FRAME.into())
                })
            }
            Ok(n) => filled += n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(FrameError::Io(e.to_string())),
        }
    }
    Ok(())
}

/// Read one frame. `header_timeout` bounds the wait for it to *begin*; `body_timeout` bounds the
/// rest of it, header and body alike, once the first byte has arrived.
///
/// The rule is not "headers are unbounded, bodies are not" — it is that a frame that is *owed*
/// has a deadline, and a frame that may never come does not. A daemon's request loop reading the
/// next request from an attached client owes nothing *before the first byte* — a desktop polling
/// once a second is idle in between and must not be dropped for it — so that call site passes
/// `None`. Everywhere else a frame was promised: a token after a connection was accepted, a
/// greeting after a token was sent, a reply after a request was sent. A peer that goes silent
/// there would otherwise hold this reader, and whatever lock is above it, forever.
pub fn read_frame<R: Read, T: DeserializeOwned>(
    r: &mut R,
    max: usize,
    header_timeout: Option<Duration>,
    body_timeout: Duration,
) -> Result<T, FrameError> {
    let mut header = [0u8; 4];
    // The header is filled in two goes because the first byte is what turns "may never come" into
    // "owed": a client that sends one length byte and stops has begun a frame, and waiting out the
    // other three with no deadline holds this reader — and, in the daemon, the one client slot in
    // eight above it — for the life of the process. Keepalive does not reach that peer; it is
    // alive and acknowledging, just not talking.
    fill(r, &mut header[..1], header_timeout.map(|d| Instant::now() + d))?;
    fill(r, &mut header[1..], Some(Instant::now() + body_timeout)).map_err(|e| match e {
        // A caller loops on `Eof` meaning "the peer left between frames"; past the first byte it
        // left mid-frame instead.
        FrameError::Eof => FrameError::Io(CLOSED_MID_FRAME.into()),
        other => other,
    })?;

    let len = u32::from_be_bytes(header) as usize;
    // Before the allocation, not after: the whole point of the cap.
    if len > max {
        return Err(FrameError::TooLarge { len, max });
    }
    let mut body = vec![0u8; len];
    fill(r, &mut body, Some(Instant::now() + body_timeout))?;
    serde_json::from_slice(&body).map_err(|e| FrameError::Malformed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips_through_a_pipe() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &"hello".to_string()).unwrap();
        write_frame(&mut buf, &42u32).unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let first: String =
            read_frame(&mut cursor, DEFAULT_MAX_FRAME, Some(DEFAULT_BODY_TIMEOUT), DEFAULT_BODY_TIMEOUT)
                .unwrap();
        let second: u32 =
            read_frame(&mut cursor, DEFAULT_MAX_FRAME, Some(DEFAULT_BODY_TIMEOUT), DEFAULT_BODY_TIMEOUT)
                .unwrap();
        assert_eq!(first, "hello");
        assert_eq!(second, 42);
    }

    #[test]
    fn a_clean_end_of_stream_is_eof_not_an_error() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        assert!(matches!(
            read_frame::<_, String>(
                &mut cursor,
                DEFAULT_MAX_FRAME,
                Some(DEFAULT_BODY_TIMEOUT),
                DEFAULT_BODY_TIMEOUT
            ),
            Err(FrameError::Eof)
        ));
    }

    /// The cap is checked before the body is read from the stream. Demonstrated
    /// by supplying a header with no body — a reader that read the body first
    /// would return `Io` from the unexpected end of stream rather than `TooLarge`.
    #[test]
    fn an_oversized_length_is_refused_before_the_body_is_read() {
        let mut framed: Vec<u8> = Vec::new();
        framed.extend_from_slice(&(9_000_000u32).to_be_bytes());
        // deliberately no body

        let mut cursor = std::io::Cursor::new(framed);
        match read_frame::<_, String>(&mut cursor, 1024, Some(DEFAULT_BODY_TIMEOUT), DEFAULT_BODY_TIMEOUT) {
            Err(FrameError::TooLarge { len, max }) => {
                assert_eq!(len, 9_000_000);
                assert_eq!(max, 1024);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn a_truncated_body_is_malformed_not_a_hang() {
        let mut framed: Vec<u8> = Vec::new();
        framed.extend_from_slice(&(64u32).to_be_bytes());
        framed.extend_from_slice(b"{\"partial\":");

        let mut cursor = std::io::Cursor::new(framed);
        assert!(matches!(
            read_frame::<_, String>(
                &mut cursor,
                DEFAULT_MAX_FRAME,
                Some(DEFAULT_BODY_TIMEOUT),
                DEFAULT_BODY_TIMEOUT
            ),
            Err(FrameError::Io(_)) | Err(FrameError::Malformed(_))
        ));
    }

    #[test]
    fn a_body_that_is_not_the_expected_shape_is_malformed() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &"a string".to_string()).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        assert!(matches!(
            read_frame::<_, u32>(
                &mut cursor,
                DEFAULT_MAX_FRAME,
                Some(DEFAULT_BODY_TIMEOUT),
                DEFAULT_BODY_TIMEOUT
            ),
            Err(FrameError::Malformed(_))
        ));
    }

    use std::io::Cursor;
    use std::time::Duration;

    /// A reader that yields its script and then reports `WouldBlock` forever, as a socket with
    /// `SO_RCVTIMEO` does when the peer has stopped talking without closing.
    ///
    /// The sleep matters: a real socket blocks for its timeout before reporting `WouldBlock`, and
    /// that is what paces `fill`'s retry loop. A fake that answered instantly would make the loop
    /// spin a core and would misrepresent what the code does in production.
    struct StallsAfter {
        given: Cursor<Vec<u8>>,
    }
    impl StallsAfter {
        fn new(bytes: Vec<u8>) -> StallsAfter {
            StallsAfter { given: Cursor::new(bytes) }
        }
    }
    impl Read for StallsAfter {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.given.read(buf)? {
                0 => {
                    std::thread::sleep(Duration::from_millis(20));
                    Err(io::Error::new(io::ErrorKind::WouldBlock, "no data yet"))
                }
                n => Ok(n),
            }
        }
    }

    /// A reader that is silent for a while and then delivers its frame — a connection that was
    /// idle between polls, which is the normal case and must not be a fault.
    struct QuietThenSpeaks {
        quiet_reads_left: usize,
        given: Cursor<Vec<u8>>,
    }
    impl Read for QuietThenSpeaks {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.quiet_reads_left > 0 {
                self.quiet_reads_left -= 1;
                std::thread::sleep(Duration::from_millis(20));
                return Err(io::Error::new(io::ErrorKind::WouldBlock, "no data yet"));
            }
            self.given.read(buf)
        }
    }

    fn framed(value: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        write_frame(&mut buf, &value.to_string()).unwrap();
        buf
    }

    /// The failure this task exists for: a peer that sends a header and then stops must not hold
    /// the reader forever. Before this change the read blocked with no deadline at all.
    #[test]
    fn a_body_that_never_arrives_times_out_rather_than_blocking() {
        let mut header_only = Vec::new();
        header_only.extend_from_slice(&(64u32).to_be_bytes());

        let started = std::time::Instant::now();
        let result = read_frame::<_, String>(
            &mut StallsAfter::new(header_only),
            DEFAULT_MAX_FRAME,
            Some(DEFAULT_BODY_TIMEOUT),
            Duration::from_millis(200),
        );
        assert!(matches!(result, Err(FrameError::Timeout)), "got {result:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "it waited far past its deadline");
    }

    /// The request loop's own case, and the property the `None` exists to protect: reading the
    /// *next* request from an attached client owes nothing, because a client that polls once a
    /// second leaves the connection idle in between and must not be dropped. That is the one call
    /// site that passes `None`.
    ///
    /// Read twice, with a silence before each, because the gap that matters is the one *between
    /// whole frames* — a desktop with no dialog open sits there for minutes. A deadline that
    /// leaked onto the header would end the connection during either gap.
    ///
    /// Asserted without leaking a blocked thread: each silence is 200ms, four times the body
    /// timeout, and then the reader speaks.
    #[test]
    fn a_request_loops_wait_for_the_next_frame_is_unbounded() {
        let mut two = framed("hello");
        two.extend(framed("again"));
        let mut idle_then_busy = QuietThenSpeaks { quiet_reads_left: 10, given: Cursor::new(two) };

        let got: String =
            read_frame(&mut idle_then_busy, DEFAULT_MAX_FRAME, None, Duration::from_millis(50))
                .expect("a request loop must not drop an idle client before a frame begins");
        assert_eq!(got, "hello");

        idle_then_busy.quiet_reads_left = 10;
        let got: String =
            read_frame(&mut idle_then_busy, DEFAULT_MAX_FRAME, None, Duration::from_millis(50))
                .expect("nor between one whole frame and the next");
        assert_eq!(got, "again");
    }

    /// The defect this change fixes: an authenticated client sends one byte of a length prefix and
    /// stops. The wait for a frame to *begin* is still unbounded — `None` below — but those other
    /// three bytes are owed, so the read gives up instead of holding the worker, and the one
    /// client slot in eight underneath it, until the daemon is restarted.
    #[test]
    fn a_header_that_begins_and_stops_times_out_even_where_the_wait_to_begin_did_not() {
        let started = std::time::Instant::now();
        let result = read_frame::<_, String>(
            &mut StallsAfter::new(vec![0u8]),
            DEFAULT_MAX_FRAME,
            None,
            Duration::from_millis(200),
        );
        assert!(matches!(result, Err(FrameError::Timeout)), "got {result:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "it waited far past its deadline");
    }

    /// A peer that closes after part of a header left mid-frame, so this is `Io` and not `Eof` —
    /// the request loop treats `Eof` as "the client went away between requests" and returns
    /// cleanly, which would log a truncated frame as an orderly goodbye.
    #[test]
    fn a_peer_that_closes_mid_header_is_a_fault_not_an_eof() {
        let result = read_frame::<_, String>(
            &mut Cursor::new(vec![0u8]),
            DEFAULT_MAX_FRAME,
            None,
            Duration::from_secs(5),
        );
        assert!(matches!(result, Err(FrameError::Io(_))), "got {result:?}");
    }

    /// A header wait that *is* owed — a token, a greeting, a reply — must time out rather than
    /// hold the reader forever. This is the client waiting on a Pi that went silent the instant
    /// it accepted the request: never a byte back, not even `WouldBlock` yet to retry against.
    #[test]
    fn an_owed_header_that_never_arrives_times_out() {
        struct NeverSpeaks;
        impl Read for NeverSpeaks {
            fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
                std::thread::sleep(Duration::from_millis(20));
                Err(io::Error::new(io::ErrorKind::WouldBlock, "no data yet"))
            }
        }

        let started = std::time::Instant::now();
        let result = read_frame::<_, String>(
            &mut NeverSpeaks,
            DEFAULT_MAX_FRAME,
            Some(Duration::from_millis(200)),
            DEFAULT_BODY_TIMEOUT,
        );
        assert!(matches!(result, Err(FrameError::Timeout)), "got {result:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "it waited far past its deadline");
    }

    /// A frame that arrives in pieces, with stalls between them, must still be read — the
    /// deadline bounds the whole body, not each read.
    #[test]
    fn a_body_arriving_in_pieces_is_reassembled() {
        let bytes = framed("hello");
        let mut piecewise = StallsAfter::new(bytes);
        let got: String = read_frame(
            &mut piecewise,
            DEFAULT_MAX_FRAME,
            Some(DEFAULT_BODY_TIMEOUT),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(got, "hello");
    }

    /// A peer that trickles — always producing a byte before the socket would ever report
    /// `WouldBlock` — must still be bound by the deadline. Neither `StallsAfter` nor
    /// `QuietThenSpeaks` trickles; both alternate silence with delivering everything at once,
    /// which is exactly why a deadline check that lived only in the `WouldBlock` arm slipped past
    /// them: a peer that never goes quiet never reaches that arm at all.
    #[test]
    fn a_trickling_body_still_times_out() {
        struct Trickles {
            given: Cursor<Vec<u8>>,
        }
        impl Read for Trickles {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                std::thread::sleep(Duration::from_millis(20));
                let n = 1.min(buf.len());
                self.given.read(&mut buf[..n])
            }
        }

        let bytes = framed("hello");
        let mut trickling = Trickles { given: Cursor::new(bytes) };
        let result = read_frame::<_, String>(
            &mut trickling,
            DEFAULT_MAX_FRAME,
            Some(DEFAULT_BODY_TIMEOUT),
            Duration::from_millis(50),
        );
        assert!(matches!(result, Err(FrameError::Timeout)), "got {result:?}");
    }

    /// A peer that closes mid-body is a fault, not a clean end — `Eof` means "closed between
    /// frames" and a caller loops on it.
    #[test]
    fn a_peer_that_closes_mid_body_is_a_fault_not_an_eof() {
        let mut truncated = Vec::new();
        truncated.extend_from_slice(&(64u32).to_be_bytes());
        truncated.extend_from_slice(b"{\"partial\":");

        let result = read_frame::<_, String>(
            &mut Cursor::new(truncated),
            DEFAULT_MAX_FRAME,
            Some(DEFAULT_BODY_TIMEOUT),
            Duration::from_secs(5),
        );
        assert!(matches!(result, Err(FrameError::Io(_))), "got {result:?}");
    }
}
