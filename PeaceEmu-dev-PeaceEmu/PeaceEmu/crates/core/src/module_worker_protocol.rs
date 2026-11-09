#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Bounded, versioned framing for an out-of-process emulation-core worker.
//!
//! This module defines wire framing only; it does not spawn processes, authenticate
//! worker executables, or provide an operating-system sandbox. Callers must still
//! enforce process lifetime, permissions, timeouts, and executable provenance.

use core::fmt;
use std::io::{self, Read, Write};

/// Fixed wire protocol version supported by this host build.
pub const WORKER_PROTOCOL_VERSION: u16 = 1;
/// Maximum payload accepted from or sent to a worker (16 MiB).
pub const MAX_WORKER_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAGIC: [u8; 4] = *b"AEWP";
const HEADER_BYTES: usize = 11;

/// A supported worker message kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum WorkerMessageKind {
    /// Host initializes a worker with validated system and content metadata.
    Initialize = 1,
    /// Host sends an input frame.
    Input = 2,
    /// Host requests one bounded emulation step.
    RunFrame = 3,
    /// Worker returns video/audio/output data for the latest frame.
    FrameOutput = 4,
    /// Host requests an orderly worker shutdown.
    Shutdown = 5,
    /// Worker reports a structured runtime error.
    Error = 6,
}

impl TryFrom<u8> for WorkerMessageKind {
    type Error = WorkerProtocolError;

    fn try_from(value: u8) -> Result<Self, WorkerProtocolError> {
        match value {
            1 => Ok(Self::Initialize),
            2 => Ok(Self::Input),
            3 => Ok(Self::RunFrame),
            4 => Ok(Self::FrameOutput),
            5 => Ok(Self::Shutdown),
            6 => Ok(Self::Error),
            other => Err(WorkerProtocolError::UnknownMessageKind(other)),
        }
    }
}

/// One fully framed worker protocol message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerMessage {
    /// Wire protocol version.
    pub version: u16,
    /// Message operation.
    pub kind: WorkerMessageKind,
    /// Opaque payload; its schema must be validated by the message handler.
    pub payload: Vec<u8>,
}

impl WorkerMessage {
    /// Creates a message using the current protocol version and checks payload size.
    pub fn new(kind: WorkerMessageKind, payload: Vec<u8>) -> Result<Self, WorkerProtocolError> {
        if payload.len() > MAX_WORKER_MESSAGE_BYTES {
            return Err(WorkerProtocolError::PayloadTooLarge(payload.len() as u64));
        }
        Ok(Self {
            version: WORKER_PROTOCOL_VERSION,
            kind,
            payload,
        })
    }
}

/// A framing, compatibility, or transport failure.
#[derive(Debug)]
pub enum WorkerProtocolError {
    /// Underlying pipe or stream operation failed.
    Io(io::Error),
    /// The stream does not begin with the worker protocol magic bytes.
    InvalidMagic,
    /// The message uses an unsupported wire version.
    UnsupportedVersion(u16),
    /// The message kind is unknown to this host.
    UnknownMessageKind(u8),
    /// The announced payload exceeds the configured bound.
    PayloadTooLarge(u64),
}

impl fmt::Display for WorkerProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "worker protocol I/O error: {error}"),
            Self::InvalidMagic => f.write_str("invalid worker protocol magic"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported worker protocol version {version}")
            }
            Self::UnknownMessageKind(kind) => write!(f, "unknown worker message kind {kind}"),
            Self::PayloadTooLarge(size) => write!(
                f,
                "worker message payload size {size} exceeds the {}-byte limit",
                MAX_WORKER_MESSAGE_BYTES
            ),
        }
    }
}

impl std::error::Error for WorkerProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for WorkerProtocolError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Writes one complete message to a worker pipe using a fixed-size header.
///
/// Wire layout: magic (4 bytes), version (2-byte big-endian), kind (1 byte),
/// payload length (4-byte big-endian), then exactly that many payload bytes.
pub fn write_message<W: Write>(
    writer: &mut W,
    message: &WorkerMessage,
) -> Result<(), WorkerProtocolError> {
    if message.version != WORKER_PROTOCOL_VERSION {
        return Err(WorkerProtocolError::UnsupportedVersion(message.version));
    }
    if message.payload.len() > MAX_WORKER_MESSAGE_BYTES {
        return Err(WorkerProtocolError::PayloadTooLarge(
            message.payload.len() as u64
        ));
    }

    let mut header = [0_u8; HEADER_BYTES];
    header[..4].copy_from_slice(&MAGIC);
    header[4..6].copy_from_slice(&message.version.to_be_bytes());
    header[6] = message.kind as u8;
    header[7..11].copy_from_slice(&(message.payload.len() as u32).to_be_bytes());
    writer.write_all(&header)?;
    writer.write_all(&message.payload)?;
    writer.flush()?;
    Ok(())
}

/// Reads one complete message, rejecting invalid headers and oversized lengths
/// before allocating memory for the untrusted payload.
pub fn read_message<R: Read>(reader: &mut R) -> Result<WorkerMessage, WorkerProtocolError> {
    let mut header = [0_u8; HEADER_BYTES];
    reader.read_exact(&mut header)?;
    if header[..4] != MAGIC {
        return Err(WorkerProtocolError::InvalidMagic);
    }

    let version = u16::from_be_bytes([header[4], header[5]]);
    if version != WORKER_PROTOCOL_VERSION {
        return Err(WorkerProtocolError::UnsupportedVersion(version));
    }
    let kind = WorkerMessageKind::try_from(header[6])?;
    let payload_len = u32::from_be_bytes([header[7], header[8], header[9], header[10]]) as u64;
    if payload_len > MAX_WORKER_MESSAGE_BYTES as u64 {
        return Err(WorkerProtocolError::PayloadTooLarge(payload_len));
    }

    let mut payload = vec![0_u8; payload_len as usize];
    reader.read_exact(&mut payload)?;
    Ok(WorkerMessage {
        version,
        kind,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_message() {
        let expected =
            WorkerMessage::new(WorkerMessageKind::FrameOutput, vec![0, 1, 2, 127, 128, 255])
                .unwrap();
        let mut bytes = Vec::new();
        write_message(&mut bytes, &expected).unwrap();
        assert_eq!(read_message(&mut bytes.as_slice()).unwrap(), expected);
    }

    #[test]
    fn rejects_bad_magic() {
        let bytes = [0_u8; HEADER_BYTES];
        assert!(matches!(
            read_message(&mut bytes.as_slice()),
            Err(WorkerProtocolError::InvalidMagic)
        ));
    }

    #[test]
    fn rejects_oversized_length_before_reading_payload() {
        let mut bytes = Vec::from(MAGIC);
        bytes.extend_from_slice(&WORKER_PROTOCOL_VERSION.to_be_bytes());
        bytes.push(WorkerMessageKind::Initialize as u8);
        bytes.extend_from_slice(&((MAX_WORKER_MESSAGE_BYTES as u32) + 1).to_be_bytes());
        assert!(matches!(
            read_message(&mut bytes.as_slice()),
            Err(WorkerProtocolError::PayloadTooLarge(_))
        ));
    }

    #[test]
    fn rejects_unknown_message_kind() {
        let mut bytes = Vec::from(MAGIC);
        bytes.extend_from_slice(&WORKER_PROTOCOL_VERSION.to_be_bytes());
        bytes.push(255);
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        assert!(matches!(
            read_message(&mut bytes.as_slice()),
            Err(WorkerProtocolError::UnknownMessageKind(255))
        ));
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut bytes = Vec::from(MAGIC);
        bytes.extend_from_slice(&2_u16.to_be_bytes());
        bytes.push(WorkerMessageKind::Initialize as u8);
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        assert!(matches!(
            read_message(&mut bytes.as_slice()),
            Err(WorkerProtocolError::UnsupportedVersion(2))
        ));
    }
}
