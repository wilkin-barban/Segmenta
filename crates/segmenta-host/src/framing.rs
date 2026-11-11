//! Native Messaging framing: a 4-byte little-endian length prefix followed by
//! exactly that many bytes of UTF-8 JSON (OpenSpec 03 §4, ODD S1).
//!
//! The cap is enforced **before** the payload is buffered: a forged length over
//! [`MAX_MESSAGE_BYTES`] never causes an allocation proportional to that length.
//! Leftover bytes stay in the reader, so a pipelined second message is read as the
//! next message instead of being dropped.

use std::fmt;
use std::io::{self, Read, Write};

use serde_json::Value;

/// Largest message the host accepts or emits, matching OpenSpec 03 §4.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Stable identifier of [`FrameError::Empty`].
pub const ERR_FRAME_EMPTY: &str = "ERR_FRAME_EMPTY";
/// Stable identifier of [`FrameError::TooLarge`].
pub const ERR_FRAME_TOO_LARGE: &str = "ERR_FRAME_TOO_LARGE";
/// Stable identifier of [`FrameError::Truncated`].
pub const ERR_FRAME_TRUNCATED: &str = "ERR_FRAME_TRUNCATED";
/// Stable identifier of [`FrameError::InvalidJson`].
pub const ERR_FRAME_INVALID_JSON: &str = "ERR_FRAME_INVALID_JSON";
/// Stable identifier of [`FrameError::Io`].
pub const ERR_FRAME_IO: &str = "ERR_FRAME_IO";

/// Framing failure of one Native Messaging message.
#[derive(Debug)]
pub enum FrameError {
    /// A zero length prefix carries no message.
    Empty,
    /// The declared length is over [`MAX_MESSAGE_BYTES`]; the body was not read.
    TooLarge {
        /// The length declared by the header.
        length: usize,
    },
    /// The stream ended in the middle of a header or a body.
    Truncated,
    /// The body was not UTF-8 JSON.
    InvalidJson {
        /// Parser detail; never contains the payload.
        detail: String,
    },
    /// The underlying reader failed.
    Io(io::Error),
}

impl FrameError {
    /// Stable `ERR_*` identifier of this failure.
    pub fn identifier(&self) -> &'static str {
        match self {
            FrameError::Empty => ERR_FRAME_EMPTY,
            FrameError::TooLarge { .. } => ERR_FRAME_TOO_LARGE,
            FrameError::Truncated => ERR_FRAME_TRUNCATED,
            FrameError::InvalidJson { .. } => ERR_FRAME_INVALID_JSON,
            FrameError::Io(_) => ERR_FRAME_IO,
        }
    }
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let identifier = self.identifier();
        match self {
            FrameError::Empty => write!(f, "{identifier}: message length is zero"),
            FrameError::TooLarge { length } => write!(
                f,
                "{identifier}: {length} bytes exceeds the {MAX_MESSAGE_BYTES} byte limit"
            ),
            FrameError::Truncated => {
                write!(
                    f,
                    "{identifier}: the stream ended before the message was complete"
                )
            }
            FrameError::InvalidJson { detail } => {
                write!(f, "{identifier}: the body is not UTF-8 JSON: {detail}")
            }
            FrameError::Io(err) => write!(f, "{identifier}: {err}"),
        }
    }
}

impl std::error::Error for FrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FrameError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for FrameError {
    fn from(err: io::Error) -> FrameError {
        FrameError::Io(err)
    }
}

/// Read one message: `Ok(None)` on a clean EOF, one [`Value`] per complete frame.
///
/// Returns [`FrameError::Empty`] for a zero length, [`FrameError::TooLarge`] when the
/// declared length is over the cap (without reading the body), [`FrameError::Truncated`]
/// when the stream ends mid-header or mid-body, and [`FrameError::InvalidJson`] when the
/// body is not UTF-8 JSON. Bytes after a complete message are left in the reader.
pub fn read_message<R: Read + ?Sized>(reader: &mut R) -> Result<Option<Value>, FrameError> {
    let mut header = [0u8; 4];
    if !read_header(reader, &mut header)? {
        return Ok(None);
    }
    let length = u32::from_le_bytes(header) as usize;
    if length == 0 {
        return Err(FrameError::Empty);
    }
    if length > MAX_MESSAGE_BYTES {
        // The cap is checked before any allocation or read of the body.
        return Err(FrameError::TooLarge { length });
    }
    let mut payload = vec![0u8; length];
    read_body(reader, &mut payload)?;
    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(|err| FrameError::InvalidJson {
            detail: err.to_string(),
        })
}

/// Write one message. The cap is enforced before a single byte reaches the browser.
pub fn write_message<W: Write + ?Sized>(writer: &mut W, value: &Value) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{ERR_FRAME_INVALID_JSON}: {err}"),
        )
    })?;
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{ERR_FRAME_TOO_LARGE}: {} bytes exceeds the {MAX_MESSAGE_BYTES} byte limit",
                payload.len()
            ),
        ));
    }
    let length = u32::try_from(payload.len()).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{ERR_FRAME_TOO_LARGE}: {err}"),
        )
    })?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()
}

/// Fill the 4 header bytes. `Ok(false)` means a clean EOF with nothing read.
fn read_header<R: Read + ?Sized>(reader: &mut R, header: &mut [u8; 4]) -> Result<bool, FrameError> {
    let mut filled = 0;
    while filled < header.len() {
        match reader.read(&mut header[filled..]) {
            Ok(0) => {
                return if filled == 0 {
                    Ok(false)
                } else {
                    Err(FrameError::Truncated)
                };
            }
            Ok(read) => filled += read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(FrameError::Io(err)),
        }
    }
    Ok(true)
}

/// Fill the body, mapping an early EOF to [`FrameError::Truncated`].
fn read_body<R: Read + ?Sized>(reader: &mut R, payload: &mut [u8]) -> Result<(), FrameError> {
    let mut filled = 0;
    while filled < payload.len() {
        match reader.read(&mut payload[filled..]) {
            Ok(0) => return Err(FrameError::Truncated),
            Ok(read) => filled += read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(FrameError::Io(err)),
        }
    }
    Ok(())
}
