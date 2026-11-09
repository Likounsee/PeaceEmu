#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Bounded disk persistence for versioned save-state envelopes.
//!
//! Writes use a same-directory temporary file, sync its contents, preserve the
//! previous primary as a backup, and sync the parent directory where supported.
//! Callers must serialize concurrent writes to the same path.

use core::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::save_state::{SaveStateEnvelope, SaveStateError, MAX_SAVE_STATE_PAYLOAD_BYTES};

/// Maximum accepted encoded save-state file size, including metadata.
pub const MAX_SAVE_STATE_FILE_BYTES: u64 =
    MAX_SAVE_STATE_PAYLOAD_BYTES as u64 + 8 + 2 + 2 + 2 + 8 + 4 + 64 + 128;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Error while loading or persisting a save state.
#[derive(Debug)]
pub enum SaveStateStoreError {
    /// Filesystem operation failed.
    Io(io::Error),
    /// Encoded save-state file exceeds the hard size limit.
    TooLarge,
    /// Envelope decoding or compatibility validation failed.
    Format(SaveStateError),
}

impl fmt::Display for SaveStateStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "save-state I/O error: {error}"),
            Self::TooLarge => formatter.write_str("save-state file exceeds the size limit"),
            Self::Format(error) => write!(formatter, "invalid save-state file: {error}"),
        }
    }
}

impl std::error::Error for SaveStateStoreError {}

impl From<io::Error> for SaveStateStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<SaveStateError> for SaveStateStoreError {
    fn from(error: SaveStateError) -> Self {
        Self::Format(error)
    }
}

/// Store for one save-state slot at an explicit application-selected path.
pub struct SaveStateStore {
    path: PathBuf,
}

impl SaveStateStore {
    /// Creates a store for the supplied primary file path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the configured primary file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the primary envelope, falling back to a valid backup on failure.
    pub fn load(&self) -> Result<SaveStateEnvelope, SaveStateStoreError> {
        match read_envelope(&self.path) {
            Ok(envelope) => Ok(envelope),
            Err(primary_error) => match read_envelope(&backup_path(&self.path)) {
                Ok(envelope) => Ok(envelope),
                Err(SaveStateStoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                    Err(primary_error)
                }
                Err(backup_error) => Err(backup_error),
            },
        }
    }

    /// Persists an envelope through a synced temporary file and recoverable backup.
    pub fn save(&self, envelope: &SaveStateEnvelope) -> Result<(), SaveStateStoreError> {
        let encoded = envelope.encode();
        if encoded.len() as u64 > MAX_SAVE_STATE_FILE_BYTES {
            return Err(SaveStateStoreError::TooLarge);
        }
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;

        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = self
            .path
            .with_extension(format!("tmp-{}-{sequence}", std::process::id()));
        let backup = backup_path(&self.path);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let write_result = file.write_all(&encoded).and_then(|()| file.sync_all());
        drop(file);
        if write_result.is_ok() {
            crash_at_test_checkpoint("temp-synced");
        }
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }

        let had_original = self.path.exists();
        if had_original && backup.exists() {
            if let Err(error) = fs::remove_file(&backup) {
                let _ = fs::remove_file(&temp);
                return Err(error.into());
            }
        }
        if let Err(error) = replace_primary_with_temp(&self.path, &temp, &backup, had_original) {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
        crash_at_test_checkpoint("primary-installed");
        sync_parent_directory(parent)?;
        Ok(())
    }
}

fn read_envelope(path: &Path) -> Result<SaveStateEnvelope, SaveStateStoreError> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    if size > MAX_SAVE_STATE_FILE_BYTES {
        return Err(SaveStateStoreError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_SAVE_STATE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SAVE_STATE_FILE_BYTES {
        return Err(SaveStateStoreError::TooLarge);
    }
    SaveStateEnvelope::decode(&bytes).map_err(Into::into)
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("bak")
}

#[cfg(test)]
fn crash_at_test_checkpoint(checkpoint: &str) {
    if std::env::var("APP_SAVE_CRASH_CHECKPOINT").as_deref() == Ok(checkpoint) {
        std::process::abort();
    }
}

#[cfg(not(test))]
fn crash_at_test_checkpoint(_checkpoint: &str) {}

fn replace_primary_with_temp(
    primary: &Path,
    temp: &Path,
    backup: &Path,
    had_original: bool,
) -> io::Result<()> {
    if had_original {
        platform_rename(primary, backup)?;
        crash_at_test_checkpoint("primary-backed-up");
    }
    if let Err(install_error) = platform_rename(temp, primary) {
        if had_original {
            let _ = platform_rename(backup, primary);
        }
        return Err(install_error);
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> io::Result<()> {
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save_state::SaveStateEnvelope;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "host-app-save-state-{}-{unique}.state",
            std::process::id()
        ))
    }

    #[test]
    fn crash_child_at_requested_save_checkpoint() {
        let Ok(path) = std::env::var("APP_SAVE_CRASH_PATH") else {
            return;
        };
        let envelope = SaveStateEnvelope::new("game-boy", "0.1.0", vec![9, 8, 7]).unwrap();
        SaveStateStore::new(path).save(&envelope).unwrap();
        panic!("requested crash checkpoint was not reached");
    }

    #[test]
    fn process_termination_recovers_only_committed_save_state() {
        use std::process::Command;

        for (checkpoint, expected_new) in [
            ("temp-synced", false),
            ("primary-backed-up", false),
            ("primary-installed", true),
        ] {
            let path = temp_path();
            let original = SaveStateEnvelope::new("game-boy", "0.1.0", vec![1, 2, 3]).unwrap();
            let replacement = SaveStateEnvelope::new("game-boy", "0.1.0", vec![9, 8, 7]).unwrap();
            SaveStateStore::new(&path).save(&original).unwrap();

            let output = Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("save_state_store::tests::crash_child_at_requested_save_checkpoint")
                .arg("--nocapture")
                .env("APP_SAVE_CRASH_PATH", &path)
                .env("APP_SAVE_CRASH_CHECKPOINT", checkpoint)
                .output()
                .unwrap();

            assert!(
                !output.status.success(),
                "child should terminate at checkpoint {checkpoint}; stdout: {} stderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );

            let recovered = SaveStateStore::new(&path).load().unwrap();
            assert_eq!(
                recovered,
                if expected_new { replacement } else { original },
                "unexpected recovered state after process termination at {checkpoint}"
            );

            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(backup_path(&path));
            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
            if let Some(parent) = path.parent() {
                if let Ok(entries) = fs::read_dir(parent) {
                    for entry in entries.flatten() {
                        let candidate = entry.path();
                        if candidate.file_name().is_some_and(|name| {
                            let name = name.to_string_lossy();
                            name.starts_with(stem.as_str()) && name.contains(".tmp-")
                        }) {
                            let _ = fs::remove_file(candidate);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn store_round_trips_an_envelope() {
        let path = temp_path();
        let store = SaveStateStore::new(&path);
        let envelope = SaveStateEnvelope::new("game-boy", "0.1.0", vec![1, 2, 3]).unwrap();

        store.save(&envelope).unwrap();
        assert_eq!(store.load().unwrap(), envelope);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(backup_path(&path));
    }

    #[test]
    fn load_recovers_from_backup_when_primary_is_corrupted() {
        let path = temp_path();
        let store = SaveStateStore::new(&path);
        let original = SaveStateEnvelope::new("game-boy", "0.1.0", vec![1, 2, 3]).unwrap();
        let replacement = SaveStateEnvelope::new("game-boy", "0.1.0", vec![4, 5, 6]).unwrap();

        store.save(&original).unwrap();
        store.save(&replacement).unwrap();
        fs::write(&path, b"corrupt").unwrap();

        assert_eq!(store.load().unwrap(), original);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(backup_path(&path));
    }

    #[test]
    fn store_recovers_backup_when_primary_is_missing() {
        let path = temp_path();
        let store = SaveStateStore::new(&path);
        let original = SaveStateEnvelope::new("game-boy", "0.1.0", vec![1, 2, 3]).unwrap();
        let replacement = SaveStateEnvelope::new("game-boy", "0.1.0", vec![4, 5, 6]).unwrap();

        store.save(&original).unwrap();
        store.save(&replacement).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(store.load().unwrap(), original);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(backup_path(&path));
    }

    #[test]
    fn load_recovers_after_interruption_between_backup_and_primary_install() {
        let path = temp_path();
        let store = SaveStateStore::new(&path);
        let original = SaveStateEnvelope::new("game-boy", "0.1.0", vec![1, 2, 3]).unwrap();
        let replacement = SaveStateEnvelope::new("game-boy", "0.1.0", vec![4, 5, 6]).unwrap();

        store.save(&original).unwrap();

        // Model a process interruption after the old primary was moved to the
        // backup but before the synced replacement temporary file was installed.
        let backup = backup_path(&path);
        fs::rename(&path, &backup).unwrap();
        let interrupted_temp =
            path.with_extension(format!("tmp-{}-interrupted", std::process::id()));
        fs::write(&interrupted_temp, replacement.encode()).unwrap();

        assert_eq!(store.load().unwrap(), original);
        assert_eq!(
            fs::read(&interrupted_temp).unwrap(),
            replacement.encode(),
            "recovery must not promote an uncommitted temporary file"
        );

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&backup);
        let _ = fs::remove_file(&interrupted_temp);
    }

    #[test]
    fn recovery_selects_only_committed_state_at_each_save_checkpoint() {
        let original = SaveStateEnvelope::new("game-boy", "0.1.0", vec![1, 2, 3]).unwrap();
        let replacement = SaveStateEnvelope::new("game-boy", "0.1.0", vec![4, 5, 6]).unwrap();

        // Model the on-disk layouts observable if the process stops at each
        // durable boundary of save(): before the old primary moves, after it
        // moves to the backup, and after the replacement is installed.
        for checkpoint in [
            "temp-created",
            "temp-written",
            "temp-synced",
            "backup-removed",
            "primary-backed-up",
            "replacement-installed",
            "directory-synced",
        ] {
            let path = temp_path().with_extension(format!("{checkpoint}.state"));
            let backup = backup_path(&path);
            let temp = path.with_extension(format!("tmp-{}-checkpoint", std::process::id()));

            fs::write(&path, original.encode()).unwrap();
            if checkpoint == "temp-created" {
                fs::write(&temp, b"").unwrap();
            } else if checkpoint == "temp-written" {
                fs::write(&temp, &replacement.encode()[..8]).unwrap();
            } else if checkpoint != "backup-removed" {
                fs::write(&temp, replacement.encode()).unwrap();
            }

            if checkpoint == "primary-backed-up" {
                fs::rename(&path, &backup).unwrap();
            } else if checkpoint == "replacement-installed" || checkpoint == "directory-synced" {
                fs::write(&backup, original.encode()).unwrap();
                fs::write(&path, replacement.encode()).unwrap();
            }

            let expected =
                if checkpoint == "replacement-installed" || checkpoint == "directory-synced" {
                    &replacement
                } else {
                    &original
                };
            assert_eq!(
                SaveStateStore::new(&path).load().unwrap(),
                *expected,
                "unexpected recovery result at checkpoint {checkpoint}"
            );

            // A valid-looking temporary file is deliberately never promoted:
            // installation of the primary is the commit point.
            if checkpoint == "primary-backed-up" {
                assert_eq!(fs::read(&temp).unwrap(), replacement.encode());
            }
            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(&backup);
            let _ = fs::remove_file(&temp);
        }
    }

    #[test]
    fn load_recovers_from_backup_when_primary_is_oversized() {
        let path = temp_path();
        let store = SaveStateStore::new(&path);
        let original = SaveStateEnvelope::new("game-boy", "0.1.0", vec![1, 2, 3]).unwrap();
        let replacement = SaveStateEnvelope::new("game-boy", "0.1.0", vec![4, 5, 6]).unwrap();

        store.save(&original).unwrap();
        store.save(&replacement).unwrap();
        fs::write(&path, vec![0; MAX_SAVE_STATE_FILE_BYTES as usize + 1]).unwrap();

        assert_eq!(store.load().unwrap(), original);
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            MAX_SAVE_STATE_FILE_BYTES + 1,
            "loading the backup must not silently delete or rewrite the rejected primary"
        );

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(backup_path(&path));
    }

    #[test]
    fn store_rejects_oversized_persisted_files() {
        let path = temp_path();
        fs::write(&path, vec![0; MAX_SAVE_STATE_FILE_BYTES as usize + 1]).unwrap();
        assert!(matches!(
            SaveStateStore::new(&path).load(),
            Err(SaveStateStoreError::TooLarge)
        ));
        let _ = fs::remove_file(&path);
    }
}
