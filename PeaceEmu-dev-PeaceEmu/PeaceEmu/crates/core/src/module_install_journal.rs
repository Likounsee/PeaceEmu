#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Crash-recovery journal for package installation transactions.
//!
//! The journal records intent and durable milestones, not package bytes. The
//! installer must persist each transition after completing the corresponding
//! filesystem operation and reconcile the journal with installed state on startup.

use core::fmt;
#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::module_management::ModuleId;

/// Maximum encoded journal size accepted from disk.
pub const MAX_INSTALL_JOURNAL_BYTES: u64 = 1024 * 1024;
const HEADER: &str = "PEACEEMU-INSTALL-JOURNAL\t1";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Durable milestone of one package installation transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallPhase {
    /// Dependencies and compatibility have been checked.
    Planned,
    /// Artifact transfer is in progress or can be retried.
    Downloading,
    /// Downloaded bytes have passed size and digest verification.
    Verified,
    /// Verified package has been unpacked into an isolated staging directory.
    Staged,
    /// Activation is about to begin; recovery must inspect the final location.
    Activating,
    /// Staged package has been activated at its final location.
    Activated,
    /// Installed state was persisted; the transaction is complete.
    Committed,
    /// Installation stopped with an error and requires cleanup or retry.
    Failed,
}

impl InstallPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Downloading => "downloading",
            Self::Verified => "verified",
            Self::Staged => "staged",
            Self::Activating => "activating",
            Self::Activated => "activated",
            Self::Committed => "committed",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "planned" => Some(Self::Planned),
            "downloading" => Some(Self::Downloading),
            "verified" => Some(Self::Verified),
            "staged" => Some(Self::Staged),
            "activating" => Some(Self::Activating),
            "activated" => Some(Self::Activated),
            "committed" => Some(Self::Committed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    fn can_transition_to(self, next: Self) -> bool {
        use InstallPhase::*;
        matches!(
            (self, next),
            (Planned, Downloading)
                | (Planned, Failed)
                | (Downloading, Downloading)
                | (Downloading, Verified)
                | (Downloading, Failed)
                | (Verified, Staged)
                | (Verified, Failed)
                | (Staged, Activating)
                | (Staged, Failed)
                | (Activating, Activated)
                | (Activating, Failed)
                | (Activated, Committed)
                | (Activated, Failed)
                | (Failed, Downloading)
                | (Failed, Verified)
                | (Failed, Staged)
        )
    }
}

/// Recovery action implied by the last durable installation milestone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallRecovery {
    /// Nothing was activated; discard temporary files and retry or cancel.
    DiscardStaging,
    /// Activation may have succeeded; reconcile the final directory with state.
    ReconcileActivation,
    /// State is committed; remove any leftover journal and staging files.
    CleanupCommitted,
    /// A failed transaction must be inspected and either cleaned up or retried.
    InspectFailure,
}

/// One journaled package operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallJournalEntry {
    /// Stable module ID.
    pub module: ModuleId,
    /// Exact package version.
    pub version: String,
    /// Last durably completed milestone.
    pub phase: InstallPhase,
}

/// Discovers journal files in one transaction directory in deterministic path order.
///
/// Only direct, regular files with the `.journal` extension are considered.
/// Temporary files and symlinks are ignored. A malformed journal or filesystem
/// error is returned to the caller so startup recovery cannot silently skip a
/// transaction that may need attention.
pub fn discover_install_journals(
    directory: impl AsRef<Path>,
) -> Result<Vec<(PathBuf, InstallJournal)>, InstallJournalError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .map_or(true, |extension| extension != "journal")
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_file() {
            paths.push(path);
        }
    }
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let journal = InstallJournal::load(&path)?;
            Ok((path, journal))
        })
        .collect()
}

/// Rejects directories and symbolic links before a cleanup operation.
fn ensure_regular_journal_file(path: &Path) -> Result<(), InstallJournalError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(InstallJournalError::InvalidPath);
    }
    Ok(())
}

/// Removes a journal only after its committed phase is durably recorded.
///
/// On Unix, synchronizes the containing directory after unlinking the journal.
/// Non-committed journals are preserved and reported as an error so callers
/// cannot accidentally erase recovery information for an incomplete install.
pub fn remove_committed_install_journal(path: impl AsRef<Path>) -> Result<(), InstallJournalError> {
    let path = path.as_ref();
    ensure_regular_journal_file(path)?;
    let journal = InstallJournal::load(path)?;
    if journal.entry().phase != InstallPhase::Committed {
        return Err(InstallJournalError::NotCommitted(journal.entry().phase));
    }

    fs::remove_file(path)?;
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Removes a journal only when its phase proves activation never began.
///
/// The caller must first successfully clean the transaction's isolated staging
/// files. Journals at or after the activation phase, including Failed, are
/// rejected because activation may have changed the final installation.
pub fn remove_discarded_install_journal(path: impl AsRef<Path>) -> Result<(), InstallJournalError> {
    let path = path.as_ref();
    ensure_regular_journal_file(path)?;
    let journal = InstallJournal::load(path)?;
    match journal.entry().phase {
        InstallPhase::Planned
        | InstallPhase::Downloading
        | InstallPhase::Verified
        | InstallPhase::Staged => {}
        phase => return Err(InstallJournalError::UnsafeDiscard(phase)),
    }

    fs::remove_file(path)?;
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// A non-destructive startup recovery decision derived from a durable journal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallRecoveryPlan {
    /// Path of the journal that must be handled.
    pub journal_path: PathBuf,
    /// Stable module ID recorded by the transaction.
    pub module: ModuleId,
    /// Exact package version recorded by the transaction.
    pub version: String,
    /// Last durably recorded transaction phase.
    pub phase: InstallPhase,
    /// Recovery action appropriate for the recorded phase.
    pub action: InstallRecovery,
}

/// Builds a deterministic recovery plan for every journal in a directory.
///
/// This function only reads and validates journals; it never deletes staging
/// data, changes installed state, or removes journals. Callers must execute the
/// returned actions through a trusted backend and preserve the activation safety
/// boundary represented by each action.
pub fn plan_install_recovery(
    directory: impl AsRef<Path>,
) -> Result<Vec<InstallRecoveryPlan>, InstallJournalError> {
    discover_install_journals(directory)?
        .into_iter()
        .map(|(journal_path, journal)| {
            let entry = journal.entry();
            Ok(InstallRecoveryPlan {
                journal_path,
                module: entry.module.clone(),
                version: entry.version.clone(),
                phase: entry.phase,
                action: journal.recovery_action(),
            })
        })
        .collect()
}

/// A bounded, versioned installation journal for one package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallJournal {
    entry: InstallJournalEntry,
}

impl InstallJournal {
    /// Starts a transaction after the install plan has been validated.
    pub fn new(module: ModuleId, version: impl Into<String>) -> Result<Self, InstallJournalError> {
        let version = version.into();
        if version.trim().is_empty() || version.chars().any(|ch| matches!(ch, '\n' | '\r' | '\t')) {
            return Err(InstallJournalError::InvalidVersion);
        }
        Ok(Self {
            entry: InstallJournalEntry {
                module,
                version,
                phase: InstallPhase::Planned,
            },
        })
    }

    /// Returns the journal entry.
    pub fn entry(&self) -> &InstallJournalEntry {
        &self.entry
    }

    /// Advances the transaction only along an allowed transition.
    pub fn advance(&mut self, next: InstallPhase) -> Result<(), InstallJournalError> {
        if !self.entry.phase.can_transition_to(next) {
            return Err(InstallJournalError::InvalidTransition {
                from: self.entry.phase,
                to: next,
            });
        }
        self.entry.phase = next;
        Ok(())
    }

    /// Returns the startup recovery action for the last recorded milestone.
    pub fn recovery_action(&self) -> InstallRecovery {
        match self.entry.phase {
            InstallPhase::Planned
            | InstallPhase::Downloading
            | InstallPhase::Verified
            | InstallPhase::Staged => InstallRecovery::DiscardStaging,
            InstallPhase::Activating | InstallPhase::Activated => {
                InstallRecovery::ReconcileActivation
            }
            InstallPhase::Committed => InstallRecovery::CleanupCommitted,
            InstallPhase::Failed => InstallRecovery::InspectFailure,
        }
    }

    /// Encodes a deterministic, line-oriented version-1 journal.
    pub fn encode(&self) -> String {
        format!(
            "{HEADER}\nmodule\t{}\nversion\t{}\nphase\t{}\n",
            self.entry.module.as_str(),
            encode_hex(self.entry.version.as_bytes()),
            self.entry.phase.as_str(),
        )
    }

    /// Decodes a journal while rejecting oversized, malformed, or ambiguous records.
    pub fn decode(input: &str) -> Result<Self, InstallJournalError> {
        if input.len() as u64 > MAX_INSTALL_JOURNAL_BYTES {
            return Err(InstallJournalError::TooLarge);
        }
        let mut lines = input.lines();
        if lines.next() != Some(HEADER) {
            return Err(InstallJournalError::InvalidFormat);
        }
        let mut module = None;
        let mut version = None;
        let mut phase = None;
        for line in lines {
            let fields: Vec<_> = line.split('\t').collect();
            match fields.as_slice() {
                ["module", value] if module.is_none() => {
                    module = Some(
                        ModuleId::new(*value).map_err(|_| InstallJournalError::InvalidFormat)?,
                    );
                }
                ["version", value] if version.is_none() => {
                    let bytes = decode_hex(value)?;
                    let value =
                        String::from_utf8(bytes).map_err(|_| InstallJournalError::InvalidFormat)?;
                    if value.trim().is_empty()
                        || value.chars().any(|ch| matches!(ch, '\n' | '\r' | '\t'))
                    {
                        return Err(InstallJournalError::InvalidVersion);
                    }
                    version = Some(value);
                }
                ["phase", value] if phase.is_none() => {
                    phase =
                        Some(InstallPhase::parse(value).ok_or(InstallJournalError::InvalidFormat)?);
                }
                _ => return Err(InstallJournalError::InvalidFormat),
            }
        }
        Ok(Self {
            entry: InstallJournalEntry {
                module: module.ok_or(InstallJournalError::InvalidFormat)?,
                version: version.ok_or(InstallJournalError::InvalidFormat)?,
                phase: phase.ok_or(InstallJournalError::InvalidFormat)?,
            },
        })
    }

    /// Loads a journal with a strict size bound.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, InstallJournalError> {
        let file = OpenOptions::new().read(true).open(path)?;
        if file.metadata()?.len() > MAX_INSTALL_JOURNAL_BYTES {
            return Err(InstallJournalError::TooLarge);
        }
        let mut input = Vec::new();
        file.take(MAX_INSTALL_JOURNAL_BYTES + 1)
            .read_to_end(&mut input)?;
        if input.len() as u64 > MAX_INSTALL_JOURNAL_BYTES {
            return Err(InstallJournalError::TooLarge);
        }
        let input = std::str::from_utf8(&input).map_err(|_| InstallJournalError::InvalidFormat)?;
        Self::decode(input)
    }

    /// Atomically replaces the journal file using a sibling temporary file.
    ///
    /// The parent directory must already exist. The temporary file is flushed
    /// before replacement; Unix directory metadata is then synchronized, and
    /// Windows replacement uses write-through semantics.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), InstallJournalError> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = path.file_name().ok_or(InstallJournalError::InvalidPath)?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = name.to_os_string();
        temp_name.push(format!(".tmp-{}-{sequence}", std::process::id()));
        let temp_path: PathBuf = parent.join(temp_name);
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)?;
            file.write_all(self.encode().as_bytes())?;
            file.sync_all()?;
            drop(file);
            platform_rename(&temp_path, path)?;
            sync_parent_directory(parent)?;
            Ok::<(), io::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        result.map_err(InstallJournalError::Io)
    }
}

/// Replaces a journal using write-through semantics on Windows.
fn platform_rename(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows_durability::rename_write_through(from, to)
    }

    #[cfg(not(windows))]
    {
        fs::rename(from, to)
    }
}

/// Flushes directory metadata on Unix after replacing the journal.
#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

/// Windows requests write-through completion as part of the rename operation.
#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> io::Result<()> {
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex(value: &str) -> Result<Vec<u8>, InstallJournalError> {
    if value.len() % 2 != 0 {
        return Err(InstallJournalError::InvalidFormat);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_digit(pair[0]).ok_or(InstallJournalError::InvalidFormat)?;
            let low = hex_digit(pair[1]).ok_or(InstallJournalError::InvalidFormat)?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

/// Errors from the installation journal.
#[derive(Debug)]
pub enum InstallJournalError {
    /// Journal file I/O failed.
    Io(io::Error),
    /// Journal exceeds the accepted size limit.
    TooLarge,
    /// Encoded journal is malformed or unsupported.
    InvalidFormat,
    /// Package version is empty or contains record separators.
    InvalidVersion,
    /// A requested phase transition is not permitted.
    InvalidTransition {
        /// Current phase recorded in the journal.
        from: InstallPhase,
        /// Requested next phase.
        to: InstallPhase,
    },
    /// Destination path does not name a file.
    InvalidPath,
    /// A journal cannot be removed as committed before reaching that phase.
    NotCommitted(InstallPhase),
    /// A journal cannot be discarded because activation may have started.
    UnsafeDiscard(InstallPhase),
}

impl fmt::Display for InstallJournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "installation journal I/O error: {error}"),
            Self::TooLarge => f.write_str("installation journal exceeds 1 MiB"),
            Self::InvalidFormat => f.write_str("invalid installation journal format"),
            Self::InvalidVersion => f.write_str("invalid package version in installation journal"),
            Self::InvalidTransition { from, to } => {
                write!(f, "invalid install phase transition: {from:?} -> {to:?}")
            }
            Self::InvalidPath => f.write_str("installation journal path must name a file"),
            Self::NotCommitted(phase) => write!(
                f,
                "cannot remove install journal in phase {phase:?} as committed"
            ),
            Self::UnsafeDiscard(phase) => write!(
                f,
                "cannot discard install journal in phase {phase:?}: activation may have started"
            ),
        }
    }
}

impl std::error::Error for InstallJournalError {}
impl From<io::Error> for InstallJournalError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_action_preserves_activation_safety_boundary() {
        let module = ModuleId::new("test-module").unwrap();
        let cases = [
            (InstallPhase::Planned, InstallRecovery::DiscardStaging),
            (InstallPhase::Downloading, InstallRecovery::DiscardStaging),
            (InstallPhase::Verified, InstallRecovery::DiscardStaging),
            (InstallPhase::Staged, InstallRecovery::DiscardStaging),
            (
                InstallPhase::Activating,
                InstallRecovery::ReconcileActivation,
            ),
            (
                InstallPhase::Activated,
                InstallRecovery::ReconcileActivation,
            ),
            (InstallPhase::Committed, InstallRecovery::CleanupCommitted),
            (InstallPhase::Failed, InstallRecovery::InspectFailure),
        ];

        for (phase, expected) in cases {
            let journal = InstallJournal {
                entry: InstallJournalEntry {
                    module: module.clone(),
                    version: "1.0.0".to_owned(),
                    phase,
                },
            };
            assert_eq!(
                journal.recovery_action(),
                expected,
                "unexpected recovery action for durable phase {phase:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn journal_cleanup_rejects_symbolic_links() {
        let directory = std::env::temp_dir().join(format!(
            "host-app-journal-symlink-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let target = directory.join("target.journal");
        let link = directory.join("link.journal");
        let mut journal =
            InstallJournal::new(ModuleId::new("test-module").unwrap(), "1.0.0").unwrap();
        journal.advance(InstallPhase::Downloading).unwrap();
        journal.advance(InstallPhase::Verified).unwrap();
        journal.advance(InstallPhase::Staged).unwrap();
        journal.advance(InstallPhase::Activating).unwrap();
        journal.advance(InstallPhase::Activated).unwrap();
        journal.advance(InstallPhase::Committed).unwrap();
        journal.save(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(matches!(
            remove_committed_install_journal(&link),
            Err(InstallJournalError::InvalidPath)
        ));
        assert!(target.exists());
        assert!(link.exists());

        fs::remove_file(link).unwrap();
        fs::remove_file(target).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn discarded_journal_removal_refuses_activation_and_failed_phases() {
        let directory = std::env::temp_dir().join(format!(
            "host-app-discard-journal-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("transaction.journal");
        let mut journal =
            InstallJournal::new(ModuleId::new("test-module").unwrap(), "1.0.0").unwrap();
        journal.advance(InstallPhase::Downloading).unwrap();
        journal.advance(InstallPhase::Verified).unwrap();
        journal.advance(InstallPhase::Staged).unwrap();
        journal.save(&path).unwrap();

        remove_discarded_install_journal(&path).unwrap();
        assert!(!path.exists());

        let mut journal =
            InstallJournal::new(ModuleId::new("test-module").unwrap(), "1.0.0").unwrap();
        journal.advance(InstallPhase::Downloading).unwrap();
        journal.advance(InstallPhase::Verified).unwrap();
        journal.advance(InstallPhase::Staged).unwrap();
        journal.advance(InstallPhase::Activating).unwrap();
        journal.advance(InstallPhase::Failed).unwrap();
        journal.save(&path).unwrap();

        assert!(matches!(
            remove_discarded_install_journal(&path),
            Err(InstallJournalError::UnsafeDiscard(InstallPhase::Failed))
        ));
        assert!(path.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn committed_journal_removal_refuses_incomplete_transaction() {
        let directory = std::env::temp_dir().join(format!(
            "host-app-committed-journal-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("transaction.journal");
        let journal = InstallJournal::new(ModuleId::new("test-module").unwrap(), "1.0.0").unwrap();
        journal.save(&path).unwrap();

        assert!(matches!(
            remove_committed_install_journal(&path),
            Err(InstallJournalError::NotCommitted(InstallPhase::Planned))
        ));
        assert!(path.exists());

        let mut committed = InstallJournal::load(&path).unwrap();
        committed.advance(InstallPhase::Downloading).unwrap();
        committed.advance(InstallPhase::Verified).unwrap();
        committed.advance(InstallPhase::Staged).unwrap();
        committed.advance(InstallPhase::Activating).unwrap();
        committed.advance(InstallPhase::Activated).unwrap();
        committed.advance(InstallPhase::Committed).unwrap();
        committed.save(&path).unwrap();

        remove_committed_install_journal(&path).unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn recovery_plan_exposes_action_without_mutating_journals() {
        let directory = std::env::temp_dir().join(format!(
            "host-app-recovery-plan-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("transaction.journal");
        let mut journal =
            InstallJournal::new(ModuleId::new("test-module").unwrap(), "1.2.3").unwrap();
        journal.advance(InstallPhase::Downloading).unwrap();
        journal.advance(InstallPhase::Verified).unwrap();
        journal.advance(InstallPhase::Staged).unwrap();
        journal.advance(InstallPhase::Activating).unwrap();
        journal.save(&path).unwrap();
        let before = fs::read(&path).unwrap();

        let plans = plan_install_recovery(&directory).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].journal_path, path);
        assert_eq!(plans[0].module.as_str(), "test-module");
        assert_eq!(plans[0].version, "1.2.3");
        assert_eq!(plans[0].phase, InstallPhase::Activating);
        assert_eq!(plans[0].action, InstallRecovery::ReconcileActivation);
        assert_eq!(fs::read(&plans[0].journal_path).unwrap(), before);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn discovery_returns_only_journals_in_deterministic_order() {
        let directory = std::env::temp_dir().join(format!(
            "host-app-journal-discovery-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();

        let first = directory.join("a.journal");
        let second = directory.join("z.journal");
        let temporary = directory.join("a.journal.tmp-123");
        InstallJournal::new(ModuleId::new("module-a").unwrap(), "1.0.0")
            .unwrap()
            .save(&first)
            .unwrap();
        InstallJournal::new(ModuleId::new("module-z").unwrap(), "2.0.0")
            .unwrap()
            .save(&second)
            .unwrap();
        fs::write(&temporary, b"partial").unwrap();
        fs::write(directory.join("notes.txt"), b"not a journal").unwrap();

        let found = discover_install_journals(&directory).unwrap();
        assert_eq!(
            found
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>(),
            vec![first.clone(), second.clone()]
        );
        assert_eq!(found[0].1.entry().module.as_str(), "module-a");
        assert_eq!(found[1].1.entry().module.as_str(), "module-z");

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn discovery_fails_closed_on_malformed_journal() {
        let directory = std::env::temp_dir().join(format!(
            "host-app-journal-malformed-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("broken.journal"), b"not a journal").unwrap();

        assert!(matches!(
            discover_install_journals(&directory),
            Err(InstallJournalError::InvalidFormat)
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn journal_round_trips_module_version_and_phase() {
        let mut journal =
            InstallJournal::new(ModuleId::new("test-module").unwrap(), "1.2.3+build.7").unwrap();
        journal.advance(InstallPhase::Downloading).unwrap();
        journal.advance(InstallPhase::Verified).unwrap();

        let encoded = journal.encode();
        let decoded = InstallJournal::decode(&encoded).unwrap();

        assert_eq!(decoded, journal);
        assert_eq!(decoded.entry().module.as_str(), "test-module");
        assert_eq!(decoded.entry().version, "1.2.3+build.7");
        assert_eq!(decoded.entry().phase, InstallPhase::Verified);
    }

    #[test]
    fn invalid_transition_does_not_change_last_durable_phase() {
        let mut journal =
            InstallJournal::new(ModuleId::new("test-module").unwrap(), "1.0.0").unwrap();

        assert!(matches!(
            journal.advance(InstallPhase::Committed),
            Err(InstallJournalError::InvalidTransition {
                from: InstallPhase::Planned,
                to: InstallPhase::Committed,
            })
        ));
        assert_eq!(journal.entry().phase, InstallPhase::Planned);
    }

    #[test]
    fn decoder_rejects_duplicate_and_unknown_records() {
        let duplicate = format!(
            "{HEADER}\nmodule\\ttest-module\nmodule\\tother-module\nversion\\t312e302e30\nphase\\tplanned\n"
        )
        .replace("\\t", "\t");
        assert!(matches!(
            InstallJournal::decode(&duplicate),
            Err(InstallJournalError::InvalidFormat)
        ));

        let unknown = format!(
            "{HEADER}\nmodule\ttest-module\nversion\t312e302e30\nphase\tplanned\nextra\tvalue\n"
        );
        assert!(matches!(
            InstallJournal::decode(&unknown),
            Err(InstallJournalError::InvalidFormat)
        ));
    }

    #[test]
    fn recovery_action_tracks_activation_and_commit_boundaries() {
        let mut journal =
            InstallJournal::new(ModuleId::new("test-module").unwrap(), "1.0.0").unwrap();
        assert_eq!(journal.recovery_action(), InstallRecovery::DiscardStaging);

        journal.advance(InstallPhase::Downloading).unwrap();
        journal.advance(InstallPhase::Verified).unwrap();
        journal.advance(InstallPhase::Staged).unwrap();
        journal.advance(InstallPhase::Activating).unwrap();
        assert_eq!(
            journal.recovery_action(),
            InstallRecovery::ReconcileActivation
        );

        journal.advance(InstallPhase::Activated).unwrap();
        journal.advance(InstallPhase::Committed).unwrap();
        assert_eq!(journal.recovery_action(), InstallRecovery::CleanupCommitted);
    }

    #[test]
    fn save_replaces_existing_journal_and_loads_latest_phase() {
        let path = std::env::temp_dir().join(format!(
            "host-app-install-journal-{}-{}.journal",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut journal =
            InstallJournal::new(ModuleId::new("test-module").unwrap(), "1.0.0").unwrap();
        journal.save(&path).unwrap();

        journal.advance(InstallPhase::Downloading).unwrap();
        journal.advance(InstallPhase::Verified).unwrap();
        journal.save(&path).unwrap();

        let loaded = InstallJournal::load(&path).unwrap();
        assert_eq!(loaded, journal);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn load_rejects_malformed_journal_without_repairing_it() {
        let path = std::env::temp_dir().join(format!(
            "host-app-install-journal-invalid-{}-{}.journal",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, "not a journal\\n").unwrap();

        assert!(matches!(
            InstallJournal::load(&path),
            Err(InstallJournalError::InvalidFormat)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "not a journal\\n");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn load_reports_invalid_utf8_as_malformed_journal_without_modifying_file() {
        let path = std::env::temp_dir().join(format!(
            "host-app-install-journal-utf8-{}-{}.journal",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let bytes = b"PEACEEMU-INSTALL-JOURNAL\t1\n\xff";
        fs::write(&path, bytes).unwrap();

        assert!(matches!(
            InstallJournal::load(&path),
            Err(InstallJournalError::InvalidFormat)
        ));
        assert_eq!(fs::read(&path).unwrap(), bytes);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn load_rejects_oversized_journal_before_decoding() {
        let path = std::env::temp_dir().join(format!(
            "host-app-install-journal-large-{}-{}.journal",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, vec![b'x'; MAX_INSTALL_JOURNAL_BYTES as usize + 1]).unwrap();

        assert!(matches!(
            InstallJournal::load(&path),
            Err(InstallJournalError::TooLarge)
        ));
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            MAX_INSTALL_JOURNAL_BYTES + 1,
            "loading an oversized journal must not modify or truncate it"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn constructor_rejects_empty_or_record_separated_versions() {
        for version in ["", "  ", "1.0\nphase\tcommitted", "1.0\r", "1.0\tbad"] {
            assert!(matches!(
                InstallJournal::new(ModuleId::new("test-module").unwrap(), version),
                Err(InstallJournalError::InvalidVersion)
            ));
        }
    }
}
