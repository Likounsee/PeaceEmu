#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Host-driven installation transaction orchestration.
//!
//! This module connects the durable journal to host-provided package operations.
//! It deliberately does not implement networking, archive formats, signatures,
//! or platform-specific activation. The backend must enforce those boundaries.

use core::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::module_install_journal::{
    plan_install_recovery, remove_committed_install_journal, remove_discarded_install_journal,
    InstallJournal, InstallJournalError, InstallPhase, InstallRecovery, InstallRecoveryPlan,
};
use crate::module_management::ModuleId;

/// Operations supplied by the trusted host installer.
///
/// Implementations should make each operation idempotent where practical.
/// In particular, activation must be atomic or have its own recovery procedure.
pub trait ModuleInstallBackend {
    /// Downloads and verifies the package bytes for the exact module version.
    fn download_and_verify(&mut self, module: &ModuleId, version: &str) -> Result<(), String>;

    /// Extracts the verified package into an isolated staging location.
    fn extract_to_staging(&mut self, module: &ModuleId, version: &str) -> Result<(), String>;

    /// Activates the staged package at its final location.
    fn activate_staged(&mut self, module: &ModuleId, version: &str) -> Result<(), String>;

    /// Persists the installed module record after activation succeeded.
    fn persist_installed_state(&mut self, module: &ModuleId, version: &str) -> Result<(), String>;

    /// Removes temporary files for a transaction that has not committed.
    ///
    /// This must not delete or overwrite a previously installed working version.
    fn cleanup_uncommitted(&mut self, module: &ModuleId, version: &str) -> Result<(), String>;
}

/// Host operations required to recover interrupted module installations.
pub trait ModuleInstallRecoveryBackend {
    /// Removes isolated transaction staging and temporary artifact files.
    ///
    /// Must not modify the currently installed module version.
    fn cleanup_transaction_files(&mut self, module: &ModuleId, version: &str)
        -> Result<(), String>;

    /// Confirms that the requested version is active and persists that fact in installed state.
    ///
    /// This must inspect the final installation and be idempotent. It must return an error
    /// rather than guess when the final state is ambiguous.
    fn reconcile_activated_state(&mut self, module: &ModuleId, version: &str)
        -> Result<(), String>;
}

/// Result for one journal handled during startup recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstallRecoveryResult {
    /// Pre-activation staging was cleaned and its journal removed.
    Discarded {
        /// Path of the removed journal.
        journal_path: PathBuf,
    },
    /// An already committed transaction's temporary files and journal were removed.
    CleanedCommitted {
        /// Path of the removed journal.
        journal_path: PathBuf,
    },
    /// Activation was reconciled, committed durably, and its journal removed.
    ReconciledCommitted {
        /// Path of the removed journal.
        journal_path: PathBuf,
    },
    /// A failed transaction was preserved for explicit inspection or retry.
    NeedsInspection {
        /// Path of the journal retained for recovery.
        journal_path: PathBuf,
        /// Transaction phase recorded when recovery stopped.
        phase: InstallPhase,
    },
}

/// Startup recovery failure. The journal is retained whenever recovery cannot finish.
#[derive(Debug)]
pub struct InstallRecoveryError {
    /// Journal that could not be recovered.
    pub journal_path: PathBuf,
    /// Operation that failed.
    pub operation: &'static str,
    /// Backend or persistence error details.
    pub message: String,
}

impl fmt::Display for InstallRecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "module installation recovery failed during {} for {}: {}",
            self.operation,
            self.journal_path.display(),
            self.message
        )
    }
}

impl std::error::Error for InstallRecoveryError {}

/// Plans and executes recovery for every journal in a transaction directory.
///
/// No journal is removed until the backend has successfully cleaned transaction files or
/// reconciled the final installed version. Ambiguous activation and failed transactions
/// remain on disk for inspection. A malformed journal aborts planning before any action.
pub fn recover_module_installs<B: ModuleInstallRecoveryBackend>(
    backend: &mut B,
    directory: impl AsRef<Path>,
) -> Result<Vec<InstallRecoveryResult>, InstallRecoveryError> {
    let plans = plan_install_recovery(directory).map_err(|error| InstallRecoveryError {
        journal_path: PathBuf::from("<journal directory>"),
        operation: "planning",
        message: error.to_string(),
    })?;
    let mut results = Vec::with_capacity(plans.len());

    for plan in plans {
        let result = recover_one_module_install(backend, &plan)?;
        results.push(result);
    }
    Ok(results)
}

fn recover_one_module_install<B: ModuleInstallRecoveryBackend>(
    backend: &mut B,
    plan: &InstallRecoveryPlan,
) -> Result<InstallRecoveryResult, InstallRecoveryError> {
    let fail = |operation, message: String| InstallRecoveryError {
        journal_path: plan.journal_path.clone(),
        operation,
        message,
    };
    let current = InstallJournal::load(&plan.journal_path)
        .map_err(|error| fail("journal validation", error.to_string()))?;
    if current.entry().module != plan.module
        || current.entry().version != plan.version
        || current.entry().phase != plan.phase
    {
        return Err(fail(
            "journal validation",
            "journal contents changed after recovery planning".to_owned(),
        ));
    }

    match plan.action {
        InstallRecovery::DiscardStaging => {
            backend
                .cleanup_transaction_files(&plan.module, &plan.version)
                .map_err(|message| fail("staging cleanup", message))?;
            remove_discarded_install_journal(&plan.journal_path)
                .map_err(|error| fail("journal removal", error.to_string()))?;
            Ok(InstallRecoveryResult::Discarded {
                journal_path: plan.journal_path.clone(),
            })
        }
        InstallRecovery::ReconcileActivation => {
            backend
                .reconcile_activated_state(&plan.module, &plan.version)
                .map_err(|message| fail("activation reconciliation", message))?;
            let mut journal = InstallJournal::load(&plan.journal_path)
                .map_err(|error| fail("journal reload", error.to_string()))?;
            if journal.entry().module != plan.module
                || journal.entry().version != plan.version
                || journal.entry().phase != plan.phase
            {
                return Err(fail(
                    "journal validation",
                    "journal contents changed during activation reconciliation".to_owned(),
                ));
            }
            if journal.entry().phase == InstallPhase::Activating {
                journal
                    .advance(InstallPhase::Activated)
                    .map_err(|error| fail("activation milestone", error.to_string()))?;
                journal
                    .save(&plan.journal_path)
                    .map_err(|error| fail("activation milestone persistence", error.to_string()))?;
            }
            if journal.entry().phase != InstallPhase::Activated {
                return Err(fail(
                    "activation reconciliation",
                    format!(
                        "journal phase changed unexpectedly to {:?}",
                        journal.entry().phase
                    ),
                ));
            }
            journal
                .advance(InstallPhase::Committed)
                .map_err(|error| fail("commit milestone", error.to_string()))?;
            journal
                .save(&plan.journal_path)
                .map_err(|error| fail("commit persistence", error.to_string()))?;
            backend
                .cleanup_transaction_files(&plan.module, &plan.version)
                .map_err(|message| fail("committed cleanup", message))?;
            remove_committed_install_journal(&plan.journal_path)
                .map_err(|error| fail("committed journal removal", error.to_string()))?;
            Ok(InstallRecoveryResult::ReconciledCommitted {
                journal_path: plan.journal_path.clone(),
            })
        }
        InstallRecovery::CleanupCommitted => {
            backend
                .cleanup_transaction_files(&plan.module, &plan.version)
                .map_err(|message| fail("committed cleanup", message))?;
            remove_committed_install_journal(&plan.journal_path)
                .map_err(|error| fail("committed journal removal", error.to_string()))?;
            Ok(InstallRecoveryResult::CleanedCommitted {
                journal_path: plan.journal_path.clone(),
            })
        }
        InstallRecovery::InspectFailure => Ok(InstallRecoveryResult::NeedsInspection {
            journal_path: plan.journal_path.clone(),
            phase: plan.phase,
        }),
    }
}

/// Last operation attempted by an installation coordinator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallOperation {
    /// Initial journal creation or persistence.
    Journal,
    /// Download and artifact verification.
    Download,
    /// Package extraction into staging.
    Extraction,
    /// Activation of staged package.
    Activation,
    /// Persistence of installed state.
    StatePersistence,
    /// Removal of temporary transaction files.
    Cleanup,
}

/// Error returned by the installation coordinator.
#[derive(Debug)]
pub enum ModuleInstallError {
    /// The durable journal could not be read or written.
    Journal(InstallJournalError),
    /// An operation failed before activation; cleanup was attempted.
    OperationFailed {
        /// Operation that failed.
        operation: InstallOperation,
        /// Module ID.
        module: ModuleId,
        /// Host-provided failure details.
        message: String,
        /// Additional cleanup failure, if any.
        cleanup_error: Option<String>,
    },
    /// Activation completed or may have completed, so automatic cleanup is unsafe.
    RecoveryRequired {
        /// Operation that failed after activation began.
        operation: InstallOperation,
        /// Module ID.
        module: ModuleId,
        /// Host-provided failure details.
        message: String,
        /// Journal path that must be reconciled on startup.
        journal_path: PathBuf,
    },
}

impl fmt::Display for ModuleInstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal(error) => write!(f, "installation journal failed: {error}"),
            Self::OperationFailed { operation, module, message, cleanup_error } => {
                write!(f, "module '{module}' failed during {operation:?}: {message}")?;
                if let Some(cleanup) = cleanup_error {
                    write!(f, "; cleanup also failed: {cleanup}")?;
                }
                Ok(())
            }
            Self::RecoveryRequired { operation, module, message, journal_path } => write!(
                f,
                "module '{module}' failed during {operation:?}: {message}; reconcile journal at {} before retrying",
                journal_path.display()
            ),
        }
    }
}

impl std::error::Error for ModuleInstallError {}

impl From<InstallJournalError> for ModuleInstallError {
    fn from(error: InstallJournalError) -> Self {
        Self::Journal(error)
    }
}

/// Installs one already-planned module version through journaled milestones.
///
/// The journal is saved before the first backend operation and after every phase
/// transition. A failure before activation is marked failed and cleaned up.
/// A failure after activation leaves the journal at its last durable milestone
/// so startup recovery can reconcile the installed directory with persisted state.
pub fn install_module<B: ModuleInstallBackend>(
    backend: &mut B,
    journal_path: impl AsRef<Path>,
    module: ModuleId,
    version: impl Into<String>,
) -> Result<(), ModuleInstallError> {
    let journal_path = journal_path.as_ref();
    let version = version.into();
    let mut journal = InstallJournal::new(module.clone(), version.clone())?;
    journal.save(journal_path)?;

    advance_and_save(&mut journal, InstallPhase::Downloading, journal_path)?;
    if let Err(message) = backend.download_and_verify(&module, &version) {
        return Err(fail_before_activation(
            backend,
            &mut journal,
            journal_path,
            InstallOperation::Download,
            message,
        ));
    }

    advance_and_save(&mut journal, InstallPhase::Verified, journal_path)?;
    if let Err(message) = backend.extract_to_staging(&module, &version) {
        return Err(fail_before_activation(
            backend,
            &mut journal,
            journal_path,
            InstallOperation::Extraction,
            message,
        ));
    }

    advance_and_save(&mut journal, InstallPhase::Staged, journal_path)?;
    // Persist intent before touching the final location. If the process stops
    // mid-activation, recovery knows to inspect rather than blindly discard staging.
    advance_and_save(&mut journal, InstallPhase::Activating, journal_path)?;
    if let Err(message) = backend.activate_staged(&module, &version) {
        // Activation may have partially changed the final location. Preserve the
        // journal as Failed and require explicit recovery rather than deleting data.
        let _ = journal.advance(InstallPhase::Failed);
        let _ = journal.save(journal_path);
        return Err(ModuleInstallError::RecoveryRequired {
            operation: InstallOperation::Activation,
            module,
            message,
            journal_path: journal_path.to_path_buf(),
        });
    }

    // From this point onward, the final installation may already exist. A journal
    // write failure must therefore request recovery instead of looking like an
    // ordinary pre-install error that callers might safely retry.
    advance_and_save_after_activation(
        &mut journal,
        InstallPhase::Activated,
        journal_path,
        InstallOperation::Journal,
    )?;
    if let Err(message) = backend.persist_installed_state(&module, &version) {
        // Do not roll back or delete files: activation succeeded, and state may be
        // partially persisted. Startup must reconcile the durable Activated journal.
        return Err(ModuleInstallError::RecoveryRequired {
            operation: InstallOperation::StatePersistence,
            module,
            message,
            journal_path: journal_path.to_path_buf(),
        });
    }

    advance_and_save_after_activation(
        &mut journal,
        InstallPhase::Committed,
        journal_path,
        InstallOperation::Journal,
    )?;
    Ok(())
}

fn advance_and_save(
    journal: &mut InstallJournal,
    phase: InstallPhase,
    path: &Path,
) -> Result<(), ModuleInstallError> {
    journal.advance(phase)?;
    journal.save(path)?;
    Ok(())
}

/// Persists a journal milestone after activation has begun.
fn advance_and_save_after_activation(
    journal: &mut InstallJournal,
    phase: InstallPhase,
    path: &Path,
    operation: InstallOperation,
) -> Result<(), ModuleInstallError> {
    if let Err(error) = journal.advance(phase).and_then(|()| journal.save(path)) {
        return Err(ModuleInstallError::RecoveryRequired {
            operation,
            module: journal.entry().module.clone(),
            message: format!("could not persist {phase:?} journal milestone: {error}"),
            journal_path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn fail_before_activation<B: ModuleInstallBackend>(
    backend: &mut B,
    journal: &mut InstallJournal,
    journal_path: &Path,
    operation: InstallOperation,
    message: String,
) -> ModuleInstallError {
    let _ = journal.advance(InstallPhase::Failed);
    let journal_error = journal.save(journal_path).err();
    let cleanup_error = backend
        .cleanup_uncommitted(&journal.entry().module, &journal.entry().version)
        .err();

    if let Some(error) = journal_error {
        return ModuleInstallError::RecoveryRequired {
            operation: InstallOperation::Journal,
            module: journal.entry().module.clone(),
            message: format!("{message}; failed to persist failure state: {error}"),
            journal_path: journal_path.to_path_buf(),
        };
    }

    ModuleInstallError::OperationFailed {
        operation,
        module: journal.entry().module.clone(),
        message,
        cleanup_error,
    }
}

/// Returns the currently recorded phase from a journal file.
pub fn read_install_phase(path: impl AsRef<Path>) -> Result<InstallPhase, ModuleInstallError> {
    Ok(InstallJournal::load(path)?.entry().phase)
}

/// Convenience conversion for callers that want to report journal I/O failures
/// without depending on this module's error enum.
pub fn journal_io_error(error: io::Error) -> ModuleInstallError {
    ModuleInstallError::Journal(InstallJournalError::Io(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestBackend {
        fail_at: Option<InstallOperation>,
        fail_cleanup: bool,
        calls: Vec<InstallOperation>,
    }

    impl TestBackend {
        fn new(fail_at: Option<InstallOperation>) -> Self {
            Self {
                fail_at,
                fail_cleanup: false,
                calls: Vec::new(),
            }
        }

        fn failing_download_and_cleanup() -> Self {
            Self {
                fail_at: Some(InstallOperation::Download),
                fail_cleanup: true,
                calls: Vec::new(),
            }
        }

        fn step(&mut self, operation: InstallOperation) -> Result<(), String> {
            self.calls.push(operation);
            if self.fail_at == Some(operation)
                || (self.fail_cleanup && operation == InstallOperation::Cleanup)
            {
                Err(format!("injected {operation:?} failure"))
            } else {
                Ok(())
            }
        }
    }

    impl ModuleInstallBackend for TestBackend {
        fn download_and_verify(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
            self.step(InstallOperation::Download)
        }

        fn extract_to_staging(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
            self.step(InstallOperation::Extraction)
        }

        fn activate_staged(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
            self.step(InstallOperation::Activation)
        }

        fn persist_installed_state(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
            self.step(InstallOperation::StatePersistence)
        }

        fn cleanup_uncommitted(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
            self.step(InstallOperation::Cleanup)
        }
    }

    fn journal_path() -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "host-app-install-test-{}-{sequence}.journal",
            std::process::id()
        ))
    }

    struct RecoveryTestBackend {
        fail_reconcile: bool,
        calls: Vec<&'static str>,
    }

    impl ModuleInstallRecoveryBackend for RecoveryTestBackend {
        fn cleanup_transaction_files(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
            self.calls.push("cleanup");
            Ok(())
        }

        fn reconcile_activated_state(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
            self.calls.push("reconcile");
            if self.fail_reconcile {
                Err("ambiguous final state".to_owned())
            } else {
                Ok(())
            }
        }
    }

    fn recovery_directory() -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "host-app-recovery-test-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn save_journal_at(directory: &Path, name: &str, phases: &[InstallPhase]) -> PathBuf {
        let path = directory.join(name);
        let mut journal =
            InstallJournal::new(ModuleId::new("test-module").unwrap(), "1.0.0").unwrap();
        for phase in phases {
            journal.advance(*phase).unwrap();
        }
        journal.save(&path).unwrap();
        path
    }

    #[test]
    fn startup_recovery_cleans_pre_activation_and_preserves_failed_journals() {
        let directory = recovery_directory();
        let discard_path = save_journal_at(
            &directory,
            "discard.journal",
            &[
                InstallPhase::Downloading,
                InstallPhase::Verified,
                InstallPhase::Staged,
            ],
        );
        let failed_path = save_journal_at(
            &directory,
            "failed.journal",
            &[
                InstallPhase::Downloading,
                InstallPhase::Verified,
                InstallPhase::Staged,
                InstallPhase::Activating,
                InstallPhase::Failed,
            ],
        );
        let mut backend = RecoveryTestBackend {
            fail_reconcile: false,
            calls: Vec::new(),
        };

        let result = recover_module_installs(&mut backend, &directory).unwrap();

        assert_eq!(result.len(), 2);
        assert!(!discard_path.exists());
        assert!(failed_path.exists());
        assert!(result.iter().any(|item| matches!(
            item,
            InstallRecoveryResult::NeedsInspection {
                phase: InstallPhase::Failed,
                ..
            }
        )));
        assert_eq!(backend.calls, vec!["cleanup"]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn startup_recovery_commits_only_after_backend_reconciliation_succeeds() {
        let directory = recovery_directory();
        let path = save_journal_at(
            &directory,
            "activating.journal",
            &[
                InstallPhase::Downloading,
                InstallPhase::Verified,
                InstallPhase::Staged,
                InstallPhase::Activating,
            ],
        );
        let mut failing_backend = RecoveryTestBackend {
            fail_reconcile: true,
            calls: Vec::new(),
        };

        let error = recover_module_installs(&mut failing_backend, &directory).unwrap_err();
        assert_eq!(error.operation, "activation reconciliation");
        assert!(path.exists());
        assert_eq!(
            InstallJournal::load(&path).unwrap().entry().phase,
            InstallPhase::Activating
        );
        assert_eq!(failing_backend.calls, vec!["reconcile"]);

        let mut backend = RecoveryTestBackend {
            fail_reconcile: false,
            calls: Vec::new(),
        };
        let result = recover_module_installs(&mut backend, &directory).unwrap();
        assert!(matches!(
            result.as_slice(),
            [InstallRecoveryResult::ReconciledCommitted { .. }]
        ));
        assert!(!path.exists());
        assert_eq!(backend.calls, vec!["reconcile", "cleanup"]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn successful_install_commits_journal_after_all_backend_steps() {
        let path = journal_path();
        let mut backend = TestBackend::new(None);

        install_module(
            &mut backend,
            &path,
            ModuleId::new("test-module").unwrap(),
            "1.0.0",
        )
        .unwrap();

        assert_eq!(read_install_phase(&path).unwrap(), InstallPhase::Committed);
        assert_eq!(
            backend.calls,
            vec![
                InstallOperation::Download,
                InstallOperation::Extraction,
                InstallOperation::Activation,
                InstallOperation::StatePersistence,
            ]
        );
        let journal = InstallJournal::load(&path).unwrap();
        assert_eq!(journal.entry().module.as_str(), "test-module");
        assert_eq!(journal.entry().version, "1.0.0");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_package_version_is_rejected_before_journal_or_backend_work() {
        let path = journal_path();
        let mut backend = TestBackend::new(None);

        let result = install_module(
            &mut backend,
            &path,
            ModuleId::new("test-module").unwrap(),
            "1.0.0\nphase\tcommitted",
        );

        assert!(matches!(
            result,
            Err(ModuleInstallError::Journal(
                InstallJournalError::InvalidVersion
            ))
        ));
        assert!(backend.calls.is_empty());
        assert!(!path.exists());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn initial_journal_failure_prevents_all_backend_operations() {
        let blocker = journal_path();
        fs::write(&blocker, b"not a directory").unwrap();
        let path = blocker.join("install.journal");
        let mut backend = TestBackend::new(None);

        let result = install_module(
            &mut backend,
            &path,
            ModuleId::new("test-module").unwrap(),
            "1.0.0",
        );

        assert!(matches!(result, Err(ModuleInstallError::Journal(_))));
        assert!(backend.calls.is_empty());
        let _ = fs::remove_file(blocker);
    }

    #[test]
    fn journal_write_failure_after_activation_requires_recovery() {
        struct ReplaceJournalBackend {
            journal_path: PathBuf,
            calls: Vec<InstallOperation>,
        }

        impl ModuleInstallBackend for ReplaceJournalBackend {
            fn download_and_verify(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
                self.calls.push(InstallOperation::Download);
                Ok(())
            }

            fn extract_to_staging(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
                self.calls.push(InstallOperation::Extraction);
                Ok(())
            }

            fn activate_staged(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
                self.calls.push(InstallOperation::Activation);
                fs::remove_file(&self.journal_path).map_err(|error| error.to_string())?;
                fs::create_dir(&self.journal_path).map_err(|error| error.to_string())?;
                Ok(())
            }

            fn persist_installed_state(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
                self.calls.push(InstallOperation::StatePersistence);
                Ok(())
            }

            fn cleanup_uncommitted(&mut self, _: &ModuleId, _: &str) -> Result<(), String> {
                self.calls.push(InstallOperation::Cleanup);
                Ok(())
            }
        }

        let path = journal_path();
        let mut backend = ReplaceJournalBackend {
            journal_path: path.clone(),
            calls: Vec::new(),
        };
        let result = install_module(
            &mut backend,
            &path,
            ModuleId::new("test-module").unwrap(),
            "1.0.0",
        );

        assert!(matches!(
            result,
            Err(ModuleInstallError::RecoveryRequired {
                operation: InstallOperation::Journal,
                ..
            })
        ));
        assert_eq!(
            backend.calls,
            vec![
                InstallOperation::Download,
                InstallOperation::Extraction,
                InstallOperation::Activation,
            ]
        );
        assert!(
            path.is_dir(),
            "fault injection should block journal replacement"
        );
        fs::remove_dir(&path).unwrap();
    }

    #[test]
    fn state_persistence_failure_keeps_activated_journal_for_recovery() {
        let path = journal_path();
        let mut backend = TestBackend::new(Some(InstallOperation::StatePersistence));
        let result = install_module(
            &mut backend,
            &path,
            ModuleId::new("test-module").unwrap(),
            "1.0.0",
        );

        assert!(matches!(
            result,
            Err(ModuleInstallError::RecoveryRequired {
                operation: InstallOperation::StatePersistence,
                ..
            })
        ));
        assert_eq!(read_install_phase(&path).unwrap(), InstallPhase::Activated);
        assert!(!backend.calls.contains(&InstallOperation::Cleanup));
        assert_eq!(
            backend.calls,
            vec![
                InstallOperation::Download,
                InstallOperation::Extraction,
                InstallOperation::Activation,
                InstallOperation::StatePersistence,
            ]
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn activation_failure_requires_recovery_without_cleanup() {
        let path = journal_path();
        let mut backend = TestBackend::new(Some(InstallOperation::Activation));
        let result = install_module(
            &mut backend,
            &path,
            ModuleId::new("test-module").unwrap(),
            "1.0.0",
        );

        assert!(matches!(
            result,
            Err(ModuleInstallError::RecoveryRequired {
                operation: InstallOperation::Activation,
                ..
            })
        ));
        assert_eq!(read_install_phase(&path).unwrap(), InstallPhase::Failed);
        assert!(!backend.calls.contains(&InstallOperation::Cleanup));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn cleanup_failure_is_reported_alongside_original_operation_failure() {
        let path = journal_path();
        let mut backend = TestBackend::failing_download_and_cleanup();
        let result = install_module(
            &mut backend,
            &path,
            ModuleId::new("test-module").unwrap(),
            "1.0.0",
        );

        match result {
            Err(ModuleInstallError::OperationFailed {
                operation: InstallOperation::Download,
                cleanup_error: Some(message),
                ..
            }) => assert!(message.contains("Cleanup failure")),
            other => panic!("expected download failure with cleanup error, got {other:?}"),
        }
        assert_eq!(read_install_phase(&path).unwrap(), InstallPhase::Failed);
        assert!(backend.calls.contains(&InstallOperation::Cleanup));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn extraction_failure_marks_journal_failed_and_cleans_staging_without_activation() {
        let path = journal_path();
        let mut backend = TestBackend::new(Some(InstallOperation::Extraction));
        let result = install_module(
            &mut backend,
            &path,
            ModuleId::new("test-module").unwrap(),
            "1.0.0",
        );

        assert!(matches!(
            result,
            Err(ModuleInstallError::OperationFailed {
                operation: InstallOperation::Extraction,
                cleanup_error: None,
                ..
            })
        ));
        assert_eq!(read_install_phase(&path).unwrap(), InstallPhase::Failed);
        assert_eq!(
            backend.calls,
            vec![
                InstallOperation::Download,
                InstallOperation::Extraction,
                InstallOperation::Cleanup,
            ]
        );
        assert!(!backend.calls.contains(&InstallOperation::Activation));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn download_failure_marks_journal_failed_and_attempts_cleanup() {
        let path = journal_path();
        let mut backend = TestBackend::new(Some(InstallOperation::Download));
        let result = install_module(
            &mut backend,
            &path,
            ModuleId::new("test-module").unwrap(),
            "1.0.0",
        );

        assert!(matches!(
            result,
            Err(ModuleInstallError::OperationFailed {
                operation: InstallOperation::Download,
                cleanup_error: None,
                ..
            })
        ));
        assert_eq!(read_install_phase(&path).unwrap(), InstallPhase::Failed);
        assert!(backend.calls.contains(&InstallOperation::Cleanup));

        let _ = fs::remove_file(path);
    }
}
