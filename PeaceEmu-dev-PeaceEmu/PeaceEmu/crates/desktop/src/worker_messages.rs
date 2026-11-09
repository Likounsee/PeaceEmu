#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Strict payload schemas layered on top of the worker framing protocol.
//!
//! JSON control messages reject unknown fields and are bounded before parsing.
//! Frame output uses a compact binary RGBA8 envelope. This validates shape and
//! size only; it does not make an untrusted worker safe to execute.

use emulation_core::module_manifest::ReleaseVersion;
use emulation_core::module_worker_protocol::{
    WorkerMessage, WorkerMessageKind, MAX_WORKER_MESSAGE_BYTES,
};
use serde::Deserialize;
use std::fmt;

/// Maximum UTF-8 bytes accepted for a JSON control payload.
pub const MAX_CONTROL_PAYLOAD_BYTES: usize = 64 * 1024;
/// Maximum dimensions accepted for one worker video frame.
pub const MAX_FRAME_DIMENSION: u32 = 4096;
const FRAME_HEADER_BYTES: usize = 9;

/// Schema or payload validation failure for a worker message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerPayloadError {
    /// JSON payload is larger than the control-message limit.
    ControlPayloadTooLarge,
    /// Payload is not valid JSON for the selected message kind.
    InvalidJson,
    /// Payload violates the selected message kind's field or value constraints.
    InvalidFields,
    /// Message kind requires an empty payload.
    ExpectedEmptyPayload,
    /// Frame payload has an unsupported pixel format, dimensions, or byte count.
    InvalidFrameOutput,
}

impl fmt::Display for WorkerPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ControlPayloadTooLarge => f.write_str("worker control payload exceeds its limit"),
            Self::InvalidJson => f.write_str("worker control payload is not valid schema JSON"),
            Self::InvalidFields => f.write_str("worker control payload contains invalid fields"),
            Self::ExpectedEmptyPayload => f.write_str("worker message requires an empty payload"),
            Self::InvalidFrameOutput => {
                f.write_str("worker frame output has an invalid binary envelope")
            }
        }
    }
}
impl std::error::Error for WorkerPayloadError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InitializePayload {
    module_id: String,
    app_version: String,
    content_size_bytes: u64,
    content_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputPayload {
    events: Vec<InputEventPayload>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputEventPayload {
    control: String,
    pressed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunFramePayload {
    frame_number: u64,
    instruction_budget: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorPayload {
    code: String,
    message: String,
}

/// Validates a message payload against the schema assigned to its operation.
///
/// The protocol framing layer must run first. Call this function on both messages
/// received from a worker and messages sent to one; never trust the peer to enforce
/// its own schemas.
pub fn validate_worker_message(message: &WorkerMessage) -> Result<(), WorkerPayloadError> {
    if message.payload.len() > MAX_WORKER_MESSAGE_BYTES {
        return Err(WorkerPayloadError::InvalidFields);
    }
    match message.kind {
        WorkerMessageKind::Initialize => {
            let payload: InitializePayload = parse_control(&message.payload)?;
            let valid_id = !payload.module_id.is_empty()
                && payload.module_id.len() <= 128
                && payload
                    .module_id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !payload.module_id.starts_with('-')
                && !payload.module_id.ends_with('-')
                && !payload.module_id.contains("--");
            let valid_digest = payload.content_sha256.len() == 64
                && payload
                    .content_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            let valid_version = ReleaseVersion::parse(&payload.app_version).is_ok();
            if !valid_id
                || !valid_digest
                || !valid_version
                || payload.content_size_bytes == 0
                || payload.content_size_bytes > 32 * 1024 * 1024
            {
                return Err(WorkerPayloadError::InvalidFields);
            }
        }
        WorkerMessageKind::Input => {
            let payload: InputPayload = parse_control(&message.payload)?;
            if payload.events.len() > 256
                || payload.events.iter().any(|event| {
                    let _pressed = event.pressed;
                    event.control.is_empty()
                        || event.control.len() > 64
                        || !event
                            .control
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                })
            {
                return Err(WorkerPayloadError::InvalidFields);
            }
        }
        WorkerMessageKind::RunFrame => {
            let payload: RunFramePayload = parse_control(&message.payload)?;
            if payload.instruction_budget == 0 || payload.instruction_budget > 1_000_000_000 {
                return Err(WorkerPayloadError::InvalidFields);
            }
            let _frame_number = payload.frame_number;
        }
        WorkerMessageKind::Shutdown => {
            if !message.payload.is_empty() {
                return Err(WorkerPayloadError::ExpectedEmptyPayload);
            }
        }
        WorkerMessageKind::Error => {
            let payload: ErrorPayload = parse_control(&message.payload)?;
            if payload.code.is_empty()
                || payload.code.len() > 64
                || !payload
                    .code
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                || payload.message.len() > 2048
                || payload.message.chars().any(char::is_control)
            {
                return Err(WorkerPayloadError::InvalidFields);
            }
        }
        WorkerMessageKind::FrameOutput => validate_frame_output(&message.payload)?,
    }
    Ok(())
}

fn parse_control<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, WorkerPayloadError> {
    if bytes.len() > MAX_CONTROL_PAYLOAD_BYTES {
        return Err(WorkerPayloadError::ControlPayloadTooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| WorkerPayloadError::InvalidJson)
}

/// Frame output format: pixel-format byte (1 = RGBA8), width u32 BE,
/// height u32 BE, then exactly width * height * 4 bytes of pixel data.
fn validate_frame_output(bytes: &[u8]) -> Result<(), WorkerPayloadError> {
    if bytes.len() < FRAME_HEADER_BYTES || bytes[0] != 1 {
        return Err(WorkerPayloadError::InvalidFrameOutput);
    }
    let width = u32::from_be_bytes(bytes[1..5].try_into().expect("fixed header range"));
    let height = u32::from_be_bytes(bytes[5..9].try_into().expect("fixed header range"));
    if width == 0 || height == 0 || width > MAX_FRAME_DIMENSION || height > MAX_FRAME_DIMENSION {
        return Err(WorkerPayloadError::InvalidFrameOutput);
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| bytes.checked_add(FRAME_HEADER_BYTES))
        .ok_or(WorkerPayloadError::InvalidFrameOutput)?;
    if expected != bytes.len() || expected > MAX_WORKER_MESSAGE_BYTES {
        return Err(WorkerPayloadError::InvalidFrameOutput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(kind: WorkerMessageKind, payload: &[u8]) -> WorkerMessage {
        WorkerMessage::new(kind, payload.to_vec()).unwrap()
    }

    #[test]
    fn accepts_valid_initialize_and_run_frame_messages() {
        let initialize = message(
            WorkerMessageKind::Initialize,
            br#"{"module_id":"game-boy","app_version":"1.2.3","content_size_bytes":4096,"content_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
        );
        assert!(validate_worker_message(&initialize).is_ok());
        let run = message(
            WorkerMessageKind::RunFrame,
            br#"{"frame_number":42,"instruction_budget":100000}"#,
        );
        assert!(validate_worker_message(&run).is_ok());
    }

    #[test]
    fn rejects_unknown_fields_invalid_values_and_oversized_control_messages() {
        let unknown = message(
            WorkerMessageKind::RunFrame,
            br#"{"frame_number":42,"instruction_budget":1000,"execute_shell":true}"#,
        );
        assert_eq!(
            validate_worker_message(&unknown),
            Err(WorkerPayloadError::InvalidJson)
        );
        let invalid = message(
            WorkerMessageKind::RunFrame,
            br#"{"frame_number":42,"instruction_budget":0}"#,
        );
        assert_eq!(
            validate_worker_message(&invalid),
            Err(WorkerPayloadError::InvalidFields)
        );
        let large = WorkerMessage::new(
            WorkerMessageKind::Input,
            vec![b' '; MAX_CONTROL_PAYLOAD_BYTES + 1],
        )
        .unwrap();
        assert_eq!(
            validate_worker_message(&large),
            Err(WorkerPayloadError::ControlPayloadTooLarge)
        );
    }

    #[test]
    fn rejects_nonempty_shutdown_and_malformed_error_messages() {
        assert_eq!(
            validate_worker_message(&message(WorkerMessageKind::Shutdown, b"{}")),
            Err(WorkerPayloadError::ExpectedEmptyPayload)
        );
        let bad_error = message(
            WorkerMessageKind::Error,
            br#"{"code":"bad-code","message":"failed"}"#,
        );
        assert_eq!(
            validate_worker_message(&bad_error),
            Err(WorkerPayloadError::InvalidFields)
        );
    }

    #[test]
    fn frame_output_checks_dimensions_and_exact_rgba_byte_count() {
        let mut valid = vec![1, 0, 0, 0, 1, 0, 0, 0, 1];
        valid.extend_from_slice(&[0, 0, 0, 255]);
        assert!(validate_worker_message(&message(WorkerMessageKind::FrameOutput, &valid)).is_ok());
        valid.pop();
        assert_eq!(
            validate_worker_message(&message(WorkerMessageKind::FrameOutput, &valid)),
            Err(WorkerPayloadError::InvalidFrameOutput)
        );
        assert_eq!(
            validate_worker_message(&message(
                WorkerMessageKind::FrameOutput,
                &[1, 0, 0, 0, 0, 0, 0, 0, 1]
            )),
            Err(WorkerPayloadError::InvalidFrameOutput)
        );
    }
}
