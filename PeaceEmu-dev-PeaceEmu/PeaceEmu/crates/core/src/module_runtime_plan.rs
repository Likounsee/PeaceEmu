#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Dependency and capability planning for runtime module activation.
//!
//! Planning is deterministic and does not load executable code. The host must
//! still verify package provenance and enforce its process-isolation policy.

use core::fmt;
use std::collections::{HashMap, HashSet};

use crate::module_descriptor::{ModuleCapability, ModuleDescriptor, ModuleDescriptorError};
use crate::module_management::ModuleId;

/// Validated view of runtime descriptors available to the host.
pub struct ModuleRuntimeCatalog<'a> {
    descriptors: &'a [ModuleDescriptor],
}

/// Catalogue validation or activation planning failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimePlanError {
    /// A descriptor failed its own metadata/API validation.
    InvalidDescriptor {
        /// ID of the descriptor that failed validation.
        module: ModuleId,
        /// Underlying descriptor validation error.
        source: ModuleDescriptorError,
    },
    /// Two descriptors use the same module ID.
    DuplicateModule(ModuleId),
    /// A dependency is not present in this runtime catalogue.
    MissingDependency {
        /// Module that declares the dependency.
        module: ModuleId,
        /// Referenced module absent from the catalogue.
        dependency: ModuleId,
    },
    /// A dependency cycle was found.
    DependencyCycle(ModuleId),
    /// The requested module is not in the catalogue.
    UnknownModule(ModuleId),
    /// The selected module does not advertise a required capability.
    MissingCapability {
        /// Module that lacks the required capability.
        module: ModuleId,
        /// Capability required by the host or selected module.
        capability: ModuleCapability,
    },
}

impl fmt::Display for RuntimePlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDescriptor { module, source } => {
                write!(f, "invalid descriptor for '{module}': {source}")
            }
            Self::DuplicateModule(id) => write!(f, "duplicate runtime module '{id}'"),
            Self::MissingDependency { module, dependency } => write!(
                f,
                "runtime module '{module}' requires missing module '{dependency}'"
            ),
            Self::DependencyCycle(id) => {
                write!(f, "runtime module dependency cycle includes '{id}'")
            }
            Self::UnknownModule(id) => write!(f, "unknown runtime module '{id}'"),
            Self::MissingCapability { module, capability } => write!(
                f,
                "runtime module '{module}' does not provide required capability {capability:?}"
            ),
        }
    }
}

impl std::error::Error for RuntimePlanError {}

/// Dependency-first activation order for a selected set of modules.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModuleActivationPlan {
    /// Unique modules to activate, with every dependency before its dependants.
    pub activation_order: Vec<ModuleId>,
}

impl<'a> ModuleRuntimeCatalog<'a> {
    /// Creates a runtime catalogue view over descriptors.
    pub const fn new(descriptors: &'a [ModuleDescriptor]) -> Self {
        Self { descriptors }
    }

    /// Returns the descriptors in their source order.
    pub fn descriptors(&self) -> &'a [ModuleDescriptor] {
        self.descriptors
    }

    /// Validates descriptors, dependency references, uniqueness and cycles.
    pub fn validate(&self) -> Result<(), RuntimePlanError> {
        let mut index = HashMap::with_capacity(self.descriptors.len());
        for (position, descriptor) in self.descriptors.iter().enumerate() {
            descriptor
                .validate()
                .map_err(|source| RuntimePlanError::InvalidDescriptor {
                    module: descriptor.id.clone(),
                    source,
                })?;
            if index.insert(descriptor.id.clone(), position).is_some() {
                return Err(RuntimePlanError::DuplicateModule(descriptor.id.clone()));
            }
        }
        for descriptor in self.descriptors {
            for dependency in &descriptor.requirements.modules {
                if !index.contains_key(dependency) {
                    return Err(RuntimePlanError::MissingDependency {
                        module: descriptor.id.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
        }
        // Iterative three-colour DFS avoids call-stack growth for deep chains.
        let mut state = vec![0_u8; self.descriptors.len()];
        for start in 0..self.descriptors.len() {
            if state[start] != 0 {
                continue;
            }
            let mut stack = vec![(start, 0_usize)];
            state[start] = 1;
            while let Some((node, next_dependency)) = stack.last_mut() {
                let descriptor = &self.descriptors[*node];
                if *next_dependency >= descriptor.requirements.modules.len() {
                    state[*node] = 2;
                    stack.pop();
                    continue;
                }
                let dependency_id = &descriptor.requirements.modules[*next_dependency];
                *next_dependency += 1;
                let dependency_index = index[dependency_id];
                match state[dependency_index] {
                    0 => {
                        state[dependency_index] = 1;
                        stack.push((dependency_index, 0));
                    }
                    1 => return Err(RuntimePlanError::DependencyCycle(dependency_id.clone())),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Resolves selected modules to a dependency-first activation plan.
    /// Required capabilities are enforced for each selected root; dependencies
    /// are validated and ordered but need not implement the root capabilities.
    pub fn plan_activation(
        &self,
        selected: &[ModuleId],
        required_capabilities: &[ModuleCapability],
    ) -> Result<ModuleActivationPlan, RuntimePlanError> {
        self.validate()?;
        let index: HashMap<&ModuleId, usize> = self
            .descriptors
            .iter()
            .enumerate()
            .map(|(position, descriptor)| (&descriptor.id, position))
            .collect();
        for id in selected {
            let Some(&position) = index.get(id) else {
                return Err(RuntimePlanError::UnknownModule(id.clone()));
            };
            for capability in required_capabilities {
                if !self.descriptors[position].supports(*capability) {
                    return Err(RuntimePlanError::MissingCapability {
                        module: id.clone(),
                        capability: *capability,
                    });
                }
            }
        }
        let mut emitted = HashSet::with_capacity(self.descriptors.len());
        let mut order = Vec::new();
        for root in selected {
            let mut stack = vec![(index[root], false)];
            while let Some((position, expanded)) = stack.pop() {
                let descriptor = &self.descriptors[position];
                if emitted.contains(&descriptor.id) {
                    continue;
                }
                if expanded {
                    emitted.insert(descriptor.id.clone());
                    order.push(descriptor.id.clone());
                    continue;
                }
                stack.push((position, true));
                for dependency in descriptor.requirements.modules.iter().rev() {
                    let dependency_index = index[dependency];
                    if !emitted.contains(dependency) {
                        stack.push((dependency_index, false));
                    }
                }
            }
        }
        Ok(ModuleActivationPlan {
            activation_order: order,
        })
    }
}
