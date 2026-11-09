#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Dependency-aware planning for optional emulator modules.
//!
//! This crate-level contract intentionally contains no network, installer, or
//! platform-specific operations. A trusted application layer can consume the
//! deterministic plan and perform verified package installation.

use core::fmt;
use std::collections::{HashMap, HashSet};

/// Stable, machine-readable identifier for a distributable module.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ModuleId(String);

impl ModuleId {
    /// Creates an ID using lowercase ASCII letters, digits, and single hyphens.
    pub fn new(value: impl Into<String>) -> Result<Self, ModuleIdError> {
        let value = value.into();
        let bytes = value.as_bytes();
        if bytes.is_empty() {
            return Err(ModuleIdError::Empty);
        }
        let mut previous_hyphen = true;
        for byte in bytes {
            let valid = (*byte >= b'a' && *byte <= b'z')
                || (*byte >= b'0' && *byte <= b'9')
                || *byte == b'-';
            if !valid {
                return Err(ModuleIdError::InvalidCharacter);
            }
            if *byte == b'-' && previous_hyphen {
                return Err(ModuleIdError::InvalidSeparator);
            }
            previous_hyphen = *byte == b'-';
        }
        if previous_hyphen {
            return Err(ModuleIdError::InvalidSeparator);
        }
        Ok(Self(value))
    }

    /// Returns the canonical ID string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error while validating a module ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleIdError {
    /// The ID is empty.
    Empty,
    /// The ID contains unsupported characters.
    InvalidCharacter,
    /// The ID starts or ends with a hyphen or contains consecutive hyphens.
    InvalidSeparator,
}

impl fmt::Display for ModuleIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("module ID cannot be empty"),
            Self::InvalidCharacter => {
                f.write_str("module ID must contain lowercase ASCII letters, digits, and hyphens")
            }
            Self::InvalidSeparator => f.write_str("module ID has an invalid hyphen separator"),
        }
    }
}

impl std::error::Error for ModuleIdError {}

/// Compatibility/readiness status presented by the module catalogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleReadiness {
    /// The module is available for normal installation.
    Installable,
    /// The module can be installed, but its functionality is not yet mature.
    Experimental,
    /// The module is not installable because implementation work is incomplete.
    InDevelopment,
}

/// Immutable catalogue metadata for one module package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleDescriptor {
    /// Stable ID used by configuration and dependency declarations.
    pub id: ModuleId,
    /// Human-readable name displayed in the setup wizard.
    pub display_name: String,
    /// Current readiness shown to users.
    pub readiness: ModuleReadiness,
    /// Direct module dependencies, which must be installed first.
    pub dependencies: Vec<ModuleId>,
}

/// A validated catalogue of optional modules.
pub struct ModuleCatalog<'a> {
    descriptors: &'a [ModuleDescriptor],
}

/// Error detected while validating a catalogue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogError {
    /// Two descriptors use the same ID.
    DuplicateModule(ModuleId),
    /// A dependency points to an ID absent from the catalogue.
    MissingDependency {
        /// Module declaring the dependency.
        module: ModuleId,
        /// Dependency ID absent from the catalogue.
        dependency: ModuleId,
    },
    /// A dependency path eventually refers back to a module already on that path.
    DependencyCycle(ModuleId),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateModule(id) => write!(f, "duplicate module '{}'", id),
            Self::MissingDependency { module, dependency } => write!(
                f,
                "module '{}' requires missing module '{}'",
                module, dependency
            ),
            Self::DependencyCycle(id) => {
                write!(f, "module dependency cycle includes '{}'", id)
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// Error while resolving a user's requested installation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanError {
    /// The user requested a module absent from the catalogue.
    UnknownModule(ModuleId),
    /// A requested or required module is not ready for installation.
    NotInstallable {
        /// Module requested directly or required by another module.
        module: ModuleId,
        /// Readiness state that prevents installation.
        readiness: ModuleReadiness,
    },
    /// The catalogue must be validated before planning.
    InvalidCatalog(CatalogError),
    /// A requested module is not currently installed.
    NotInstalled(ModuleId),
    /// An installed module is absent from the catalogue, so its dependencies cannot be checked.
    UnknownInstalledModule(ModuleId),
    /// An installed module still depends on a module selected for removal.
    RequiredBy {
        /// Module that cannot yet be removed.
        module: ModuleId,
        /// Installed dependent that requires it.
        dependent: ModuleId,
    },
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownModule(id) => write!(f, "unknown module '{}'", id),
            Self::NotInstallable { module, readiness } => write!(
                f,
                "module '{}' cannot be installed in state {:?}",
                module, readiness
            ),
            Self::InvalidCatalog(error) => write!(f, "invalid module catalogue: {error}"),
            Self::NotInstalled(id) => write!(f, "module '{id}' is not installed"),
            Self::UnknownInstalledModule(id) => write!(
                f,
                "installed module '{id}' is missing from the catalogue; dependency safety cannot be verified"
            ),
            Self::RequiredBy { module, dependent } => write!(
                f,
                "module '{module}' cannot be removed while installed module '{dependent}' depends on it"
            ),
        }
    }
}

impl std::error::Error for PlanError {}

/// Deterministic installation order with dependencies before dependants.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModuleInstallPlan {
    /// Modules to install in order. Already-installed modules are omitted.
    pub install_order: Vec<ModuleId>,
}

/// Deterministic removal order with dependants removed before their dependencies.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModuleRemovalPlan {
    /// Modules to remove in order. Dependants always precede requested dependencies.
    pub removal_order: Vec<ModuleId>,
}

impl<'a> ModuleCatalog<'a> {
    /// Creates a catalogue view over immutable descriptors.
    pub const fn new(descriptors: &'a [ModuleDescriptor]) -> Self {
        Self { descriptors }
    }

    /// Returns all catalogue descriptors in their declared order.
    pub fn descriptors(&self) -> &'a [ModuleDescriptor] {
        self.descriptors
    }

    /// Checks uniqueness, dependency references, and dependency cycles.
    pub fn validate(&self) -> Result<(), CatalogError> {
        // Build the lookup once so large catalogues do not require a linear
        // scan of every descriptor for each ID and dependency.
        let indices: HashMap<ModuleId, usize> = self
            .descriptors
            .iter()
            .enumerate()
            .map(|(index, descriptor)| (descriptor.id.clone(), index))
            .collect();
        let mut seen_ids = HashSet::with_capacity(self.descriptors.len());
        for descriptor in self.descriptors {
            if !seen_ids.insert(&descriptor.id) {
                return Err(CatalogError::DuplicateModule(descriptor.id.clone()));
            }
            for dependency in &descriptor.dependencies {
                if !indices.contains_key(dependency) {
                    return Err(CatalogError::MissingDependency {
                        module: descriptor.id.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
        }

        // Validate the graph iteratively: catalogue input may be deeply nested,
        // so recursive DFS could exhaust the process stack.
        let mut state = vec![0_u8; self.descriptors.len()]; // 0 = unseen, 1 = active, 2 = complete
        let mut stack: Vec<(usize, usize)> = Vec::new();

        for start in 0..self.descriptors.len() {
            if state[start] == 2 {
                continue;
            }
            state[start] = 1;
            stack.push((start, 0));
            while !stack.is_empty() {
                let last = stack.len() - 1;
                let (current, next_dependency) = stack[last];
                let dependencies = &self.descriptors[current].dependencies;
                if next_dependency == dependencies.len() {
                    state[current] = 2;
                    stack.pop();
                    continue;
                }

                let dependency = &dependencies[next_dependency];
                stack[last].1 += 1;
                let dependency_index = *indices
                    .get(dependency)
                    .expect("dependency existence was validated before graph traversal");
                match state[dependency_index] {
                    0 => {
                        state[dependency_index] = 1;
                        stack.push((dependency_index, 0));
                    }
                    1 => return Err(CatalogError::DependencyCycle(dependency.clone())),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Resolves requested modules and transitive dependencies into install order.
    ///
    /// The result is deterministic: dependencies precede dependants, catalogue
    /// declaration order breaks ties, and IDs already listed as installed are
    /// not emitted. Missing installed dependencies are still planned.
    pub fn plan_install(
        &self,
        requested: &[ModuleId],
        installed: &[ModuleId],
    ) -> Result<ModuleInstallPlan, PlanError> {
        self.validate().map_err(PlanError::InvalidCatalog)?;
        let indices: HashMap<ModuleId, usize> = self
            .descriptors
            .iter()
            .enumerate()
            .map(|(index, descriptor)| (descriptor.id.clone(), index))
            .collect();
        let mut state = vec![0_u8; self.descriptors.len()]; // 0 = unseen, 1 = active, 2 = complete
        let mut stack: Vec<(usize, usize)> = Vec::new();
        let mut order = Vec::new();

        for id in requested {
            let Some(&start) = indices.get(id) else {
                return Err(PlanError::UnknownModule(id.clone()));
            };
            if state[start] == 2 {
                continue;
            }
            let descriptor = &self.descriptors[start];
            if !installed.contains(id) && descriptor.readiness == ModuleReadiness::InDevelopment {
                return Err(PlanError::NotInstallable {
                    module: id.clone(),
                    readiness: descriptor.readiness,
                });
            }
            state[start] = 1;
            stack.push((start, 0));

            while !stack.is_empty() {
                let last = stack.len() - 1;
                let (current, next_dependency) = stack[last];
                let descriptor = &self.descriptors[current];
                if next_dependency == descriptor.dependencies.len() {
                    state[current] = 2;
                    stack.pop();
                    if !installed.contains(&descriptor.id) {
                        order.push(descriptor.id.clone());
                    }
                    continue;
                }

                let dependency = &descriptor.dependencies[next_dependency];
                stack[last].1 += 1;
                let dependency_index = *indices
                    .get(dependency)
                    .expect("catalogue validation guarantees every dependency exists");
                match state[dependency_index] {
                    0 => {
                        let dependency_descriptor = &self.descriptors[dependency_index];
                        if !installed.contains(dependency)
                            && dependency_descriptor.readiness == ModuleReadiness::InDevelopment
                        {
                            return Err(PlanError::NotInstallable {
                                module: dependency.clone(),
                                readiness: dependency_descriptor.readiness,
                            });
                        }
                        state[dependency_index] = 1;
                        stack.push((dependency_index, 0));
                    }
                    1 => {
                        return Err(PlanError::InvalidCatalog(CatalogError::DependencyCycle(
                            dependency.clone(),
                        )));
                    }
                    _ => {}
                }
            }
        }
        Ok(ModuleInstallPlan {
            install_order: order,
        })
    }

    /// Plans a safe uninstall without removing dependencies implicitly.
    ///
    /// Every requested module must be installed. Removal is rejected if any installed
    /// module outside the removal set transitively depends on a requested module. When
    /// multiple dependent modules are selected together, dependants are ordered first.
    pub fn plan_uninstall(
        &self,
        requested: &[ModuleId],
        installed: &[ModuleId],
    ) -> Result<ModuleRemovalPlan, PlanError> {
        self.validate().map_err(PlanError::InvalidCatalog)?;
        if requested.is_empty() {
            return Ok(ModuleRemovalPlan::default());
        }
        let indices: HashMap<ModuleId, usize> = self
            .descriptors
            .iter()
            .enumerate()
            .map(|(index, descriptor)| (descriptor.id.clone(), index))
            .collect();
        // Fail closed when installed state contains a module absent from the
        // catalogue: its dependency edges are unknown, so removal safety cannot
        // be established reliably.
        for id in installed {
            if !indices.contains_key(id) {
                return Err(PlanError::UnknownInstalledModule(id.clone()));
            }
        }

        let mut removal = HashSet::new();
        for id in requested {
            if !indices.contains_key(id) {
                return Err(PlanError::UnknownModule(id.clone()));
            }
            if !installed.contains(id) {
                return Err(PlanError::NotInstalled(id.clone()));
            }
            removal.insert(id.clone());
        }

        for dependent in installed {
            if removal.contains(dependent) {
                continue;
            }
            let Some(&start) = indices.get(dependent) else {
                continue;
            };
            let mut visited = HashSet::new();
            let mut stack = vec![start];
            while let Some(index) = stack.pop() {
                for dependency in &self.descriptors[index].dependencies {
                    if removal.contains(dependency) {
                        return Err(PlanError::RequiredBy {
                            module: dependency.clone(),
                            dependent: dependent.clone(),
                        });
                    }
                    if let Some(&dependency_index) = indices.get(dependency) {
                        if visited.insert(dependency_index) {
                            stack.push(dependency_index);
                        }
                    }
                }
            }
        }

        let mut state = vec![0_u8; self.descriptors.len()];
        let mut install_order = Vec::new();
        let mut stack: Vec<(usize, usize)> = Vec::new();
        for id in requested {
            let start = indices[id];
            if state[start] == 2 {
                continue;
            }
            state[start] = 1;
            stack.push((start, 0));
            while let Some((current, next)) = stack.last().copied() {
                let dependencies = &self.descriptors[current].dependencies;
                if next == dependencies.len() {
                    state[current] = 2;
                    install_order.push(self.descriptors[current].id.clone());
                    stack.pop();
                    continue;
                }
                stack.last_mut().expect("stack is non-empty").1 += 1;
                let dependency = &dependencies[next];
                if !removal.contains(dependency) {
                    continue;
                }
                let dependency_index = indices[dependency];
                match state[dependency_index] {
                    0 => {
                        state[dependency_index] = 1;
                        stack.push((dependency_index, 0));
                    }
                    1 => {
                        return Err(PlanError::InvalidCatalog(CatalogError::DependencyCycle(
                            dependency.clone(),
                        )));
                    }
                    _ => {}
                }
            }
        }
        install_order.reverse();
        Ok(ModuleRemovalPlan {
            removal_order: install_order,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> ModuleId {
        ModuleId::new(value).unwrap()
    }

    fn descriptor(
        id_value: &str,
        display_name: &str,
        readiness: ModuleReadiness,
        dependencies: &[&str],
    ) -> ModuleDescriptor {
        ModuleDescriptor {
            id: id(id_value),
            display_name: display_name.to_owned(),
            readiness,
            dependencies: dependencies.iter().map(|value| id(value)).collect(),
        }
    }

    fn catalogue_descriptors() -> Vec<ModuleDescriptor> {
        vec![
            descriptor(
                "shared-runtime",
                "Shared runtime",
                ModuleReadiness::Installable,
                &[],
            ),
            descriptor(
                "game-boy",
                "Game Boy",
                ModuleReadiness::Experimental,
                &["shared-runtime"],
            ),
            descriptor(
                "nes",
                "NES",
                ModuleReadiness::Installable,
                &["shared-runtime"],
            ),
            descriptor(
                "experimental",
                "Experimental",
                ModuleReadiness::InDevelopment,
                &[],
            ),
        ]
    }

    #[test]
    fn validates_module_ids() {
        assert_eq!(ModuleId::new(""), Err(ModuleIdError::Empty));
        assert_eq!(
            ModuleId::new("Game-Boy"),
            Err(ModuleIdError::InvalidCharacter)
        );
        assert_eq!(ModuleId::new("-gb"), Err(ModuleIdError::InvalidSeparator));
        assert_eq!(
            ModuleId::new("gb--color"),
            Err(ModuleIdError::InvalidSeparator)
        );
        assert_eq!(id("game-boy").as_str(), "game-boy");
    }

    #[test]
    fn plans_shared_dependencies_once_and_before_requested_modules() {
        let descriptors = catalogue_descriptors();
        let catalog = ModuleCatalog::new(&descriptors);
        let plan = catalog
            .plan_install(&[id("game-boy"), id("nes")], &[])
            .unwrap();
        assert_eq!(
            plan.install_order,
            vec![id("shared-runtime"), id("game-boy"), id("nes")]
        );
    }

    #[test]
    fn omits_installed_modules_but_repairs_missing_dependencies() {
        let descriptors = catalogue_descriptors();
        let catalog = ModuleCatalog::new(&descriptors);
        let plan = catalog
            .plan_install(&[id("game-boy")], &[id("game-boy")])
            .unwrap();
        assert_eq!(plan.install_order, vec![id("shared-runtime")]);
        let plan = catalog
            .plan_install(&[id("game-boy")], &[id("shared-runtime")])
            .unwrap();
        assert_eq!(plan.install_order, vec![id("game-boy")]);
    }

    #[test]
    fn rejects_unknown_and_not_yet_implemented_modules() {
        let descriptors = catalogue_descriptors();
        let catalog = ModuleCatalog::new(&descriptors);
        assert_eq!(
            catalog.plan_install(&[id("future-system")], &[]),
            Err(PlanError::UnknownModule(id("future-system")))
        );
        assert_eq!(
            catalog.plan_install(&[id("experimental")], &[]),
            Err(PlanError::NotInstallable {
                module: id("experimental"),
                readiness: ModuleReadiness::InDevelopment,
            })
        );
    }

    #[test]
    fn rejects_uninstallable_transitive_dependency() {
        let descriptors = vec![
            descriptor(
                "unfinished-runtime",
                "Unfinished runtime",
                ModuleReadiness::InDevelopment,
                &[],
            ),
            descriptor(
                "game-boy",
                "Game Boy",
                ModuleReadiness::Installable,
                &["unfinished-runtime"],
            ),
        ];
        let catalog = ModuleCatalog::new(&descriptors);

        assert_eq!(
            catalog.plan_install(&[id("game-boy")], &[]),
            Err(PlanError::NotInstallable {
                module: id("unfinished-runtime"),
                readiness: ModuleReadiness::InDevelopment,
            })
        );
    }

    #[test]
    fn installed_unfinished_dependency_does_not_block_existing_module_plan() {
        let descriptors = vec![
            descriptor(
                "unfinished-runtime",
                "Unfinished runtime",
                ModuleReadiness::InDevelopment,
                &[],
            ),
            descriptor(
                "game-boy",
                "Game Boy",
                ModuleReadiness::Installable,
                &["unfinished-runtime"],
            ),
        ];
        let catalog = ModuleCatalog::new(&descriptors);

        let plan = catalog
            .plan_install(
                &[id("game-boy")],
                &[id("unfinished-runtime"), id("game-boy")],
            )
            .unwrap();

        assert!(plan.install_order.is_empty());
    }

    #[test]
    fn rejects_duplicate_ids_and_missing_dependencies() {
        let duplicate = vec![
            descriptor("shared-runtime", "A", ModuleReadiness::Installable, &[]),
            descriptor("shared-runtime", "B", ModuleReadiness::Installable, &[]),
        ];
        assert_eq!(
            ModuleCatalog::new(&duplicate).validate(),
            Err(CatalogError::DuplicateModule(id("shared-runtime")))
        );

        let missing = vec![descriptor(
            "game-boy",
            "Game Boy",
            ModuleReadiness::Installable,
            &["future-system"],
        )];
        assert_eq!(
            ModuleCatalog::new(&missing).validate(),
            Err(CatalogError::MissingDependency {
                module: id("game-boy"),
                dependency: id("future-system"),
            })
        );
    }

    #[test]
    fn rejects_dependency_cycles() {
        let descriptors = vec![
            descriptor("cycle-a", "A", ModuleReadiness::Installable, &["cycle-b"]),
            descriptor("cycle-b", "B", ModuleReadiness::Installable, &["cycle-a"]),
        ];
        assert_eq!(
            ModuleCatalog::new(&descriptors).validate(),
            Err(CatalogError::DependencyCycle(id("cycle-a")))
        );
    }

    #[test]
    fn validates_deep_catalogue_without_recursive_traversal() {
        const MODULE_COUNT: usize = 2048;
        let mut descriptors = (0..MODULE_COUNT)
            .map(|index| {
                descriptor(
                    &format!("node-{index}"),
                    "Node",
                    ModuleReadiness::Installable,
                    &[],
                )
            })
            .collect::<Vec<_>>();
        for (index, descriptor) in descriptors.iter_mut().enumerate().take(MODULE_COUNT - 1) {
            descriptor
                .dependencies
                .push(id(&format!("node-{}", index + 1)));
        }
        let catalog = ModuleCatalog::new(&descriptors);
        assert!(catalog.validate().is_ok());
        let plan = catalog.plan_install(&[id("node-0")], &[]).unwrap();
        assert_eq!(plan.install_order.len(), MODULE_COUNT);
        assert_eq!(
            plan.install_order.first(),
            Some(&id(&format!("node-{}", MODULE_COUNT - 1)))
        );
        assert_eq!(plan.install_order.last(), Some(&id("node-0")));
    }

    #[test]
    fn iterative_catalogue_validation_rejects_deep_cycle() {
        const MODULE_COUNT: usize = 1024;
        let mut descriptors = (0..MODULE_COUNT)
            .map(|index| {
                descriptor(
                    &format!("node-{index}"),
                    "Node",
                    ModuleReadiness::Installable,
                    &[],
                )
            })
            .collect::<Vec<_>>();
        for (index, descriptor) in descriptors.iter_mut().enumerate().take(MODULE_COUNT - 1) {
            descriptor
                .dependencies
                .push(id(&format!("node-{}", index + 1)));
        }
        descriptors[MODULE_COUNT - 1]
            .dependencies
            .push(id("node-0"));
        assert_eq!(
            ModuleCatalog::new(&descriptors).validate(),
            Err(CatalogError::DependencyCycle(id("node-0")))
        );
    }

    #[test]
    fn refuses_to_remove_a_dependency_used_by_an_installed_transitive_dependent() {
        let descriptors = vec![
            descriptor("runtime", "Runtime", ModuleReadiness::Installable, &[]),
            descriptor("video", "Video", ModuleReadiness::Installable, &["runtime"]),
            descriptor("player", "Player", ModuleReadiness::Installable, &["video"]),
        ];
        let catalog = ModuleCatalog::new(&descriptors);

        assert_eq!(
            catalog.plan_uninstall(
                &[id("runtime")],
                &[id("runtime"), id("player"), id("video")]
            ),
            Err(PlanError::RequiredBy {
                module: id("runtime"),
                dependent: id("player"),
            })
        );
    }

    #[test]
    fn empty_uninstall_is_a_no_op_even_with_unknown_legacy_installed_modules() {
        let descriptors = vec![descriptor(
            "runtime",
            "Runtime",
            ModuleReadiness::Installable,
            &[],
        )];
        let catalog = ModuleCatalog::new(&descriptors);

        let plan = catalog
            .plan_uninstall(&[], &[id("runtime"), id("legacy-plugin")])
            .unwrap();

        assert!(plan.removal_order.is_empty());
    }

    #[test]
    fn uninstall_fails_closed_when_an_installed_module_is_missing_from_catalogue() {
        let descriptors = vec![descriptor(
            "runtime",
            "Runtime",
            ModuleReadiness::Installable,
            &[],
        )];
        let catalog = ModuleCatalog::new(&descriptors);

        assert_eq!(
            catalog.plan_uninstall(&[id("runtime")], &[id("runtime"), id("legacy-plugin")],),
            Err(PlanError::UnknownInstalledModule(id("legacy-plugin")))
        );
    }

    #[test]
    fn removes_selected_dependents_before_their_dependencies() {
        let descriptors = vec![
            descriptor("runtime", "Runtime", ModuleReadiness::Installable, &[]),
            descriptor("video", "Video", ModuleReadiness::Installable, &["runtime"]),
            descriptor("player", "Player", ModuleReadiness::Installable, &["video"]),
        ];
        let catalog = ModuleCatalog::new(&descriptors);
        let plan = catalog
            .plan_uninstall(
                &[id("runtime"), id("player"), id("video")],
                &[id("runtime"), id("video"), id("player")],
            )
            .unwrap();

        assert_eq!(
            plan.removal_order,
            vec![id("player"), id("video"), id("runtime")]
        );
    }

    #[test]
    fn uninstall_rejects_unknown_and_uninstalled_modules() {
        let descriptors = catalogue_descriptors();
        let catalog = ModuleCatalog::new(&descriptors);

        assert_eq!(
            catalog.plan_uninstall(&[id("future-system")], &[]),
            Err(PlanError::UnknownModule(id("future-system")))
        );
        assert_eq!(
            catalog.plan_uninstall(&[id("game-boy")], &[]),
            Err(PlanError::NotInstalled(id("game-boy")))
        );
    }

    #[test]
    fn duplicate_install_requests_never_duplicate_a_module_in_the_plan() {
        let descriptors = catalogue_descriptors();
        let catalog = ModuleCatalog::new(&descriptors);

        let plan = catalog
            .plan_install(&[id("game-boy"), id("nes"), id("game-boy"), id("nes")], &[])
            .unwrap();

        assert_eq!(
            plan.install_order,
            vec![id("shared-runtime"), id("game-boy"), id("nes")]
        );
    }

    #[test]
    fn duplicate_uninstall_requests_never_duplicate_a_module_in_the_plan() {
        let descriptors = catalogue_descriptors();
        let catalog = ModuleCatalog::new(&descriptors);

        let plan = catalog
            .plan_uninstall(
                &[id("game-boy"), id("game-boy"), id("shared-runtime")],
                &[id("shared-runtime"), id("game-boy")],
            )
            .unwrap();

        assert_eq!(
            plan.removal_order,
            vec![id("game-boy"), id("shared-runtime")]
        );
    }

    #[test]
    fn empty_uninstall_request_is_a_no_op() {
        let descriptors = catalogue_descriptors();
        let catalog = ModuleCatalog::new(&descriptors);

        let plan = catalog
            .plan_uninstall(&[], &[id("shared-runtime")])
            .unwrap();

        assert!(plan.removal_order.is_empty());
    }

    #[test]
    fn catalogue_validation_is_independent_of_selection() {
        let descriptors = catalogue_descriptors();
        assert!(ModuleCatalog::new(&descriptors).validate().is_ok());
    }
}
