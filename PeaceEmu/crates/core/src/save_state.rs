#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Bounded, versioned save-state envelopes independent of any host UI or disk layout.
//!
//! The checksum detects accidental corruption; it is not a cryptographic
//! authenticity mechanism. Each core remains responsible for validating its
//! internal payload before mutating machine state.

use core::fmt;

/// Current on-disk/in-memory envelope format version.
pub const SAVE_STATE_FORMAT_VERSION: u16 = 1;

/// Maximum accepted save-state payload size (64 MiB).
pub const MAX_SAVE_STATE_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

const MAGIC: &[u8; 8] = b"ARCHSAVE";
const FIXED_HEADER_BYTES: usize = 8 + 2 + 2 + 2 + 8 + 4;
const MAX_ID_BYTES: usize = 64;
const MAX_CORE_VERSION_BYTES: usize = 128;

/// Metadata required to associate a state with the core that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveStateMetadata {
    /// Stable core/system identifier.
    pub system_id: String,
    /// Core implementation version, not the host API version.
    pub core_version: String,
}

/// A validated, owned save-state payload and its compatibility metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveStateEnvelope {
    metadata: SaveStateMetadata,
    payload: Vec<u8>,
}

impl SaveStateEnvelope {
    /// Creates an envelope after validating metadata and payload bounds.
    pub fn new(
        system_id: impl Into<String>,
        core_version: impl Into<String>,
        payload: Vec<u8>,
    ) -> Result<Self, SaveStateError> {
        let metadata = SaveStateMetadata {
            system_id: system_id.into(),
            core_version: core_version.into(),
        };
        validate_metadata(&metadata)?;
        if payload.is_empty() {
            return Err(SaveStateError::EmptyPayload);
        }
        if payload.len() > MAX_SAVE_STATE_PAYLOAD_BYTES {
            return Err(SaveStateError::PayloadTooLarge {
                actual: payload.len() as u64,
                maximum: MAX_SAVE_STATE_PAYLOAD_BYTES as u64,
            });
        }
        Ok(Self { metadata, payload })
    }

    /// Returns compatibility metadata without exposing mutable internals.
    pub fn metadata(&self) -> &SaveStateMetadata {
        &self.metadata
    }

    /// Returns the opaque state bytes owned by this envelope.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Rejects states produced by another system or core implementation version.
    pub fn validate_for(&self, system_id: &str, core_version: &str) -> Result<(), SaveStateError> {
        if self.metadata.system_id != system_id {
            return Err(SaveStateError::IncompatibleSystem);
        }
        if self.metadata.core_version != core_version {
            return Err(SaveStateError::IncompatibleCoreVersion);
        }
        Ok(())
    }

    /// Encodes a deterministic binary envelope with a CRC-32 corruption check.
    pub fn encode(&self) -> Vec<u8> {
        let system = self.metadata.system_id.as_bytes();
        let version = self.metadata.core_version.as_bytes();
        let mut output = Vec::with_capacity(
            FIXED_HEADER_BYTES + system.len() + version.len() + self.payload.len(),
        );
        output.extend_from_slice(MAGIC);
        output.extend_from_slice(&SAVE_STATE_FORMAT_VERSION.to_le_bytes());
        output.extend_from_slice(&(system.len() as u16).to_le_bytes());
        output.extend_from_slice(&(version.len() as u16).to_le_bytes());
        output.extend_from_slice(&(self.payload.len() as u64).to_le_bytes());
        output.extend_from_slice(&crc32(&self.payload).to_le_bytes());
        output.extend_from_slice(system);
        output.extend_from_slice(version);
        output.extend_from_slice(&self.payload);
        output
    }

    /// Decodes an envelope, checking its format, exact length, metadata and checksum.
    pub fn decode(bytes: &[u8]) -> Result<Self, SaveStateError> {
        if bytes.len() < FIXED_HEADER_BYTES {
            return Err(SaveStateError::Truncated);
        }
        if &bytes[..MAGIC.len()] != MAGIC {
            return Err(SaveStateError::InvalidMagic);
        }
        let format = u16::from_le_bytes([bytes[8], bytes[9]]);
        if format != SAVE_STATE_FORMAT_VERSION {
            return Err(SaveStateError::UnsupportedFormat(format));
        }
        let system_len = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
        let version_len = u16::from_le_bytes([bytes[12], bytes[13]]) as usize;
        let payload_len = u64::from_le_bytes(bytes[14..22].try_into().expect("fixed header"));
        let expected_crc = u32::from_le_bytes(bytes[22..26].try_into().expect("fixed header"));

        if system_len == 0
            || system_len > MAX_ID_BYTES
            || version_len == 0
            || version_len > MAX_CORE_VERSION_BYTES
        {
            return Err(SaveStateError::InvalidMetadata);
        }
        if payload_len == 0 {
            return Err(SaveStateError::EmptyPayload);
        }
        if payload_len > MAX_SAVE_STATE_PAYLOAD_BYTES as u64 {
            return Err(SaveStateError::PayloadTooLarge {
                actual: payload_len,
                maximum: MAX_SAVE_STATE_PAYLOAD_BYTES as u64,
            });
        }
        let payload_len = usize::try_from(payload_len).map_err(|_| SaveStateError::Truncated)?;
        let expected_len = FIXED_HEADER_BYTES
            .checked_add(system_len)
            .and_then(|len| len.checked_add(version_len))
            .and_then(|len| len.checked_add(payload_len))
            .ok_or(SaveStateError::Truncated)?;
        if bytes.len() < expected_len {
            return Err(SaveStateError::Truncated);
        }
        if bytes.len() != expected_len {
            return Err(SaveStateError::TrailingData);
        }

        let system_start = FIXED_HEADER_BYTES;
        let version_start = system_start + system_len;
        let payload_start = version_start + version_len;
        let system_id = core::str::from_utf8(&bytes[system_start..version_start])
            .map_err(|_| SaveStateError::InvalidMetadata)?
            .to_owned();
        let core_version = core::str::from_utf8(&bytes[version_start..payload_start])
            .map_err(|_| SaveStateError::InvalidMetadata)?
            .to_owned();
        let metadata = SaveStateMetadata {
            system_id,
            core_version,
        };
        validate_metadata(&metadata)?;
        let payload = bytes[payload_start..].to_vec();
        if crc32(&payload) != expected_crc {
            return Err(SaveStateError::ChecksumMismatch);
        }
        Ok(Self { metadata, payload })
    }
}

fn validate_metadata(metadata: &SaveStateMetadata) -> Result<(), SaveStateError> {
    let id = metadata.system_id.as_bytes();
    if id.is_empty()
        || id.len() > MAX_ID_BYTES
        || !id.iter().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (*byte == b'-' && index > 0 && index + 1 < id.len() && id[index - 1] != b'-')
        })
        || id.last() == Some(&b'-')
        || metadata.system_id.contains("--")
    {
        return Err(SaveStateError::InvalidMetadata);
    }
    let version = metadata.core_version.as_bytes();
    if version.is_empty()
        || version.len() > MAX_CORE_VERSION_BYTES
        || metadata.core_version.trim().is_empty()
        || version.iter().any(|byte| *byte < 0x20 || *byte == 0x7f)
    {
        return Err(SaveStateError::InvalidMetadata);
    }
    Ok(())
}

/// Failure to create, decode or match a save-state envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveStateError {
    /// The envelope is shorter than its fixed header or declared content.
    Truncated,
    /// The envelope format is product-neutral.
    InvalidMagic,
    /// The envelope uses a format version not supported by this implementation.
    UnsupportedFormat(u16),
    /// Metadata fields are malformed or outside their bounds.
    InvalidMetadata,
    /// Payload is empty.
    EmptyPayload,
    /// Payload exceeds the configured hard limit.
    PayloadTooLarge {
        /// Actual or declared payload size.
        actual: u64,
        /// Maximum accepted payload size.
        maximum: u64,
    },
    /// Encoded bytes remain after the declared payload.
    TrailingData,
    /// Payload checksum does not match the encoded checksum.
    ChecksumMismatch,
    /// Save state belongs to a different system.
    IncompatibleSystem,
    /// Save state belongs to another core implementation version.
    IncompatibleCoreVersion,
}

impl fmt::Display for SaveStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("save-state envelope is truncated"),
            Self::InvalidMagic => formatter.write_str("invalid save-state envelope magic"),
            Self::UnsupportedFormat(version) => {
                write!(formatter, "unsupported save-state format version {version}")
            }
            Self::InvalidMetadata => formatter.write_str("invalid save-state metadata"),
            Self::EmptyPayload => formatter.write_str("save-state payload cannot be empty"),
            Self::PayloadTooLarge { actual, maximum } => {
                write!(
                    formatter,
                    "save-state payload {actual} exceeds limit {maximum}"
                )
            }
            Self::TrailingData => formatter.write_str("unexpected trailing save-state data"),
            Self::ChecksumMismatch => formatter.write_str("save-state checksum mismatch"),
            Self::IncompatibleSystem => formatter.write_str("save state belongs to another system"),
            Self::IncompatibleCoreVersion => {
                formatter.write_str("save state belongs to another core version")
            }
        }
    }
}

impl std::error::Error for SaveStateError {}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SaveStateEnvelope {
        SaveStateEnvelope::new("game-boy", "0.1.0", vec![1, 2, 3, 4]).unwrap()
    }

    #[test]
    fn round_trip_preserves_metadata_and_payload() {
        let original = sample();
        let decoded = SaveStateEnvelope::decode(&original.encode()).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn rejects_incompatible_system_and_core_versions() {
        let state = sample();
        assert_eq!(
            state.validate_for("other-system", "0.1.0"),
            Err(SaveStateError::IncompatibleSystem)
        );
        assert_eq!(
            state.validate_for("game-boy", "0.2.0"),
            Err(SaveStateError::IncompatibleCoreVersion)
        );
    }

    #[test]
    fn rejects_corruption_truncation_trailing_data_and_unknown_format() {
        let encoded = sample().encode();
        let mut corrupted = encoded.clone();
        *corrupted.last_mut().unwrap() ^= 0x80;
        assert_eq!(
            SaveStateEnvelope::decode(&corrupted),
            Err(SaveStateError::ChecksumMismatch)
        );
        assert_eq!(
            SaveStateEnvelope::decode(&encoded[..10]),
            Err(SaveStateError::Truncated)
        );
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert_eq!(
            SaveStateEnvelope::decode(&trailing),
            Err(SaveStateError::TrailingData)
        );
        let mut unknown = encoded;
        unknown[8] = 99;
        assert_eq!(
            SaveStateEnvelope::decode(&unknown),
            Err(SaveStateError::UnsupportedFormat(99))
        );
    }

    #[test]
    fn rejects_invalid_metadata_and_empty_payload() {
        assert_eq!(
            SaveStateEnvelope::new("Bad ID", "0.1.0", vec![1]),
            Err(SaveStateError::InvalidMetadata)
        );
        assert_eq!(
            SaveStateEnvelope::new("game-boy", "0.1.0", vec![]),
            Err(SaveStateError::EmptyPayload)
        );
    }

    #[test]
    fn rejects_declared_payload_above_limit_before_allocating() {
        let mut encoded = sample().encode();
        let too_large = (MAX_SAVE_STATE_PAYLOAD_BYTES as u64 + 1).to_le_bytes();
        encoded[14..22].copy_from_slice(&too_large);
        assert_eq!(
            SaveStateEnvelope::decode(&encoded),
            Err(SaveStateError::PayloadTooLarge {
                actual: MAX_SAVE_STATE_PAYLOAD_BYTES as u64 + 1,
                maximum: MAX_SAVE_STATE_PAYLOAD_BYTES as u64,
            })
        );
    }
}
