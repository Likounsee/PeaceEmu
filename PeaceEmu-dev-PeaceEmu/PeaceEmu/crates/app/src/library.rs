#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Lightweight, frontend-independent catalog of user-imported content.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Validation outcome recorded for a library item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentValidation {
    /// The content has not yet passed through a registered format probe and core.
    Pending,
    /// A registered probe and compatible core accepted the content at launch.
    Accepted,
    /// The latest launch attempt rejected the content.
    Rejected(String),
}

/// Result of probing content bytes against the registered format probes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentFormatDetection {
    /// No byte probe has run yet.
    Pending,
    /// No registered probe matched the inspected prefix.
    NoMatch,
    /// Exactly one format probe matched the inspected prefix.
    Detected(String),
    /// More than one format probe matched; launch must resolve compatibility.
    Ambiguous(Vec<String>),
    /// At least one probe requires a longer prefix before deciding.
    NeedsMoreData,
}

/// Live filesystem availability of a library item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentAvailability {
    /// The path currently points to a regular file.
    Available,
    /// The original path no longer exists.
    Missing,
    /// The path exists but is not a regular file.
    NotRegularFile,
    /// Metadata could not be read for a reason other than a missing path.
    Unavailable,
}

/// A content item that has been imported into the current host library.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentEntry {
    path: PathBuf,
    title: String,
    size_bytes: u64,
    format_hint: Option<String>,
    compatible_system_hint: Option<String>,
    format_detection: ContentFormatDetection,
    validation: ContentValidation,
}

impl ContentEntry {
    /// Returns the canonical filesystem path to the content.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the filename-derived title shown in the library.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the content file size captured when it was imported.
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Returns the extension-derived format hint, which is not proof of validity.
    pub fn format_hint(&self) -> Option<&str> {
        self.format_hint.as_deref()
    }

    /// Returns the system family suggested by the extension, if recognized.
    ///
    /// This is a display hint only. The registered format probe and core remain
    /// authoritative for compatibility.
    pub fn compatible_system_hint(&self) -> Option<&str> {
        self.compatible_system_hint.as_deref()
    }

    /// Returns the latest byte-probe result, distinct from the filename extension hint.
    pub fn format_detection(&self) -> &ContentFormatDetection {
        &self.format_detection
    }

    /// Returns the most recently recorded launch-validation outcome.
    pub fn validation(&self) -> &ContentValidation {
        &self.validation
    }

    /// Checks the current filesystem state instead of trusting import-time metadata.
    pub fn availability(&self) -> ContentAvailability {
        match fs::metadata(&self.path) {
            Ok(metadata) if metadata.is_file() => ContentAvailability::Available,
            Ok(_) => ContentAvailability::NotRegularFile,
            Err(error) if error.kind() == io::ErrorKind::NotFound => ContentAvailability::Missing,
            Err(_) => ContentAvailability::Unavailable,
        }
    }
}

/// Errors returned while importing or removing a library entry.
#[derive(Debug)]
pub enum ContentLibraryError {
    /// Filesystem metadata or canonicalization failed.
    Io(io::Error),
    /// The path points to a directory or another non-file object.
    NotAFile(PathBuf),
    /// The file is empty and cannot be launched as content.
    Empty(PathBuf),
    /// The file exceeds the host's configured content-size limit.
    TooLarge {
        /// Canonical path to the oversized content.
        path: PathBuf,
        /// Maximum number of bytes allowed by the caller.
        max_bytes: u64,
        /// Actual size reported by filesystem metadata.
        actual_bytes: u64,
    },
    /// The canonical path is already present in the library.
    AlreadyImported(PathBuf),
    /// No matching entry exists in the library.
    NotImported(PathBuf),
}

impl fmt::Display for ContentLibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "content library filesystem error: {error}"),
            Self::NotAFile(path) => {
                write!(f, "content path is not a regular file: {}", path.display())
            }
            Self::Empty(path) => write!(f, "content file is empty: {}", path.display()),
            Self::TooLarge {
                path,
                max_bytes,
                actual_bytes,
            } => write!(
                f,
                "content file is too large: {} (maximum {max_bytes} bytes, got {actual_bytes})",
                path.display()
            ),
            Self::AlreadyImported(path) => {
                write!(f, "content is already imported: {}", path.display())
            }
            Self::NotImported(path) => {
                write!(f, "content is not in the library: {}", path.display())
            }
        }
    }
}

impl std::error::Error for ContentLibraryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// An in-memory catalog that can be reused by terminal and desktop frontends.
///
/// Importing a path canonicalizes it, requires a regular non-empty file, and
/// enforces the caller's byte limit. Format detection and core-specific
/// validation still happen through the common launch pipeline.
#[derive(Default)]
pub struct ContentLibrary {
    entries: BTreeMap<PathBuf, ContentEntry>,
}

impl ContentLibrary {
    /// Creates an empty content catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns imported entries in stable canonical-path order.
    pub fn entries(&self) -> impl Iterator<Item = &ContentEntry> {
        self.entries.values()
    }

    /// Looks up an entry by its canonical filesystem path.
    ///
    /// The path must use the canonical form exposed by [`ContentEntry::path`].
    /// This avoids a filesystem lookup and is suitable for repeated UI lookups.
    pub fn get(&self, canonical_path: impl AsRef<Path>) -> Option<&ContentEntry> {
        self.entries.get(canonical_path.as_ref())
    }

    /// Returns the number of imported entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when the catalog contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Imports a file after validating its filesystem metadata and size.
    pub fn import(
        &mut self,
        path: impl AsRef<Path>,
        max_bytes: u64,
    ) -> Result<&ContentEntry, ContentLibraryError> {
        let canonical = fs::canonicalize(path.as_ref()).map_err(ContentLibraryError::Io)?;
        let metadata = fs::metadata(&canonical).map_err(ContentLibraryError::Io)?;
        if !metadata.is_file() {
            return Err(ContentLibraryError::NotAFile(canonical));
        }
        let size_bytes = metadata.len();
        if size_bytes == 0 {
            return Err(ContentLibraryError::Empty(canonical));
        }
        if size_bytes > max_bytes {
            return Err(ContentLibraryError::TooLarge {
                path: canonical,
                max_bytes,
                actual_bytes: size_bytes,
            });
        }
        if self.entries.contains_key(&canonical) {
            return Err(ContentLibraryError::AlreadyImported(canonical));
        }
        let title = canonical
            .file_stem()
            .or_else(|| canonical.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| canonical.display().to_string());
        let extension = canonical
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        let compatible_system_hint = extension.as_deref().and_then(|extension| match extension {
            "gb" => Some("Game Boy"),
            "gbc" => Some("Game Boy / Game Boy Color"),
            _ => None,
        });
        let entry = ContentEntry {
            path: canonical.clone(),
            title,
            size_bytes,
            format_hint: extension,
            compatible_system_hint: compatible_system_hint.map(str::to_owned),
            format_detection: ContentFormatDetection::Pending,
            validation: ContentValidation::Pending,
        };
        self.entries.insert(canonical.clone(), entry);
        Ok(self
            .entries
            .get(&canonical)
            .expect("entry was just inserted"))
    }

    /// Records a format-probe result for a canonical library path.
    ///
    /// Returns false if the path is not currently in the catalog.
    pub fn record_format_detection(
        &mut self,
        canonical_path: impl AsRef<Path>,
        detection: ContentFormatDetection,
    ) -> bool {
        if let Some(entry) = self.entries.get_mut(canonical_path.as_ref()) {
            entry.format_detection = detection;
            true
        } else {
            false
        }
    }

    /// Records the latest launch-validation outcome for a canonical library path.
    ///
    /// Returns false if the path is not currently in the catalog.
    pub fn record_validation(
        &mut self,
        canonical_path: impl AsRef<Path>,
        validation: ContentValidation,
    ) -> bool {
        if let Some(entry) = self.entries.get_mut(canonical_path.as_ref()) {
            entry.validation = validation;
            true
        } else {
            false
        }
    }

    /// Removes an imported entry by path, including when its file has disappeared.
    ///
    /// A canonical path returned by this library can be removed even after the
    /// underlying file is deleted or becomes inaccessible. For other path forms,
    /// canonicalization is attempted so relative paths and symlinks keep working.
    pub fn remove(&mut self, path: impl AsRef<Path>) -> Result<ContentEntry, ContentLibraryError> {
        let path = path.as_ref();
        if let Some(entry) = self.entries.remove(path) {
            return Ok(entry);
        }

        let canonical = fs::canonicalize(path).map_err(ContentLibraryError::Io)?;
        self.entries
            .remove(&canonical)
            .ok_or(ContentLibraryError::NotImported(canonical))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str, bytes: &[u8]) -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let directory =
                std::env::temp_dir().join(format!("host-app-library-{}-{id}", std::process::id()));
            fs::create_dir(&directory).unwrap();
            let path = directory.join(name);
            fs::write(&path, bytes).unwrap();
            Self(path)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            if self.0.is_dir() {
                let _ = fs::remove_dir(&self.0);
            } else {
                let _ = fs::remove_file(&self.0);
            }
            if let Some(parent) = self.0.parent() {
                let _ = fs::remove_dir(parent);
            }
        }
    }

    #[test]
    fn import_tracks_canonical_path_title_and_size() {
        let file = TempFile::new("zelda.gb", b"rom-data");
        let mut library = ContentLibrary::new();

        let entry = library.import(&file.0, 64).unwrap();
        assert_eq!(entry.path(), fs::canonicalize(&file.0).unwrap());
        assert_eq!(entry.title(), "zelda");
        assert_eq!(entry.size_bytes(), 8);
        assert_eq!(entry.format_hint(), Some("gb"));
        assert_eq!(entry.compatible_system_hint(), Some("Game Boy"));
        assert_eq!(entry.format_detection(), &ContentFormatDetection::Pending);
        assert_eq!(entry.validation(), &ContentValidation::Pending);
        assert_eq!(entry.availability(), ContentAvailability::Available);
        assert_eq!(library.len(), 1);
    }

    #[test]
    fn lookup_uses_the_canonical_path_and_returns_none_for_unknown_paths() {
        let file = TempFile::new("lookup.gb", b"rom-data");
        let mut library = ContentLibrary::new();
        let canonical = library.import(&file.0, 64).unwrap().path().to_path_buf();

        assert_eq!(library.get(&canonical).unwrap().title(), "lookup");
        assert!(library
            .get(canonical.with_file_name("missing.gb"))
            .is_none());
        assert_eq!(library.len(), 1);
    }

    #[test]
    fn entries_iterate_in_stable_canonical_path_order() {
        let first = TempFile::new("zeta.gb", b"z");
        let second = TempFile::new("alpha.gb", b"a");
        let third = TempFile::new("middle.gb", b"m");
        let mut library = ContentLibrary::new();

        library.import(&first.0, 64).unwrap();
        library.import(&second.0, 64).unwrap();
        library.import(&third.0, 64).unwrap();

        let actual: Vec<_> = library
            .entries()
            .map(|entry| entry.path().to_path_buf())
            .collect();
        let mut expected = vec![
            fs::canonicalize(&first.0).unwrap(),
            fs::canonicalize(&second.0).unwrap(),
            fs::canonicalize(&third.0).unwrap(),
        ];
        expected.sort();

        assert_eq!(actual, expected);
        assert_eq!(library.len(), 3);
    }

    #[test]
    fn import_rejects_duplicates_empty_files_and_oversized_files() {
        let file = TempFile::new("game.gb", b"1234");
        let empty = TempFile::new("empty.gb", b"");
        let mut library = ContentLibrary::new();

        library.import(&file.0, 4).unwrap();
        assert!(matches!(
            library.import(&file.0, 4),
            Err(ContentLibraryError::AlreadyImported(_))
        ));
        assert!(matches!(
            library.import(&empty.0, 4),
            Err(ContentLibraryError::Empty(_))
        ));
        assert!(matches!(
            library.import(&file.0, 3),
            Err(ContentLibraryError::TooLarge { .. })
        ));
    }

    #[test]
    fn import_rejects_missing_paths_without_mutating_catalog() {
        let root = std::env::temp_dir().join(format!(
            "host-app-library-missing-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let missing = root.join("missing.gb");
        let mut library = ContentLibrary::new();

        assert!(matches!(
            library.import(&missing, 64),
            Err(ContentLibraryError::Io(_))
        ));
        assert!(library.is_empty());
        assert_eq!(library.len(), 0);
    }

    #[test]
    fn validation_updates_only_existing_canonical_entries() {
        let file = TempFile::new("validation.gb", b"rom-data");
        let mut library = ContentLibrary::new();
        let canonical = library.import(&file.0, 64).unwrap().path().to_path_buf();

        assert!(library.record_validation(&canonical, ContentValidation::Accepted));
        assert_eq!(
            library.get(&canonical).unwrap().validation(),
            &ContentValidation::Accepted
        );

        let missing = canonical.with_file_name("not-imported.gb");
        assert!(
            !library.record_validation(&missing, ContentValidation::Rejected("invalid".to_owned()))
        );
        assert_eq!(library.len(), 1);
        assert_eq!(
            library.get(&canonical).unwrap().validation(),
            &ContentValidation::Accepted
        );
    }

    #[test]
    fn import_accepts_a_file_exactly_at_the_size_limit() {
        let file = TempFile::new("limit.gb", b"12345678");
        let mut library = ContentLibrary::new();

        let entry = library.import(&file.0, 8).unwrap();
        assert_eq!(entry.size_bytes(), 8);
        assert_eq!(library.len(), 1);
    }

    #[test]
    fn format_detection_updates_only_matching_canonical_entries() {
        let file = TempFile::new("detected.gb", b"rom-data");
        let mut library = ContentLibrary::new();
        library.import(&file.0, 64).unwrap();
        let canonical = fs::canonicalize(&file.0).unwrap();

        assert!(library.record_format_detection(
            &canonical,
            ContentFormatDetection::Detected("game-boy".to_owned())
        ));
        assert_eq!(
            library.entries().next().unwrap().format_detection(),
            &ContentFormatDetection::Detected("game-boy".to_owned())
        );
        assert!(!library.record_format_detection(
            canonical.with_file_name("other.gb"),
            ContentFormatDetection::NoMatch
        ));
        assert_eq!(library.len(), 1);
    }

    #[test]
    fn extension_hints_are_case_insensitive_and_not_validation() {
        let file = TempFile::new("UNKNOWN.GBC", b"unverified");
        let mut library = ContentLibrary::new();

        let entry = library.import(&file.0, 64).unwrap();
        assert_eq!(entry.format_hint(), Some("gbc"));
        assert_eq!(
            entry.compatible_system_hint(),
            Some("Game Boy / Game Boy Color")
        );
        assert_eq!(entry.validation(), &ContentValidation::Pending);
        assert_eq!(entry.format_detection(), &ContentFormatDetection::Pending);
    }

    #[test]
    fn availability_is_rechecked_when_an_imported_file_disappears() {
        let file = TempFile::new("removed.gb", b"rom-data");
        let mut library = ContentLibrary::new();
        library.import(&file.0, 64).unwrap();
        let canonical = fs::canonicalize(&file.0).unwrap();

        assert_eq!(
            library.entries().next().unwrap().availability(),
            ContentAvailability::Available
        );
        fs::remove_file(&canonical).unwrap();
        assert_eq!(
            library.entries().next().unwrap().availability(),
            ContentAvailability::Missing
        );
    }

    #[test]
    fn availability_detects_when_an_imported_file_is_replaced_by_a_directory() {
        let file = TempFile::new("replaced.gb", b"rom-data");
        let mut library = ContentLibrary::new();
        let canonical = library.import(&file.0, 64).unwrap().path().to_path_buf();

        assert_eq!(
            library.entries().next().unwrap().availability(),
            ContentAvailability::Available
        );
        fs::remove_file(&canonical).unwrap();
        fs::create_dir(&canonical).unwrap();

        assert_eq!(
            library.entries().next().unwrap().availability(),
            ContentAvailability::NotRegularFile
        );
    }

    #[test]
    fn validation_outcome_is_recorded_without_claiming_extension_proves_validity() {
        let file = TempFile::new("bad.gb", b"not a valid cartridge");
        let mut library = ContentLibrary::new();
        library.import(&file.0, 64).unwrap();
        let canonical = fs::canonicalize(&file.0).unwrap();

        assert!(library.record_validation(
            &canonical,
            ContentValidation::Rejected("invalid cartridge header".to_owned())
        ));
        assert_eq!(
            library.entries().next().unwrap().validation(),
            &ContentValidation::Rejected("invalid cartridge header".to_owned())
        );
        assert!(!library.record_validation(
            PathBuf::from("not-in-library.gb"),
            ContentValidation::Accepted
        ));
    }

    #[test]
    fn stale_entry_can_be_removed_after_its_file_disappears() {
        let file = TempFile::new("stale.gb", b"rom-data");
        let mut library = ContentLibrary::new();
        let canonical = library.import(&file.0, 64).unwrap().path().to_path_buf();

        fs::remove_file(&canonical).unwrap();
        assert_eq!(
            library.entries().next().unwrap().availability(),
            ContentAvailability::Missing
        );

        let removed = library.remove(&canonical).unwrap();
        assert_eq!(removed.path(), canonical);
        assert!(library.is_empty());
    }

    #[test]
    fn failed_removal_does_not_mutate_other_entries() {
        let file = TempFile::new("kept.gb", b"rom-data");
        let mut library = ContentLibrary::new();
        let canonical = library.import(&file.0, 64).unwrap().path().to_path_buf();
        let missing = canonical.with_file_name("missing.gb");

        assert!(matches!(
            library.remove(&missing),
            Err(ContentLibraryError::Io(_))
        ));
        assert_eq!(library.len(), 1);
        assert_eq!(library.entries().next().unwrap().path(), canonical);
    }

    #[test]
    fn remove_requires_an_imported_path_and_updates_the_catalog() {
        let file = TempFile::new("game.gb", b"rom");
        let mut library = ContentLibrary::new();

        assert!(matches!(
            library.remove(&file.0),
            Err(ContentLibraryError::NotImported(_))
        ));
        library.import(&file.0, 16).unwrap();
        assert_eq!(library.remove(&file.0).unwrap().title(), "game");
        assert!(library.is_empty());
    }

    #[test]
    fn import_rejects_directories() {
        let directory = std::env::temp_dir().join(format!(
            "host-app-library-dir-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let mut library = ContentLibrary::new();

        assert!(matches!(
            library.import(&directory, 64),
            Err(ContentLibraryError::NotAFile(_))
        ));
        fs::remove_dir(directory).unwrap();
    }
}
