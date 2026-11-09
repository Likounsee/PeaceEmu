//! Bounded artifact staging with an explicit digest-verification boundary.
//!
//! This module deliberately does not implement cryptography or networking. The
//! host supplies a maintained SHA-256 verifier, while this crate enforces the
//! declared byte count and ensures unverified bytes never reach the destination.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Verifies a staged file against a canonical lowercase SHA-256 digest.
///
/// Implementations should use a maintained cryptographic library and must
/// return Ok(false) for a digest mismatch. This trait is a trust boundary:
/// callers must not use a permissive or placeholder implementation in production.
pub trait ArtifactDigestVerifier {
    /// Returns whether the file's SHA-256 digest equals expected_sha256.
    fn verify_sha256(&self, path: &Path, expected_sha256: &str) -> io::Result<bool>;
}

/// Failure while staging an artifact.
#[derive(Debug)]
pub enum ArtifactStageError {
    /// The declared byte count was zero.
    EmptyArtifact,
    /// The expected digest is not exactly 64 lowercase hexadecimal characters.
    InvalidDigest,
    /// Temporary and destination paths refer to the same path.
    PathsMustDiffer,
    /// The artifact contained more bytes than its declared size.
    TooLarge {
        /// Maximum byte count declared by the caller.
        expected: u64,
    },
    /// The stream ended before its declared size.
    TooSmall {
        /// Byte count declared by the caller.
        expected: u64,
        /// Number of bytes actually read.
        actual: u64,
    },
    /// The digest verifier rejected the staged bytes.
    DigestMismatch,
    /// The destination already exists and will not be overwritten.
    DestinationExists,
    /// An underlying filesystem or reader operation failed.
    Io(io::Error),
}

impl std::fmt::Display for ArtifactStageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyArtifact => f.write_str("artifact size must be greater than zero"),
            Self::InvalidDigest => {
                f.write_str("expected SHA-256 must be 64 lowercase hexadecimal characters")
            }
            Self::PathsMustDiffer => f.write_str("temporary and destination paths must differ"),
            Self::TooLarge { expected } => {
                write!(f, "artifact exceeds declared size of {expected} bytes")
            }
            Self::TooSmall { expected, actual } => {
                write!(f, "artifact ended at {actual} bytes; expected {expected}")
            }
            Self::DigestMismatch => {
                f.write_str("artifact SHA-256 digest does not match the manifest")
            }
            Self::DestinationExists => f.write_str("artifact destination already exists"),
            Self::Io(error) => write!(f, "artifact staging I/O failed: {error}"),
        }
    }
}

impl std::error::Error for ArtifactStageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ArtifactStageError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Copies a package stream into a temporary file, checks its exact size and
/// digest, then renames it to the destination only after all checks succeed.
///
/// temporary_path should be in the same filesystem as destination_path so the
/// final rename is atomic on supported filesystems. The temporary file is
/// created exclusively; an existing temporary file is never truncated. Failed
/// operations remove the temporary file when possible. This function does not
/// fetch URLs, unpack archives, activate modules, or verify manifest signatures.
pub fn stage_verified_artifact<R: Read, V: ArtifactDigestVerifier>(
    reader: &mut R,
    temporary_path: impl AsRef<Path>,
    destination_path: impl AsRef<Path>,
    expected_size: u64,
    expected_sha256: &str,
    verifier: &V,
) -> Result<PathBuf, ArtifactStageError> {
    let temporary_path = temporary_path.as_ref();
    let destination_path = destination_path.as_ref();

    if expected_size == 0 {
        return Err(ArtifactStageError::EmptyArtifact);
    }
    if !is_canonical_sha256(expected_sha256) {
        return Err(ArtifactStageError::InvalidDigest);
    }
    if temporary_path == destination_path {
        return Err(ArtifactStageError::PathsMustDiffer);
    }
    if fs::symlink_metadata(destination_path).is_ok() {
        return Err(ArtifactStageError::DestinationExists);
    }

    let result = stage_inner(
        reader,
        temporary_path,
        destination_path,
        expected_size,
        expected_sha256,
        verifier,
    );
    if result.is_err() {
        let _ = fs::remove_file(temporary_path);
    }
    result?;
    Ok(destination_path.to_path_buf())
}

fn stage_inner<R: Read, V: ArtifactDigestVerifier>(
    reader: &mut R,
    temporary_path: &Path,
    destination_path: &Path,
    expected_size: u64,
    expected_sha256: &str,
    verifier: &V,
) -> Result<(), ArtifactStageError> {
    let mut file = OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(temporary_path)?;

    let mut total = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let next = total.saturating_add(count as u64);
        if next > expected_size {
            return Err(ArtifactStageError::TooLarge {
                expected: expected_size,
            });
        }
        file.write_all(&buffer[..count])?;
        total = next;
    }

    if total != expected_size {
        return Err(ArtifactStageError::TooSmall {
            expected: expected_size,
            actual: total,
        });
    }

    file.flush()?;
    file.sync_all()?;
    drop(file);

    if !verifier.verify_sha256(temporary_path, expected_sha256)? {
        return Err(ArtifactStageError::DigestMismatch);
    }
    if fs::symlink_metadata(destination_path).is_ok() {
        return Err(ArtifactStageError::DestinationExists);
    }
    fs::rename(temporary_path, destination_path)?;
    Ok(())
}

fn is_canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
