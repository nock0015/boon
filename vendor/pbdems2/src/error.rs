//! Error and result types shared by every fallible operation in the crate.

use std::fmt;

/// Errors that can occur while parsing a demo file.
///
/// Malformed input produces an error rather than a panic, so a corrupt demo
/// can be reported and skipped instead of taking the process down.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// The underlying reader or file failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// The file does not begin with the `PBDEMS2\0` magic prefix.
    #[error("invalid demo file: magic bytes mismatch (expected PBDEMS2\\0, got {got:?})")]
    InvalidMagic {
        /// The eight bytes actually found at the start of the file.
        got: [u8; 8],
    },
    /// A read ran past the end of the available data.
    #[error("unexpected end of data: needed {needed} bits, have {available}")]
    Overflow {
        /// Bits the read required.
        needed: usize,
        /// Bits still available in the stream.
        available: usize,
    },
    /// Snappy decompression of a command body or string-table payload failed.
    #[error("decompression error: {0}")]
    Decompress(String),
    /// A declared size exceeded the corresponding [`crate::DecodeLimits`] cap.
    ///
    /// Raised before allocating, so a hostile length field cannot turn into an
    /// unbounded reservation.
    #[error("{resource} exceeds decode limit: {actual} > {limit}")]
    LimitExceeded {
        /// Name of the limit that tripped, as used in [`crate::DecodeLimits`].
        resource: &'static str,
        /// Configured ceiling for the resource.
        limit: usize,
        /// Size the demo asked for.
        actual: usize,
    },
    /// A within-limits allocation was still refused by the allocator.
    #[error("could not reserve {requested} bytes for {resource}")]
    Allocation {
        /// Name of the buffer being reserved.
        resource: &'static str,
        /// Byte count that could not be reserved.
        requested: usize,
    },
    /// Wraps a failure with the position of the command that produced it.
    #[error("command at byte {offset} (command {command:?}, tick {tick:?}): {source}")]
    Command {
        /// Absolute byte offset of the command in the file.
        offset: usize,
        /// Command identifier, when the header was read far enough to know it.
        command: Option<i32>,
        /// Demo tick, when the header was read far enough to know it.
        tick: Option<i32>,
        /// The underlying failure.
        #[source]
        source: Box<Error>,
    },
    /// Wraps a failure with the position and type of an inner packet message.
    #[error("packet message at bit {bit_offset} (type {message_type:?}): {source}")]
    PacketMessage {
        /// Bit offset of the message-type header within its packet payload.
        bit_offset: usize,
        /// Message identifier, when framing was read far enough to know it.
        message_type: Option<u32>,
        /// The underlying framing, limit, allocation, or payload-copy failure.
        #[source]
        source: Box<Error>,
    },
    /// The bitstream was structurally valid but semantically inconsistent,
    /// which normally means an earlier decode desynced.
    #[error("parse error: {context}")]
    Parse {
        /// Human-readable description of what did not line up.
        context: String,
    },
}

// Manual impl because `snap::Error` doesn't implement `std::error::Error`,
// so thiserror's `#[from]` derive can't be used.
impl From<snap::Error> for Error {
    fn from(e: snap::Error) -> Self {
        Error::Decompress(e.to_string())
    }
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Lightweight error type for field value conversions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldValueConversionError;

impl fmt::Display for FieldValueConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "incompatible types or out of range integer conversion attempted"
        )
    }
}

impl std::error::Error for FieldValueConversionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_display() {
        let err = Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
        let msg = format!("{err}");
        assert!(msg.contains("IO error"));
    }

    #[test]
    fn invalid_magic_display() {
        let err = Error::InvalidMagic { got: [0; 8] };
        let msg = format!("{err}");
        assert!(msg.contains("magic bytes"));
    }

    #[test]
    fn overflow_display() {
        let err = Error::Overflow {
            needed: 64,
            available: 8,
        };
        let msg = format!("{err}");
        assert!(msg.contains("64"));
        assert!(msg.contains("8"));
    }

    #[test]
    fn parse_error_display() {
        let err = Error::Parse {
            context: "bad data".to_string(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("bad data"));
    }

    #[test]
    fn field_value_conversion_error() {
        let err = FieldValueConversionError;
        let msg = format!("{err}");
        assert!(msg.contains("incompatible"));
        // Verify it implements std::error::Error
        let _: &dyn std::error::Error = &err;
    }
}
