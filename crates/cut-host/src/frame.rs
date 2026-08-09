// SPDX-License-Identifier: GPL-3.0-or-later

//! Length-prefixed JSON frames.
//!
//! The length is a big-endian `u32` and it is checked against the cap *before* the
//! body is read. A Cut Host runs on a Pi with a gigabyte of RAM, so a header
//! claiming more than the cap must cost nothing to refuse.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{self, Read, Write};

/// 32 MiB. A large cut is megabytes of polylines as JSON text; this leaves room
/// for that and refuses anything that could only be an attack or a bug.
pub const DEFAULT_MAX_FRAME: usize = 32 * 1024 * 1024;

#[derive(Debug)]
pub enum FrameError {
    /// The peer closed cleanly between frames. Not a fault.
    Eof,
    TooLarge { len: usize, max: usize },
    Io(String),
    Malformed(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Eof => write!(f, "the peer closed the connection"),
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

pub fn read_frame<R: Read, T: DeserializeOwned>(r: &mut R, max: usize) -> Result<T, FrameError> {
    let mut header = [0u8; 4];
    match r.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Err(FrameError::Eof),
        Err(e) => return Err(FrameError::Io(e.to_string())),
    }
    let len = u32::from_be_bytes(header) as usize;
    // Before the allocation, not after: the whole point of the cap.
    if len > max {
        return Err(FrameError::TooLarge { len, max });
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(|e| FrameError::Io(e.to_string()))?;
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
        let first: String = read_frame(&mut cursor, DEFAULT_MAX_FRAME).unwrap();
        let second: u32 = read_frame(&mut cursor, DEFAULT_MAX_FRAME).unwrap();
        assert_eq!(first, "hello");
        assert_eq!(second, 42);
    }

    #[test]
    fn a_clean_end_of_stream_is_eof_not_an_error() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        assert!(matches!(read_frame::<_, String>(&mut cursor, DEFAULT_MAX_FRAME), Err(FrameError::Eof)));
    }

    /// The rule this task exists for: the length is refused *before* the body is
    /// read, so a hostile header cannot make the host allocate what it claims.
    /// Asserted by giving a huge length and no body at all — a reader that
    /// allocated first would block or die instead of returning `TooLarge`.
    #[test]
    fn an_oversized_length_is_refused_without_reading_a_body() {
        let mut framed: Vec<u8> = Vec::new();
        framed.extend_from_slice(&(9_000_000u32).to_be_bytes());
        // deliberately no body

        let mut cursor = std::io::Cursor::new(framed);
        match read_frame::<_, String>(&mut cursor, 1024) {
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
            read_frame::<_, String>(&mut cursor, DEFAULT_MAX_FRAME),
            Err(FrameError::Io(_)) | Err(FrameError::Malformed(_))
        ));
    }

    #[test]
    fn a_body_that_is_not_the_expected_shape_is_malformed() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &"a string".to_string()).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        assert!(matches!(read_frame::<_, u32>(&mut cursor, DEFAULT_MAX_FRAME), Err(FrameError::Malformed(_))));
    }
}
