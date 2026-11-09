#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Versioned persistence for first-launch and module state.

use core::fmt;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::module_management::ModuleId;
use crate::module_state::{FirstLaunchStatus, InstalledModule, ModuleManagerState};

/// Maximum accepted state-file size (4 MiB).
pub const MAX_STATE_FILE_BYTES: u64 = 4 * 1024 * 1024;
const HEADER: &str = "PEACEEMU-MODULE-STATE\t1";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Resolves the legacy per-user state path without creating it.
///
/// Applications may instead pass an explicit path to ModuleStateStore::new.
/// This function only reads standard environment variables; it does not elevate
/// privileges or write outside the current user's configuration directory.
pub fn default_user_state_path() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let root = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "APPDATA is not set"))?;

    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?
        .join("Library")
        .join("Application Support");

    #[cfg(all(unix, not(target_os = "macos")))]
    let root = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => path,
        _ => std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?
            .join(".config"),
    };

    Ok(root.join("PeaceEmu").join("module-state.v1"))
}

/// A malformed or unsupported persisted state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateFormatError {
    /// The header or format version is unsupported.
    UnsupportedFormat,
    /// A record is malformed or missing a required field.
    MalformedRecord,
    /// A module identifier is invalid.
    InvalidModuleId,
    /// A version string has invalid hexadecimal or UTF-8 encoding.
    InvalidVersion,
    /// A module occurs more than once in the same collection.
    DuplicateModule,
}

impl fmt::Display for StateFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat => f.write_str("unsupported module state format"),
            Self::MalformedRecord => f.write_str("malformed module state record"),
            Self::InvalidModuleId => f.write_str("invalid module ID in persisted state"),
            Self::InvalidVersion => f.write_str("invalid version encoding in persisted state"),
            Self::DuplicateModule => f.write_str("duplicate module in persisted state"),
        }
    }
}
impl std::error::Error for StateFormatError {}

/// An error encountered while loading or saving module state.
#[derive(Debug)]
pub enum ModuleStateStoreError {
    /// Filesystem operation failed.
    Io(io::Error),
    /// State file exceeds the size limit.
    TooLarge,
    /// Persisted data is invalid.
    Format(StateFormatError),
}
impl fmt::Display for ModuleStateStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "module state I/O error: {error}"),
            Self::TooLarge => f.write_str("module state file exceeds 4 MiB"),
            Self::Format(error) => write!(f, "invalid module state: {error}"),
        }
    }
}
impl std::error::Error for ModuleStateStoreError {}
impl From<io::Error> for ModuleStateStoreError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<StateFormatError> for ModuleStateStoreError {
    fn from(value: StateFormatError) -> Self {
        Self::Format(value)
    }
}

impl ModuleManagerState {
    /// Encodes state in deterministic version-1 UTF-8 records.
    ///
    /// Installed version strings are hexadecimal-encoded to prevent control
    /// characters from changing record boundaries.
    pub fn encode(&self) -> String {
        let mut output = format!(
            "{HEADER}\nstatus\t{}\n",
            match self.first_launch_status() {
                FirstLaunchStatus::NotStarted => "not-started",
                FirstLaunchStatus::Skipped => "skipped",
                FirstLaunchStatus::Completed => "completed",
            }
        );
        for id in self.selected_modules() {
            output.push_str(&format!("selected\t{}\n", id.as_str()));
        }
        for module in self.installed_modules() {
            output.push_str(&format!("installed\t{}\t", module.id.as_str()));
            for byte in module.version.as_bytes() {
                use core::fmt::Write as _;
                write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
            }
            output.push('\n');
        }
        output
    }

    /// Decodes version-1 state, rejecting duplicate or ambiguous records.
    pub fn decode(input: &str) -> Result<Self, StateFormatError> {
        let mut lines = input.lines();
        if lines.next() != Some(HEADER) {
            return Err(StateFormatError::UnsupportedFormat);
        }
        let mut status = None;
        let mut selected = Vec::new();
        let mut selected_ids = HashSet::new();
        let mut installed: Vec<InstalledModule> = Vec::new();
        let mut installed_ids = HashSet::new();
        for line in lines {
            if line.is_empty() {
                return Err(StateFormatError::MalformedRecord);
            }
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["status", value] if status.is_none() => {
                    status = Some(match *value {
                        "not-started" => FirstLaunchStatus::NotStarted,
                        "skipped" => FirstLaunchStatus::Skipped,
                        "completed" => FirstLaunchStatus::Completed,
                        _ => return Err(StateFormatError::MalformedRecord),
                    });
                }
                ["selected", raw_id] if status.is_some() => {
                    let id =
                        ModuleId::new(*raw_id).map_err(|_| StateFormatError::InvalidModuleId)?;
                    if !selected_ids.insert(id.clone()) {
                        return Err(StateFormatError::DuplicateModule);
                    }
                    selected.push(id);
                }
                ["installed", raw_id, raw_version] if status.is_some() => {
                    let id =
                        ModuleId::new(*raw_id).map_err(|_| StateFormatError::InvalidModuleId)?;
                    if !installed_ids.insert(id.clone()) {
                        return Err(StateFormatError::DuplicateModule);
                    }
                    if raw_version.len() % 2 != 0 {
                        return Err(StateFormatError::InvalidVersion);
                    }
                    let mut bytes = Vec::with_capacity(raw_version.len() / 2);
                    for pair in raw_version.as_bytes().chunks_exact(2) {
                        let high = nibble(pair[0]).ok_or(StateFormatError::InvalidVersion)?;
                        let low = nibble(pair[1]).ok_or(StateFormatError::InvalidVersion)?;
                        bytes.push((high << 4) | low);
                    }
                    let version =
                        String::from_utf8(bytes).map_err(|_| StateFormatError::InvalidVersion)?;
                    if version.trim().is_empty() {
                        return Err(StateFormatError::InvalidVersion);
                    }
                    installed.push(InstalledModule { id, version });
                }
                _ => return Err(StateFormatError::MalformedRecord),
            }
        }
        let status = status.ok_or(StateFormatError::MalformedRecord)?;
        let mut state = ModuleManagerState::new();
        state.set_selection(selected);
        match status {
            FirstLaunchStatus::NotStarted => {}
            FirstLaunchStatus::Skipped => state.mark_setup_skipped(),
            FirstLaunchStatus::Completed => state.mark_setup_completed(),
        }
        for module in installed {
            state
                .record_installed(module.id, module.version)
                .map_err(|_| StateFormatError::InvalidVersion)?;
        }
        Ok(state)
    }
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Disk-backed state store for a caller-selected application-data path.
///
/// Saves stage and flush a same-directory temporary file. A backup is retained
/// while replacing the primary state; callers must serialize concurrent writes.
pub struct ModuleStateStore {
    path: PathBuf,
}
impl ModuleStateStore {
    /// Creates a store at the exact path supplied by the application.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the configured state path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads persisted state, or returns a fresh profile when no file exists.
    pub fn load(&self) -> Result<ModuleManagerState, ModuleStateStoreError> {
        match read_state(&self.path) {
            Ok(state) => Ok(state),
            Err(primary) => {
                let backup = backup_path(&self.path);
                match read_state(&backup) {
                    Ok(state) => Ok(state),
                    Err(ModuleStateStoreError::Io(error))
                        if error.kind() == io::ErrorKind::NotFound
                            && matches!(
                                &primary,
                                ModuleStateStoreError::Io(primary_error)
                                    if primary_error.kind() == io::ErrorKind::NotFound
                            ) =>
                    {
                        Ok(ModuleManagerState::new())
                    }
                    Err(ModuleStateStoreError::Io(error))
                        if error.kind() == io::ErrorKind::NotFound =>
                    {
                        Err(primary)
                    }
                    Err(backup_error) => Err(backup_error),
                }
            }
        }
    }

    /// Saves state using a flushed temporary file and recoverable backup.
    pub fn save(&self, state: &ModuleManagerState) -> Result<(), ModuleStateStoreError> {
        self.save_with_directory_sync(state, sync_parent_directory)
    }

    fn save_with_directory_sync<F>(
        &self,
        state: &ModuleManagerState,
        sync_directory: F,
    ) -> Result<(), ModuleStateStoreError>
    where
        F: FnOnce(&Path) -> io::Result<()>,
    {
        self.save_with_operations(state, sync_directory, platform_rename)
    }

    fn save_with_operations<F, R>(
        &self,
        state: &ModuleManagerState,
        sync_directory: F,
        mut rename: R,
    ) -> Result<(), ModuleStateStoreError>
    where
        F: FnOnce(&Path) -> io::Result<()>,
        R: FnMut(&Path, &Path) -> io::Result<()>,
    {
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let encoded = state.encode();
        if encoded.len() as u64 > MAX_STATE_FILE_BYTES {
            return Err(ModuleStateStoreError::TooLarge);
        }
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
        let write_result = file
            .write_all(encoded.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }

        let had_original = self.path.exists();
        // If the primary is missing, the backup may be the only recoverable
        // copy. Keep it until the new primary has been installed successfully.
        if had_original && backup.exists() {
            if let Err(error) = fs::remove_file(&backup) {
                let _ = fs::remove_file(&temp);
                return Err(error.into());
            }
        }
        if let Err(error) =
            replace_primary_with_temp_using(&self.path, &temp, &backup, had_original, &mut rename)
        {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
        sync_directory(parent)?;
        Ok(())
    }
}

/// Installs a staged state file, restoring the previous primary if installation fails.
#[cfg(test)]
fn replace_primary_with_temp(
    primary: &Path,
    temp: &Path,
    backup: &Path,
    had_original: bool,
) -> io::Result<()> {
    replace_primary_with_temp_using(primary, temp, backup, had_original, platform_rename)
}

fn replace_primary_with_temp_using<F>(
    primary: &Path,
    temp: &Path,
    backup: &Path,
    had_original: bool,
    mut rename: F,
) -> io::Result<()>
where
    F: FnMut(&Path, &Path) -> io::Result<()>,
{
    if had_original {
        rename(primary, backup)?;
    }
    if let Err(install_error) = rename(temp, primary) {
        if had_original {
            // Preserve the installation error as the primary failure. If
            // restoration also fails, the backup is intentionally left intact
            // for recovery by ModuleStateStore::load.
            let _ = rename(backup, primary);
        }
        return Err(install_error);
    }
    Ok(())
}

/// Synchronizes directory metadata where the platform exposes directory sync.
/// Windows instead uses write-through MoveFileExW for every rename in the save
/// transaction; directory handles cannot be flushed with FlushFileBuffers.
#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> io::Result<()> {
    Ok(())
}

/// Uses a write-through rename on Windows and the standard rename elsewhere.
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

fn read_state(path: &Path) -> Result<ModuleManagerState, ModuleStateStoreError> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    if size > MAX_STATE_FILE_BYTES {
        return Err(ModuleStateStoreError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_STATE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_STATE_FILE_BYTES {
        return Err(ModuleStateStoreError::TooLarge);
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| ModuleStateStoreError::Format(StateFormatError::MalformedRecord))?;
    ModuleManagerState::decode(text).map_err(Into::into)
}
fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("bak")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_PATH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_path() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEST_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "host-app-module-state-{}-{unique}-{sequence}.state",
            std::process::id()
        ))
    }

    #[test]
    fn failed_restore_keeps_backup_available_for_recovery() {
        let primary = temp_path();
        let temp = primary.with_extension("missing-staged-file");
        let backup = backup_path(&primary);
        let mut original = ModuleManagerState::new();
        original.mark_setup_completed();
        fs::write(&primary, original.encode()).unwrap();

        let mut rename_calls = 0;
        let result = replace_primary_with_temp_using(&primary, &temp, &backup, true, |from, to| {
            rename_calls += 1;
            match rename_calls {
                1 => fs::rename(from, to),
                2 | 3 => Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected rename failure",
                )),
                _ => unreachable!("replacement must make at most three rename calls"),
            }
        });

        assert_eq!(rename_calls, 3);
        assert!(result.is_err());
        assert!(!primary.exists());
        assert_eq!(read_state(&backup).unwrap(), original);

        let store = ModuleStateStore::new(&primary);
        assert_eq!(store.load().unwrap(), original);
        let _ = fs::remove_file(&primary);
        let _ = fs::remove_file(&backup);
    }

    #[test]
    fn failed_temp_install_restores_original_primary() {
        let primary = temp_path();
        let temp = primary.with_extension("missing-staged-file");
        let backup = backup_path(&primary);
        let mut original = ModuleManagerState::new();
        original.mark_setup_completed();
        fs::write(&primary, original.encode()).unwrap();

        assert!(replace_primary_with_temp(&primary, &temp, &backup, true).is_err());
        assert_eq!(read_state(&primary).unwrap(), original);
        assert!(!backup.exists());

        let _ = fs::remove_file(&primary);
    }

    #[test]
    fn save_cleans_staged_file_and_restores_primary_when_install_rename_fails() {
        let path = temp_path();
        let store = ModuleStateStore::new(&path);
        let mut original = ModuleManagerState::new();
        original.mark_setup_completed();
        store.save(&original).unwrap();

        let mut replacement = ModuleManagerState::new();
        replacement.mark_setup_skipped();
        let mut rename_calls = 0;
        let result = store.save_with_operations(
            &replacement,
            |_| Ok(()),
            |from, to| {
                rename_calls += 1;
                match rename_calls {
                    1 => fs::rename(from, to),
                    2 => Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected staged-file install failure",
                    )),
                    3 => fs::rename(from, to),
                    _ => unreachable!("save should attempt at most three renames"),
                }
            },
        );

        assert!(matches!(result, Err(ModuleStateStoreError::Io(error))
            if error.kind() == io::ErrorKind::PermissionDenied));
        assert_eq!(rename_calls, 3);
        assert_eq!(store.load().unwrap(), original);
        let parent = path.parent().unwrap();
        let temp_prefix = format!(
            "{}.tmp-{}-",
            path.file_stem().unwrap().to_string_lossy(),
            std::process::id()
        );
        let leftovers = fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&temp_prefix)
            })
            .collect::<Vec<_>>();
        assert!(leftovers.is_empty(), "staged state file must be removed");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(backup_path(&path));
    }

    #[test]
    fn save_reports_directory_sync_failure_after_preserving_new_state_and_backup() {
        let path = temp_path();
        let store = ModuleStateStore::new(&path);
        let mut previous = ModuleManagerState::new();
        previous.mark_setup_completed();
        store.save(&previous).unwrap();

        let mut replacement = ModuleManagerState::new();
        replacement.mark_setup_skipped();
        let result = store.save_with_directory_sync(&replacement, |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected directory sync failure",
            ))
        });

        assert!(matches!(result, Err(ModuleStateStoreError::Io(error))
            if error.kind() == io::ErrorKind::PermissionDenied));
        assert_eq!(store.load().unwrap(), replacement);
        assert_eq!(read_state(&backup_path(&path)).unwrap(), previous);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(backup_path(&path));
    }

    #[cfg(unix)]
    #[test]
    fn directory_sync_failure_is_reported() {
        let missing_directory = temp_path().with_extension("missing-directory");
        assert!(sync_parent_directory(&missing_directory).is_err());
    }

    #[test]
    fn codec_round_trips_status_selection_and_control_characters_in_version() {
        let mut state = ModuleManagerState::new();
        state.set_selection([
            ModuleId::new("game-boy").unwrap(),
            ModuleId::new("nes").unwrap(),
        ]);
        state
            .record_installed(ModuleId::new("game-boy").unwrap(), "1.2\t0\nβ")
            .unwrap();
        state.mark_setup_completed();
        assert_eq!(ModuleManagerState::decode(&state.encode()).unwrap(), state);
    }

    #[test]
    fn codec_rejects_unknown_version_malformed_and_duplicate_records() {
        assert_eq!(
            ModuleManagerState::decode("PEACEEMU-MODULE-STATE\t2\nstatus\tnot-started\n"),
            Err(StateFormatError::UnsupportedFormat)
        );
        assert_eq!(
            ModuleManagerState::decode("PEACEEMU-MODULE-STATE\t1\nselected\tgb\n"),
            Err(StateFormatError::MalformedRecord)
        );
        assert_eq!(
            ModuleManagerState::decode(
                "PEACEEMU-MODULE-STATE\t1\nstatus\tnot-started\nselected\tgb\nselected\tgb\n"
            ),
            Err(StateFormatError::DuplicateModule)
        );
    }

    #[test]
    fn store_rejects_oversized_state_before_creating_or_replacing_files() {
        let path = temp_path();
        let store = ModuleStateStore::new(&path);
        let mut state = ModuleManagerState::new();
        state
            .record_installed(
                ModuleId::new("large-module").unwrap(),
                "v".repeat(MAX_STATE_FILE_BYTES as usize),
            )
            .unwrap();

        assert!(matches!(
            store.save(&state),
            Err(ModuleStateStoreError::TooLarge)
        ));
        assert!(!path.exists());
        assert!(!backup_path(&path).exists());
    }

    #[test]
    fn store_rejects_oversized_replacement_without_changing_existing_state() {
        let path = temp_path();
        let store = ModuleStateStore::new(&path);
        let mut existing = ModuleManagerState::new();
        existing.mark_setup_completed();
        store.save(&existing).unwrap();

        let mut oversized = ModuleManagerState::new();
        oversized
            .record_installed(
                ModuleId::new("large-module").unwrap(),
                "v".repeat(MAX_STATE_FILE_BYTES as usize),
            )
            .unwrap();

        assert!(matches!(
            store.save(&oversized),
            Err(ModuleStateStoreError::TooLarge)
        ));
        assert_eq!(store.load().unwrap(), existing);
        assert!(!backup_path(&path).exists());

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(backup_path(&path));
    }

    #[test]
    fn store_rejects_oversized_persisted_file() {
        let path = temp_path();
        fs::write(&path, vec![b'x'; MAX_STATE_FILE_BYTES as usize + 1]).unwrap();
        let store = ModuleStateStore::new(&path);
        assert!(matches!(store.load(), Err(ModuleStateStoreError::TooLarge)));
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(backup_path(&path));
    }

    #[test]
    fn store_cleans_temporary_file_when_backup_removal_fails() {
        let path = temp_path();
        let store = ModuleStateStore::new(&path);
        let mut existing = ModuleManagerState::new();
        existing.mark_setup_completed();
        store.save(&existing).unwrap();

        let backup = backup_path(&path);
        fs::create_dir(&backup).unwrap();

        assert!(store.save(&ModuleManagerState::new()).is_err());
        assert_eq!(read_state(&path).unwrap(), existing);

        let prefix = format!("{}.tmp-", path.file_stem().unwrap().to_string_lossy());
        let leftovers = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .count();
        assert_eq!(leftovers, 0, "failed save must remove its temporary file");

        fs::remove_dir(&backup).unwrap();
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(backup_path(&path));
    }

    #[test]
    fn store_preserves_only_backup_when_primary_is_missing() {
        let path = temp_path();
        let backup = backup_path(&path);
        let store = ModuleStateStore::new(&path);
        let mut recoverable = ModuleManagerState::new();
        recoverable.mark_setup_skipped();
        fs::write(&backup, recoverable.encode()).unwrap();

        let mut replacement = ModuleManagerState::new();
        replacement.mark_setup_completed();
        store.save(&replacement).unwrap();

        assert_eq!(store.load().unwrap(), replacement);
        assert!(backup.exists());
        assert_eq!(read_state(&backup).unwrap(), recoverable);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&backup);
    }

    #[test]
    fn store_recovers_valid_backup_when_primary_exceeds_size_limit() {
        let path = temp_path();
        let backup = backup_path(&path);
        let store = ModuleStateStore::new(&path);
        let mut recoverable = ModuleManagerState::new();
        recoverable.mark_setup_skipped();
        fs::write(&backup, recoverable.encode()).unwrap();
        fs::write(&path, vec![b'x'; MAX_STATE_FILE_BYTES as usize + 1]).unwrap();

        assert_eq!(store.load().unwrap(), recoverable);
        assert!(
            path.exists(),
            "recovery must not silently delete the oversized primary"
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_STATE_FILE_BYTES + 1);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&backup);
    }

    #[test]
    fn store_does_not_treat_corrupt_primary_without_backup_as_first_launch() {
        let path = temp_path();
        let store = ModuleStateStore::new(&path);
        fs::write(&path, b"corrupt").unwrap();

        assert!(
            store.load().is_err(),
            "a corrupt primary without a backup must not be treated as first launch"
        );

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(backup_path(&path));
    }

    #[test]
    fn store_persists_and_recovers_backup_after_corruption() {
        let path = temp_path();
        let store = ModuleStateStore::new(&path);
        assert!(store.load().unwrap().should_show_first_launch());
        let mut first = ModuleManagerState::new();
        first.mark_setup_skipped();
        store.save(&first).unwrap();
        let mut second = ModuleManagerState::new();
        second.mark_setup_completed();
        store.save(&second).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(store.load().unwrap(), first);
        fs::write(&path, b"corrupt").unwrap();
        assert_eq!(store.load().unwrap(), first);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(backup_path(&path));
    }
}
