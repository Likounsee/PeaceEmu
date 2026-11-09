#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Runtime-facing module metadata contract.
//!
//! A valid descriptor describes compatibility and provenance; it does not
//! sandbox native code or prove that a third-party module is trustworthy.

use core::fmt;
use std::collections::HashSet;

use crate::module_management::ModuleId;
use crate::module_manifest::ReleaseVersion;
use crate::{CoreApiVersion, CURRENT_CORE_API_VERSION};

/// Capabilities that a module declares to the host.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModuleCapabilities {
    /// Module can accept content supplied by the host.
    pub content_loading: bool,
    /// Module can persist user save data.
    pub persistent_saves: bool,
    /// Module supports saving and restoring complete execution state.
    pub save_states: bool,
    /// Module can produce video frames.
    pub video_output: bool,
    /// Module can produce audio buffers.
    pub audio_output: bool,
    /// Module accepts host input events.
    pub input: bool,
}

/// Provenance metadata supplied by the module publisher.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleProvenance {
    /// Publisher name shown to the user.
    pub publisher: String,
    /// Whether this publisher is recognized by the application policy.
    ///
    /// This is metadata only. A caller must derive trust from a verified
    /// signature and a configured trust store, never from this boolean alone.
    pub policy_trusted: bool,
    /// Optional HTTPS homepage for the publisher or project.
    pub homepage: Option<String>,
}

/// Requirements that must be satisfied before a module can be activated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleRequirements {
    /// Other modules that must be present and compatible.
    pub modules: Vec<ModuleId>,
    /// Required host API version.
    pub host_api: CoreApiVersion,
}

/// Versioned, runtime-facing description of one module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleDescriptor {
    /// Stable package identifier.
    pub id: ModuleId,
    /// Exact release version of this module.
    pub version: ReleaseVersion,
    /// Host/module contract version implemented by this module.
    pub api_version: CoreApiVersion,
    /// Functionality the module actually implements.
    pub capabilities: ModuleCapabilities,
    /// Publisher and trust-policy metadata.
    pub provenance: ModuleProvenance,
    /// Dependencies and host API requirements.
    pub requirements: ModuleRequirements,
}

/// Why a module descriptor was rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModuleDescriptorError {
    /// Publisher name is empty or contains control characters.
    InvalidPublisher,
    /// Homepage is not a structurally acceptable HTTPS URL.
    InvalidHomepage,
    /// A module cannot depend on itself.
    SelfDependency,
    /// A dependency is declared more than once.
    DuplicateDependency(ModuleId),
    /// The module requires an API version this host cannot provide.
    IncompatibleHostApi {
        /// API required by the descriptor.
        required: CoreApiVersion,
        /// API supported by this host.
        supported: CoreApiVersion,
    },
}

impl fmt::Display for ModuleDescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPublisher => {
                f.write_str("module publisher must be non-empty and contain no control characters")
            }
            Self::InvalidHomepage => {
                f.write_str("module publisher homepage must be a valid HTTPS URL")
            }
            Self::SelfDependency => f.write_str("module cannot depend on itself"),
            Self::DuplicateDependency(id) => {
                write!(f, "module dependency '{id}' is declared more than once")
            }
            Self::IncompatibleHostApi {
                required,
                supported,
            } => write!(
                f,
                "module requires host API {}.{}, but this host supports {}.{}",
                required.major, required.minor, supported.major, supported.minor
            ),
        }
    }
}

impl std::error::Error for ModuleDescriptorError {}

impl ModuleDescriptor {
    /// Validates publisher metadata, dependency declarations and host API needs.
    ///
    /// This checks the descriptor in isolation; the caller must separately
    /// resolve dependency existence, verify package signatures and enforce the
    /// trust policy before loading executable module code.
    pub fn validate(&self) -> Result<(), ModuleDescriptorError> {
        if self.provenance.publisher.trim().is_empty()
            || self.provenance.publisher.chars().any(char::is_control)
        {
            return Err(ModuleDescriptorError::InvalidPublisher);
        }
        if let Some(homepage) = &self.provenance.homepage {
            if !valid_https_homepage(homepage) {
                return Err(ModuleDescriptorError::InvalidHomepage);
            }
        }
        let mut seen = HashSet::with_capacity(self.requirements.modules.len());
        for dependency in &self.requirements.modules {
            if dependency == &self.id {
                return Err(ModuleDescriptorError::SelfDependency);
            }
            if !seen.insert(dependency) {
                return Err(ModuleDescriptorError::DuplicateDependency(
                    dependency.clone(),
                ));
            }
        }
        let supported = CURRENT_CORE_API_VERSION;
        if !self.requirements.host_api.is_compatible_with(supported) {
            return Err(ModuleDescriptorError::IncompatibleHostApi {
                required: self.requirements.host_api,
                supported,
            });
        }
        Ok(())
    }

    /// Returns whether the descriptor advertises a capability.
    pub fn supports(&self, capability: ModuleCapability) -> bool {
        match capability {
            ModuleCapability::ContentLoading => self.capabilities.content_loading,
            ModuleCapability::PersistentSaves => self.capabilities.persistent_saves,
            ModuleCapability::SaveStates => self.capabilities.save_states,
            ModuleCapability::VideoOutput => self.capabilities.video_output,
            ModuleCapability::AudioOutput => self.capabilities.audio_output,
            ModuleCapability::Input => self.capabilities.input,
        }
    }
}

/// A capability that can be queried by a host UI or orchestration layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleCapability {
    /// Accepting content from the host.
    ContentLoading,
    /// Persisting save data.
    PersistentSaves,
    /// Saving/restoring execution state.
    SaveStates,
    /// Producing video frames.
    VideoOutput,
    /// Producing audio buffers.
    AudioOutput,
    /// Accepting host input.
    Input,
}

fn valid_https_homepage(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    if value
        .bytes()
        .any(|b| b.is_ascii_control() || b.is_ascii_whitespace() || b == b'\\')
    {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let host = authority.split(':').next().unwrap_or("");
    !host.is_empty()
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
