#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Cross-platform path validation for untrusted package archive entries.
//!
//! This is a prerequisite for extraction, not an extractor. Callers must still
//! prevent symlink/hard-link traversal and create files beneath a trusted root.

use core::fmt;

/// A validated archive path made only of ordinary relative components.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SafeArchivePath {
    normalized: String,
}

impl SafeArchivePath {
    /// Returns the slash-separated relative path suitable for joining component-by-component.
    pub fn as_str(&self) -> &str {
        &self.normalized
    }

    /// Returns validated path components without interpreting host-specific separators.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.normalized.split('/')
    }

    /// Joins the validated components beneath a caller-provided extraction root.
    ///
    /// The caller must ensure the root is trusted and that no component is replaced
    /// by a symlink during extraction.
    pub fn join_under(&self, root: impl AsRef<std::path::Path>) -> std::path::PathBuf {
        let mut path = root.as_ref().to_path_buf();
        for component in self.components() {
            path.push(component);
        }
        path
    }
}

/// Reason an archive entry path was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchivePathError {
    /// The path is empty.
    Empty,
    /// The path is absolute or uses a platform-specific rooted form.
    Absolute,
    /// The path contains a parent or current-directory traversal component.
    Traversal,
    /// The path contains a backslash, colon, control character, or invalid separator.
    InvalidCharacter,
    /// A component aliases a reserved Windows device name or ends in dot/space.
    PlatformAmbiguous,
}

impl fmt::Display for ArchivePathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("archive entry path cannot be empty"),
            Self::Absolute => f.write_str("archive entry path must be relative"),
            Self::Traversal => f.write_str("archive entry path cannot contain . or .. components"),
            Self::InvalidCharacter => {
                f.write_str("archive entry path contains an invalid character or separator")
            }
            Self::PlatformAmbiguous => {
                f.write_str("archive entry path contains a platform-ambiguous component")
            }
        }
    }
}
impl std::error::Error for ArchivePathError {}

/// Validates an untrusted archive entry name for portable extraction.
///
/// The accepted grammar uses `/` as its only separator. Empty, `.`, and `..`
/// components are rejected; so are absolute paths, drive/stream colons, control
/// characters, Windows device names, and trailing dots/spaces. Directory entries
/// may include one trailing `/`, which is removed from the returned normalized path.
pub fn validate_archive_entry_path(value: &str) -> Result<SafeArchivePath, ArchivePathError> {
    if value.is_empty() {
        return Err(ArchivePathError::Empty);
    }
    if value.starts_with('/') {
        return Err(ArchivePathError::Absolute);
    }
    if value.contains('\\') || value.contains(':') || value.chars().any(char::is_control) {
        return Err(ArchivePathError::InvalidCharacter);
    }
    let value = value.strip_suffix('/').unwrap_or(value);
    if value.is_empty() {
        return Err(ArchivePathError::Absolute);
    }
    let parts: Vec<&str> = value.split('/').collect();
    for part in &parts {
        if *part == "." || *part == ".." {
            return Err(ArchivePathError::Traversal);
        }
        if part.is_empty() {
            return Err(ArchivePathError::InvalidCharacter);
        }
        if part.ends_with('.') || part.ends_with(' ') {
            return Err(ArchivePathError::PlatformAmbiguous);
        }
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if reserved {
            return Err(ArchivePathError::PlatformAmbiguous);
        }
    }
    Ok(SafeArchivePath {
        normalized: parts.join("/"),
    })
}
