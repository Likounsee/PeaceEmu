#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Persistence-agnostic user state for first-launch setup and installed modules.
//!
//! The application layer is responsible for storing this model transactionally.
//! Updating this in-memory state must happen only after a package has been
//! verified and atomically activated on disk.

use core::fmt;

use crate::module_management::ModuleId;

/// Whether the first-launch wizard has been deliberately completed or skipped.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FirstLaunchStatus {
    /// No deliberate setup decision has been recorded yet.
    #[default]
    NotStarted,
    /// The user explicitly chose to skip setup and configure modules later.
    Skipped,
    /// The user completed the selection/review flow.
    Completed,
}

/// A module package version recorded as successfully installed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledModule {
    /// Stable package ID.
    pub id: ModuleId,
    /// Exact package version activated on disk.
    pub version: String,
}

/// In-memory user choices and successfully installed module versions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModuleManagerState {
    first_launch_status: FirstLaunchStatus,
    selected_modules: Vec<ModuleId>,
    installed_modules: Vec<InstalledModule>,
}

/// Error while updating persisted module state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModuleStateError {
    /// An installed package must have a non-empty version string.
    EmptyVersion,
    /// A batch contains the same module more than once.
    DuplicateModuleInBatch(ModuleId),
}

impl fmt::Display for ModuleStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyVersion => f.write_str("installed module version cannot be empty"),
            Self::DuplicateModuleInBatch(id) => {
                write!(f, "module '{id}' occurs more than once in the state update")
            }
        }
    }
}

impl std::error::Error for ModuleStateError {}

impl ModuleManagerState {
    /// Creates a state for a new user profile.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether the first-launch wizard should be offered.
    pub fn should_show_first_launch(&self) -> bool {
        self.first_launch_status == FirstLaunchStatus::NotStarted
    }

    /// Returns the recorded first-launch decision.
    pub fn first_launch_status(&self) -> FirstLaunchStatus {
        self.first_launch_status
    }

    /// Returns the currently selected module IDs in user-selected order.
    pub fn selected_modules(&self) -> &[ModuleId] {
        &self.selected_modules
    }

    /// Returns successfully installed modules in activation order.
    pub fn installed_modules(&self) -> &[InstalledModule] {
        &self.installed_modules
    }

    /// Replaces the selected modules, preserving first occurrence and order.
    ///
    /// This records user intent only. The catalogue and dependency planner must
    /// validate the selection before the application starts installation.
    pub fn set_selection(&mut self, selected: impl IntoIterator<Item = ModuleId>) {
        self.selected_modules.clear();
        for id in selected {
            if !self.selected_modules.contains(&id) {
                self.selected_modules.push(id);
            }
        }
    }

    /// Records that the user explicitly skipped first-launch setup.
    pub fn mark_setup_skipped(&mut self) {
        self.first_launch_status = FirstLaunchStatus::Skipped;
    }

    /// Records that the user completed the setup selection/review flow.
    ///
    /// This does not mean every requested package installed successfully.
    pub fn mark_setup_completed(&mut self) {
        self.first_launch_status = FirstLaunchStatus::Completed;
    }

    /// Returns whether a package is recorded as installed.
    pub fn is_installed(&self, id: &ModuleId) -> bool {
        self.installed_modules.iter().any(|module| &module.id == id)
    }

    /// Returns the exact installed version, if the package is recorded.
    pub fn installed_version(&self, id: &ModuleId) -> Option<&str> {
        self.installed_modules
            .iter()
            .find(|module| &module.id == id)
            .map(|module| module.version.as_str())
    }

    /// Records a verified and activated package version.
    ///
    /// An existing record for the same module is updated in place, preserving
    /// the module's order in the installed list.
    pub fn record_installed(
        &mut self,
        id: ModuleId,
        version: impl Into<String>,
    ) -> Result<(), ModuleStateError> {
        let version = version.into();
        if version.trim().is_empty() {
            return Err(ModuleStateError::EmptyVersion);
        }
        if let Some(existing) = self
            .installed_modules
            .iter_mut()
            .find(|module| module.id == id)
        {
            existing.version = version;
        } else {
            self.installed_modules.push(InstalledModule { id, version });
        }
        Ok(())
    }

    /// Records several verified and activated package versions as one in-memory operation.
    ///
    /// All input is validated before the state changes: an empty version or duplicate
    /// module ID leaves the complete installed-module list untouched. Existing records
    /// retain their position; new records are appended in input order. The caller must
    /// activate every package on disk before invoking this method.
    pub fn record_installed_batch(
        &mut self,
        packages: impl IntoIterator<Item = (ModuleId, String)>,
    ) -> Result<(), ModuleStateError> {
        let packages: Vec<(ModuleId, String)> = packages.into_iter().collect();
        let mut seen = std::collections::HashSet::with_capacity(packages.len());
        for (id, version) in &packages {
            if version.trim().is_empty() {
                return Err(ModuleStateError::EmptyVersion);
            }
            if !seen.insert(id.clone()) {
                return Err(ModuleStateError::DuplicateModuleInBatch(id.clone()));
            }
        }

        for (id, version) in packages {
            if let Some(existing) = self
                .installed_modules
                .iter_mut()
                .find(|module| module.id == id)
            {
                existing.version = version;
            } else {
                self.installed_modules.push(InstalledModule { id, version });
            }
        }
        Ok(())
    }

    /// Removes multiple installed-package records after all corresponding files were removed.
    ///
    /// IDs absent from the installed list are ignored. The caller is responsible for
    /// validating dependency safety and completing filesystem removal before this call.
    /// The returned count is the number of records removed.
    pub fn record_uninstalled_batch(&mut self, ids: impl IntoIterator<Item = ModuleId>) -> usize {
        let ids: std::collections::HashSet<ModuleId> = ids.into_iter().collect();
        let before = self.installed_modules.len();
        self.installed_modules
            .retain(|module| !ids.contains(&module.id));
        before - self.installed_modules.len()
    }

    /// Removes an installed-package record after successful on-disk removal.
    ///
    /// Dependency safety and filesystem removal must be completed by the
    /// installer before calling this method. Returns false if no record exists.
    pub fn record_uninstalled(&mut self, id: &ModuleId) -> bool {
        if let Some(index) = self
            .installed_modules
            .iter()
            .position(|module| &module.id == id)
        {
            self.installed_modules.remove(index);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> ModuleId {
        ModuleId::new(value).unwrap()
    }

    #[test]
    fn new_profile_shows_setup_until_user_skips_or_completes_it() {
        let mut state = ModuleManagerState::new();
        assert!(state.should_show_first_launch());
        assert_eq!(state.first_launch_status(), FirstLaunchStatus::NotStarted);

        state.mark_setup_skipped();
        assert!(!state.should_show_first_launch());
        assert_eq!(state.first_launch_status(), FirstLaunchStatus::Skipped);

        state.mark_setup_completed();
        assert!(!state.should_show_first_launch());
        assert_eq!(state.first_launch_status(), FirstLaunchStatus::Completed);
    }

    #[test]
    fn selection_deduplicates_ids_without_reordering_them() {
        let mut state = ModuleManagerState::new();
        state.set_selection([id("game-boy"), id("nes"), id("game-boy")]);
        assert_eq!(state.selected_modules(), &[id("game-boy"), id("nes")]);
    }

    #[test]
    fn installed_versions_are_recorded_and_updated_in_place() {
        let mut state = ModuleManagerState::new();
        state.record_installed(id("game-boy"), "1.0.0").unwrap();
        state.record_installed(id("nes"), "2.1.0").unwrap();
        state.record_installed(id("game-boy"), "1.0.1").unwrap();

        assert_eq!(state.installed_version(&id("game-boy")), Some("1.0.1"));
        assert_eq!(state.installed_version(&id("nes")), Some("2.1.0"));
        assert_eq!(state.installed_modules().len(), 2);
        assert_eq!(state.installed_modules()[0].id, id("game-boy"));
    }

    #[test]
    fn rejects_empty_versions_without_mutating_state() {
        let mut state = ModuleManagerState::new();
        assert_eq!(
            state.record_installed(id("game-boy"), "  "),
            Err(ModuleStateError::EmptyVersion)
        );
        assert!(state.installed_modules().is_empty());
    }

    #[test]
    fn rejected_installed_batch_leaves_existing_state_unchanged() {
        let mut state = ModuleManagerState::new();
        state.record_installed(id("game-boy"), "1.0.0").unwrap();
        let before = state.clone();

        assert_eq!(
            state.record_installed_batch([
                (id("nes"), "2.0.0".to_owned()),
                (id("nes"), "2.1.0".to_owned()),
            ]),
            Err(ModuleStateError::DuplicateModuleInBatch(id("nes")))
        );
        assert_eq!(state, before);

        assert_eq!(
            state.record_installed_batch([
                (id("nes"), "2.0.0".to_owned()),
                (id("snes"), " ".to_owned()),
            ]),
            Err(ModuleStateError::EmptyVersion)
        );
        assert_eq!(state, before);
    }

    #[test]
    fn installed_batch_updates_existing_versions_and_appends_new_modules_in_order() {
        let mut state = ModuleManagerState::new();
        state.record_installed(id("game-boy"), "1.0.0").unwrap();
        state.record_installed(id("nes"), "2.0.0").unwrap();

        state
            .record_installed_batch([
                (id("nes"), "2.1.0".to_owned()),
                (id("snes"), "3.0.0".to_owned()),
                (id("game-boy"), "1.0.1".to_owned()),
            ])
            .unwrap();

        assert_eq!(
            state
                .installed_modules()
                .iter()
                .map(|module| (module.id.as_str(), module.version.as_str()))
                .collect::<Vec<_>>(),
            vec![("game-boy", "1.0.1"), ("nes", "2.1.0"), ("snes", "3.0.0"),]
        );
    }

    #[test]
    fn uninstall_batch_removes_only_matching_records_and_returns_count() {
        let mut state = ModuleManagerState::new();
        state.record_installed(id("game-boy"), "1.0.0").unwrap();
        state.record_installed(id("nes"), "2.0.0").unwrap();
        state.record_installed(id("snes"), "3.0.0").unwrap();

        assert_eq!(
            state.record_uninstalled_batch([id("nes"), id("nes"), id("missing")]),
            1
        );
        assert_eq!(
            state
                .installed_modules()
                .iter()
                .map(|module| module.id.as_str())
                .collect::<Vec<_>>(),
            vec!["game-boy", "snes"]
        );
        assert_eq!(state.record_uninstalled_batch([id("nes")]), 0);
    }

    #[test]
    fn uninstall_record_changes_only_after_success_and_is_idempotent() {
        let mut state = ModuleManagerState::new();
        state.record_installed(id("game-boy"), "1.0.0").unwrap();
        state.record_installed(id("nes"), "1.0.0").unwrap();

        assert!(state.record_uninstalled(&id("game-boy")));
        assert!(!state.record_uninstalled(&id("game-boy")));
        assert!(!state.is_installed(&id("game-boy")));
        assert!(state.is_installed(&id("nes")));
    }
}
