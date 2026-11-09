#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Versioned, platform-aware validation for remote module catalogue metadata.
//! This module does not fetch manifests, authenticate signatures, download, or install code.

use crate::module_management::{ModuleId, ModuleReadiness};
use crate::module_state::InstalledModule;
use core::cmp::Ordering;
use core::fmt;
use std::collections::{HashMap, HashSet};

/// Current supported schema version for module manifests.
pub const SUPPORTED_MANIFEST_SCHEMA: u32 = 1;

/// Strict numeric version with exactly three canonical decimal components.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReleaseVersion {
    /// Major version.
    pub major: u64,
    /// Minor version.
    pub minor: u64,
    /// Patch version.
    pub patch: u64,
}

impl ReleaseVersion {
    /// Parses exactly major.minor.patch; leading zeroes are rejected except for zero itself.
    pub fn parse(value: &str) -> Result<Self, VersionParseError> {
        let mut parts = value.split('.');
        let major = parse_component(parts.next().ok_or(VersionParseError)?)?;
        let minor = parse_component(parts.next().ok_or(VersionParseError)?)?;
        let patch = parse_component(parts.next().ok_or(VersionParseError)?)?;
        if parts.next().is_some() {
            return Err(VersionParseError);
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

fn is_valid_https_url(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    if value
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace() || byte == b'\\')
    {
        return false;
    }

    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return false;
    }

    let port = if let Some(bracketed) = authority.strip_prefix('[') {
        let Some((address, suffix)) = bracketed.split_once(']') else {
            return false;
        };
        if address.parse::<std::net::Ipv6Addr>().is_err() {
            return false;
        }
        if suffix.is_empty() {
            None
        } else if let Some(port) = suffix.strip_prefix(':') {
            Some(port)
        } else {
            return false;
        }
    } else {
        let mut pieces = authority.split(':');
        let host = pieces.next().unwrap_or("");
        let port = pieces.next();
        if pieces.next().is_some()
            || host.is_empty()
            || host.len() > 253
            || host.starts_with('.')
            || host.ends_with('.')
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return false;
        }
        // Numeric dotted hosts are IPv4 literals, not DNS names. Reject malformed
        // forms rather than accepting an invalid address as a hostname.
        if host
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
            && host.contains('.')
            && host.parse::<std::net::Ipv4Addr>().is_err()
        {
            return false;
        }
        port
    };

    match port {
        None => true,
        Some(port) => port.parse::<u16>().is_ok_and(|number| number != 0),
    }
}

fn parse_component(value: &str) -> Result<u64, VersionParseError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(VersionParseError);
    }
    value.parse().map_err(|_| VersionParseError)
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
    }
}
impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl fmt::Display for ReleaseVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Error parsing a strict numeric release version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionParseError;
impl fmt::Display for VersionParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("version must be canonical major.minor.patch")
    }
}
impl std::error::Error for VersionParseError {}

/// Operating system targeted by a module artifact.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TargetOs {
    /// Microsoft Windows.
    Windows,
    /// Apple macOS.
    Macos,
    /// Linux.
    Linux,
}

/// CPU architecture targeted by a module artifact.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TargetArch {
    /// 64-bit x86.
    X86_64,
    /// 64-bit ARM.
    Aarch64,
}

/// A host target; compatibility is explicit rather than inferred.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ModuleTarget {
    /// Operating system.
    pub os: TargetOs,
    /// CPU architecture.
    pub arch: TargetArch,
}

/// Metadata for an immutable downloadable package artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageArtifact {
    /// HTTPS URL to the artifact.
    pub url: String,
    /// Exact artifact size in bytes.
    pub size_bytes: u64,
    /// Expected SHA-256 digest as 64 lowercase hexadecimal characters.
    pub sha256: String,
    /// Identifier of the trusted signing key expected by the installer.
    pub signing_key_id: String,
}

/// A module record in a versioned manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestModule {
    /// Stable module ID.
    pub id: ModuleId,
    /// Package version.
    pub version: ReleaseVersion,
    /// Minimum compatible PeaceEmu version, inclusive.
    pub min_app_version: ReleaseVersion,
    /// First incompatible PeaceEmu version, exclusive.
    pub max_app_version_exclusive: Option<ReleaseVersion>,
    /// Explicitly supported operating-system and CPU combinations.
    pub targets: Vec<ModuleTarget>,
    /// User-facing implementation readiness.
    pub readiness: ModuleReadiness,
    /// Direct module dependencies.
    pub dependencies: Vec<ModuleId>,
    /// Artifact metadata; development-only modules must not publish an artifact.
    pub artifact: Option<PackageArtifact>,
}

/// Parsed module catalogue document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleManifest {
    /// Schema version used to interpret this document.
    pub schema_version: u32,
    /// Monotonically increasing catalogue revision.
    pub revision: u64,
    /// Entries in display order.
    pub modules: Vec<ManifestModule>,
}

/// Error detected while validating a manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManifestError {
    /// Manifest schema is not supported.
    UnsupportedSchema(u32),
    /// Revision zero is invalid.
    InvalidRevision,
    /// Two entries use the same module ID.
    DuplicateModule(ModuleId),
    /// A dependency is absent from the manifest.
    MissingDependency {
        /// Module declaring the dependency.
        module: ModuleId,
        /// Missing dependency ID.
        dependency: ModuleId,
    },
    /// A module declares the same direct dependency more than once.
    DuplicateDependency {
        /// Module declaring the duplicate dependency.
        module: ModuleId,
        /// Dependency that was repeated.
        dependency: ModuleId,
    },
    /// Dependency graph contains a cycle.
    DependencyCycle(ModuleId),
    /// Compatibility bounds contradict each other.
    InvalidCompatibilityRange(ModuleId),
    /// Installable or experimental entry lacks artifact metadata.
    MissingArtifact(ModuleId),
    /// Development-only entry advertises an artifact.
    ArtifactForInDevelopmentModule(ModuleId),
    /// Artifact URL, size, digest or signing-key ID is invalid.
    InvalidArtifact(ModuleId),
    /// A target appears more than once for a module.
    DuplicateTarget(ModuleId),
    /// Module has no artifact for the selected host target.
    UnsupportedTarget(ModuleId),
    /// A user-selected module ID does not exist in this manifest.
    UnknownSelection(ModuleId),
    /// A selected module is marked as still in development.
    ModuleNotInstallable(ModuleId),
    /// Application version is outside the module compatibility range.
    IncompatibleApplicationVersion(ModuleId),
    /// Summing package sizes exceeded the integer range.
    ArtifactSizeOverflow,
    /// The selected installation exceeds the caller's download budget.
    DownloadLimitExceeded {
        /// Total bytes required by the dependency-complete plan.
        required_bytes: u64,
        /// Maximum bytes permitted by the caller.
        maximum_bytes: u64,
    },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(v) => write!(f, "unsupported manifest schema {v}"),
            Self::InvalidRevision => f.write_str("manifest revision must be greater than zero"),
            Self::DuplicateModule(id) => write!(f, "duplicate module '{id}'"),
            Self::MissingDependency { module, dependency } => write!(
                f,
                "module '{module}' requires missing module '{dependency}'"
            ),
            Self::DuplicateDependency { module, dependency } => write!(
                f,
                "module '{module}' declares dependency '{dependency}' more than once"
            ),
            Self::DependencyCycle(id) => write!(f, "module dependency cycle includes '{id}'"),
            Self::InvalidCompatibilityRange(id) => write!(
                f,
                "module '{id}' has an invalid application compatibility range"
            ),
            Self::MissingArtifact(id) => write!(f, "module '{id}' requires artifact metadata"),
            Self::ArtifactForInDevelopmentModule(id) => write!(
                f,
                "in-development module '{id}' cannot publish an installable artifact"
            ),
            Self::InvalidArtifact(id) => write!(f, "module '{id}' has invalid artifact metadata"),
            Self::DuplicateTarget(id) => write!(f, "module '{id}' declares a duplicate target"),
            Self::UnsupportedTarget(id) => write!(
                f,
                "module '{id}' has no artifact for the current host target"
            ),
            Self::UnknownSelection(id) => {
                write!(f, "selected module {id} is absent from the manifest")
            }
            Self::ModuleNotInstallable(id) => write!(f, "module {id} is not installable yet"),
            Self::IncompatibleApplicationVersion(id) => write!(
                f,
                "module '{id}' is incompatible with this PeaceEmu version"
            ),
            Self::ArtifactSizeOverflow => {
                f.write_str("total module download size exceeds the supported range")
            }
            Self::DownloadLimitExceeded {
                required_bytes,
                maximum_bytes,
            } => write!(
                f,
                "installation requires {required_bytes} bytes, exceeding the {maximum_bytes}-byte download limit"
            ),
        }
    }
}
impl std::error::Error for ManifestError {}

/// A dependency-complete installation plan with its exact declared download size.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallationPreflight<'a> {
    /// Modules in dependency-first installation order.
    pub modules: Vec<&'a ManifestModule>,
    /// Sum of the declared artifact sizes, counting each module once.
    pub total_download_bytes: u64,
}

/// Kind of change required to bring an installed module to the catalogue version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleUpdateAction {
    /// The module is not currently installed.
    Install,
    /// A different version is installed and should be replaced.
    Upgrade,
}

/// One actionable entry in a dependency-first update plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleUpdate<'a> {
    /// Catalogue entry to install.
    pub module: &'a ManifestModule,
    /// Required operation for this module.
    pub action: ModuleUpdateAction,
    /// Version currently recorded as installed, if any.
    pub installed_version: Option<&'a str>,
}

impl ModuleManifest {
    /// Validates schema, IDs, dependency references, version ranges and artifact metadata.
    /// This is structural validation only; callers must authenticate the manifest separately.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != SUPPORTED_MANIFEST_SCHEMA {
            return Err(ManifestError::UnsupportedSchema(self.schema_version));
        }
        if self.revision == 0 {
            return Err(ManifestError::InvalidRevision);
        }

        // Build the ID index once. Besides powering iterative cycle detection,
        // this makes duplicate-ID and dependency-existence checks expected O(1)
        // rather than rescanning the entire untrusted catalogue for each entry.
        let mut indices: HashMap<ModuleId, usize> = HashMap::with_capacity(self.modules.len());
        for (index, module) in self.modules.iter().enumerate() {
            if indices.insert(module.id.clone(), index).is_some() {
                return Err(ManifestError::DuplicateModule(module.id.clone()));
            }
        }

        for module in &self.modules {
            if module
                .max_app_version_exclusive
                .is_some_and(|max| max <= module.min_app_version)
            {
                return Err(ManifestError::InvalidCompatibilityRange(module.id.clone()));
            }
            let mut seen_targets = HashSet::with_capacity(module.targets.len());
            for target in &module.targets {
                if !seen_targets.insert(*target) {
                    return Err(ManifestError::DuplicateTarget(module.id.clone()));
                }
            }
            match (&module.artifact, module.readiness) {
                (None, ModuleReadiness::Installable | ModuleReadiness::Experimental) => {
                    return Err(ManifestError::MissingArtifact(module.id.clone()))
                }
                (Some(_), ModuleReadiness::InDevelopment) => {
                    return Err(ManifestError::ArtifactForInDevelopmentModule(
                        module.id.clone(),
                    ))
                }
                (Some(artifact), _) => {
                    let valid_url = is_valid_https_url(&artifact.url);
                    let valid_digest = artifact.sha256.len() == 64
                        && artifact
                            .sha256
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                    if !valid_url
                        || artifact.size_bytes == 0
                        || !valid_digest
                        || artifact.signing_key_id.trim().is_empty()
                    {
                        return Err(ManifestError::InvalidArtifact(module.id.clone()));
                    }
                }
                (None, ModuleReadiness::InDevelopment) => {}
            }
            let mut seen_dependencies = HashSet::with_capacity(module.dependencies.len());
            for dependency in &module.dependencies {
                if !seen_dependencies.insert(dependency) {
                    return Err(ManifestError::DuplicateDependency {
                        module: module.id.clone(),
                        dependency: dependency.clone(),
                    });
                }
                if !indices.contains_key(dependency) {
                    return Err(ManifestError::MissingDependency {
                        module: module.id.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
        }
        // Dependencies have been checked above, and module IDs are unique. Use an
        // explicit DFS stack so untrusted, deeply nested manifests cannot exhaust
        // the call stack during cycle detection.
        let mut state = vec![0_u8; self.modules.len()]; // 0 = unseen, 1 = active, 2 = complete
        let mut stack: Vec<(usize, usize)> = Vec::new();

        for start in 0..self.modules.len() {
            if state[start] == 2 {
                continue;
            }
            state[start] = 1;
            stack.push((start, 0));

            while !stack.is_empty() {
                let last = stack.len() - 1;
                let (current, next_dependency) = stack[last];
                let dependencies = &self.modules[current].dependencies;

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
                    1 => return Err(ManifestError::DependencyCycle(dependency.clone())),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Resolves a user selection into a dependency-first installation plan.
    ///
    /// Only selected modules and their transitive dependencies are returned. Duplicate
    /// selections are ignored while preserving the user's selection order for independent
    /// modules. In-development modules, incompatible versions and unsupported targets are
    /// rejected instead of being silently omitted.
    pub fn plan_installation_for_target(
        &self,
        selected: &[ModuleId],
        app_version: ReleaseVersion,
        target: ModuleTarget,
    ) -> Result<Vec<&ManifestModule>, ManifestError> {
        self.validate()?;

        let indices: HashMap<ModuleId, usize> = self
            .modules
            .iter()
            .enumerate()
            .map(|(index, module)| (module.id.clone(), index))
            .collect();
        let mut plan = Vec::new();
        let mut included = vec![false; self.modules.len()];

        for selected_id in selected {
            let start = *indices
                .get(selected_id)
                .ok_or_else(|| ManifestError::UnknownSelection(selected_id.clone()))?;
            let mut stack = vec![(start, false)];
            while let Some((index, expanded)) = stack.pop() {
                if included[index] {
                    continue;
                }
                let module = &self.modules[index];
                if !expanded {
                    if module.readiness == ModuleReadiness::InDevelopment {
                        return Err(ManifestError::ModuleNotInstallable(module.id.clone()));
                    }
                    if app_version < module.min_app_version
                        || module
                            .max_app_version_exclusive
                            .is_some_and(|max| app_version >= max)
                    {
                        return Err(ManifestError::IncompatibleApplicationVersion(
                            module.id.clone(),
                        ));
                    }
                    if module.artifact.is_none() {
                        return Err(ManifestError::MissingArtifact(module.id.clone()));
                    }
                    if !module.targets.contains(&target) {
                        return Err(ManifestError::UnsupportedTarget(module.id.clone()));
                    }
                    stack.push((index, true));
                    for dependency in module.dependencies.iter().rev() {
                        let dependency_index = *indices
                            .get(dependency)
                            .expect("manifest validation checked dependency existence");
                        if !included[dependency_index] {
                            stack.push((dependency_index, false));
                        }
                    }
                } else if !included[index] {
                    included[index] = true;
                    plan.push(module);
                }
            }
        }
        Ok(plan)
    }

    /// Resolves an installation and checks its declared download size against a budget.
    ///
    /// The total includes each transitive dependency exactly once and uses checked arithmetic.
    /// This is a metadata preflight only: it does not download, verify, extract, or install files.
    pub fn preflight_installation_for_target(
        &self,
        selected: &[ModuleId],
        app_version: ReleaseVersion,
        target: ModuleTarget,
        maximum_download_bytes: u64,
    ) -> Result<InstallationPreflight<'_>, ManifestError> {
        let modules = self.plan_installation_for_target(selected, app_version, target)?;
        let total_download_bytes = modules.iter().try_fold(0_u64, |total, module| {
            let artifact = module
                .artifact
                .as_ref()
                .expect("installation planning rejects modules without artifacts");
            total
                .checked_add(artifact.size_bytes)
                .ok_or(ManifestError::ArtifactSizeOverflow)
        })?;
        if total_download_bytes > maximum_download_bytes {
            return Err(ManifestError::DownloadLimitExceeded {
                required_bytes: total_download_bytes,
                maximum_bytes: maximum_download_bytes,
            });
        }
        Ok(InstallationPreflight {
            modules,
            total_download_bytes,
        })
    }

    /// Plans only the missing or outdated modules for a selected dependency closure.
    ///
    /// The returned entries preserve dependency-first installation order. Already-installed
    /// modules at exactly the catalogue version are omitted, including dependencies shared
    /// by multiple selected modules. Version strings in installed state are compared with the
    /// canonical catalogue version; an unrecognized or older string is treated as outdated.
    pub fn plan_update_for_target<'a>(
        &'a self,
        selected: &[ModuleId],
        installed: &'a [InstalledModule],
        app_version: ReleaseVersion,
        target: ModuleTarget,
    ) -> Result<Vec<ModuleUpdate<'a>>, ManifestError> {
        let installation = self.plan_installation_for_target(selected, app_version, target)?;
        let installed_by_id: HashMap<&ModuleId, &InstalledModule> =
            installed.iter().map(|entry| (&entry.id, entry)).collect();

        let mut updates = Vec::new();
        for module in installation {
            let current = installed_by_id.get(&module.id);
            let wanted_version = module.version.to_string();
            match current {
                Some(entry) if entry.version == wanted_version => {}
                Some(entry) => updates.push(ModuleUpdate {
                    module,
                    action: ModuleUpdateAction::Upgrade,
                    installed_version: Some(entry.version.as_str()),
                }),
                None => updates.push(ModuleUpdate {
                    module,
                    action: ModuleUpdateAction::Install,
                    installed_version: None,
                }),
            }
        }
        Ok(updates)
    }

    /// Lists modules that are individually installable by this application on a host target.
    ///
    /// This is intended for catalogue/setup UIs: unavailable entries are omitted rather than
    /// causing the whole list to fail. The manifest itself must still be structurally valid;
    /// dependency availability is resolved later by `plan_installation_for_target`.
    pub fn compatible_modules_for_target(
        &self,
        app_version: ReleaseVersion,
        target: ModuleTarget,
    ) -> Result<Vec<&ManifestModule>, ManifestError> {
        self.validate()?;
        Ok(self
            .modules
            .iter()
            .filter(|module| {
                module.readiness != ModuleReadiness::InDevelopment
                    && module.artifact.is_some()
                    && module.targets.contains(&target)
                    && app_version >= module.min_app_version
                    && module
                        .max_app_version_exclusive
                        .map_or(true, |max| app_version < max)
            })
            .collect())
    }
    /// Validates the manifest and filters entries for an explicit host and application version.
    /// In-development entries are omitted. A compatibility error is returned rather than silently
    /// presenting an incompatible package as available.
    pub fn modules_for_target(
        &self,
        app_version: ReleaseVersion,
        target: ModuleTarget,
    ) -> Result<Vec<&ManifestModule>, ManifestError> {
        self.validate()?;
        let mut compatible = Vec::new();
        for module in &self.modules {
            if module.readiness == ModuleReadiness::InDevelopment {
                continue;
            }
            if app_version < module.min_app_version
                || module
                    .max_app_version_exclusive
                    .is_some_and(|max| app_version >= max)
            {
                return Err(ManifestError::IncompatibleApplicationVersion(
                    module.id.clone(),
                ));
            }
            if !module.targets.contains(&target) {
                return Err(ManifestError::UnsupportedTarget(module.id.clone()));
            }
            compatible.push(module);
        }
        Ok(compatible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(value: &str) -> ModuleId {
        ModuleId::new(value).unwrap()
    }
    fn target() -> ModuleTarget {
        ModuleTarget {
            os: TargetOs::Linux,
            arch: TargetArch::X86_64,
        }
    }
    fn artifact() -> PackageArtifact {
        PackageArtifact {
            url: "https://packages.example.invalid/module.tar.zst".into(),
            size_bytes: 1024,
            sha256: "a".repeat(64),
            signing_key_id: "release-2026".into(),
        }
    }
    fn module(name: &str, deps: &[&str]) -> ManifestModule {
        ManifestModule {
            id: id(name),
            version: ReleaseVersion::parse("1.2.3").unwrap(),
            min_app_version: ReleaseVersion::parse("0.1.0").unwrap(),
            max_app_version_exclusive: Some(ReleaseVersion::parse("2.0.0").unwrap()),
            targets: vec![target()],
            readiness: ModuleReadiness::Installable,
            dependencies: deps.iter().map(|v| id(v)).collect(),
            artifact: Some(artifact()),
        }
    }
    fn manifest(modules: Vec<ManifestModule>) -> ModuleManifest {
        ModuleManifest {
            schema_version: SUPPORTED_MANIFEST_SCHEMA,
            revision: 1,
            modules,
        }
    }

    #[test]
    fn parses_and_orders_numeric_versions() {
        let a = ReleaseVersion::parse("1.9.0").unwrap();
        let b = ReleaseVersion::parse("1.10.0").unwrap();
        assert!(a < b);
        assert_eq!(b.to_string(), "1.10.0");
        for bad in [
            "", "1", "1.2", "01.2.3", "1.02.3", "1.2.3.4", "v1.2.3", "1.-2.3",
        ] {
            assert!(ReleaseVersion::parse(bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn validates_manifest_and_filters_by_target_and_app_version() {
        let m = manifest(vec![
            module("shared-runtime", &[]),
            module("game-boy", &["shared-runtime"]),
        ]);
        let selected = m
            .modules_for_target(ReleaseVersion::parse("1.5.0").unwrap(), target())
            .unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[1].id.as_str(), "game-boy");
        assert!(matches!(
            m.modules_for_target(ReleaseVersion::parse("2.0.0").unwrap(), target()),
            Err(ManifestError::IncompatibleApplicationVersion(_))
        ));
    }

    #[test]
    fn rejects_duplicate_ids_missing_dependencies_and_cycles() {
        assert!(matches!(
            manifest(vec![module("one", &[]), module("one", &[])]).validate(),
            Err(ManifestError::DuplicateModule(_))
        ));
        assert!(matches!(
            manifest(vec![module("one", &["missing"])]).validate(),
            Err(ManifestError::MissingDependency { .. })
        ));
        assert!(matches!(
            manifest(vec![
                module("shared", &[]),
                module("one", &["shared", "shared"])
            ])
            .validate(),
            Err(ManifestError::DuplicateDependency { .. })
        ));
        assert!(matches!(
            manifest(vec![module("one", &["two"]), module("two", &["one"])]).validate(),
            Err(ManifestError::DependencyCycle(_))
        ));
    }

    #[test]
    fn validates_deep_dependency_chain_without_recursive_traversal() {
        const MODULE_COUNT: usize = 2048;
        let mut modules = (0..MODULE_COUNT)
            .map(|index| module(&format!("node-{index}"), &[]))
            .collect::<Vec<_>>();
        for (index, module) in modules.iter_mut().enumerate().take(MODULE_COUNT - 1) {
            module.dependencies.push(id(&format!("node-{}", index + 1)));
        }

        assert!(manifest(modules).validate().is_ok());
    }

    #[test]
    fn rejects_a_cycle_at_the_end_of_a_deep_dependency_chain() {
        const MODULE_COUNT: usize = 1024;
        let mut modules = (0..MODULE_COUNT)
            .map(|index| module(&format!("node-{index}"), &[]))
            .collect::<Vec<_>>();
        for (index, module) in modules.iter_mut().enumerate().take(MODULE_COUNT - 1) {
            module.dependencies.push(id(&format!("node-{}", index + 1)));
        }
        modules[MODULE_COUNT - 1].dependencies.push(id("node-0"));

        assert!(matches!(
            manifest(modules).validate(),
            Err(ManifestError::DependencyCycle(_))
        ));
    }

    #[test]
    fn rejects_invalid_artifacts() {
        let mut oversized_label = module("game-boy", &[]);
        oversized_label.artifact.as_mut().unwrap().url =
            format!("https://{}.example.invalid/file.tar", "a".repeat(64));
        assert!(matches!(
            manifest(vec![oversized_label]).validate(),
            Err(ManifestError::InvalidArtifact(_))
        ));

        let mut entry = module("game-boy", &[]);
        entry.artifact.as_mut().unwrap().url = "http://example.invalid/file".into();
        assert!(matches!(
            manifest(vec![entry]).validate(),
            Err(ManifestError::InvalidArtifact(_))
        ));
        for url in [
            "https://",
            "https:///file.tar",
            "https://?download=1",
            "https://:443/file",
            "https://user@packages.example.invalid/file.tar",
            "https://packages..example.invalid/file.tar",
            "https://.packages.example.invalid/file.tar",
            "https://packages.example.invalid./file.tar",
            "https://packages.example.invalid\\\\file.tar",
            "https://packages.example.invalid:abc/file.tar",
            "https://packages.example.invalid:0/file.tar",
            "https://packages.example.invalid:65536/file.tar",
            "https://-packages.example.invalid/file.tar",
            "https://packages-.example.invalid/file.tar",
            "https://999.999.999.999/file.tar",
            "https://192.168.1.999/file.tar",
            "https://[not-ipv6]/file.tar",
        ] {
            let mut entry = module("game-boy", &[]);
            entry.artifact.as_mut().unwrap().url = url.into();
            assert!(
                matches!(
                    manifest(vec![entry]).validate(),
                    Err(ManifestError::InvalidArtifact(_))
                ),
                "accepted invalid HTTPS authority: {url}"
            );
        }
        let mut entry = module("game-boy", &[]);
        entry.artifact.as_mut().unwrap().sha256 = "not-a-hash".into();
        assert!(matches!(
            manifest(vec![entry]).validate(),
            Err(ManifestError::InvalidArtifact(_))
        ));
        let mut entry = module("game-boy", &[]);
        entry.artifact = None;
        assert!(matches!(
            manifest(vec![entry]).validate(),
            Err(ManifestError::MissingArtifact(_))
        ));
    }

    #[test]
    fn accepts_valid_https_authorities_and_ports() {
        for url in [
            "https://packages.example.invalid/file.tar",
            "https://packages.example.invalid:443/file.tar",
            "https://127.0.0.1:8443/file.tar",
            "https://[::1]:8443/file.tar",
        ] {
            assert!(is_valid_https_url(url), "rejected valid HTTPS URL: {url}");
        }
    }

    #[test]
    fn rejects_duplicate_targets_and_artifacts_for_development_modules() {
        let mut entry = module("game-boy", &[]);
        entry.targets.push(target());
        assert!(matches!(
            manifest(vec![entry]).validate(),
            Err(ManifestError::DuplicateTarget(_))
        ));
        let mut entry = module("future-console", &[]);
        entry.readiness = ModuleReadiness::InDevelopment;
        assert!(matches!(
            manifest(vec![entry]).validate(),
            Err(ManifestError::ArtifactForInDevelopmentModule(_))
        ));
    }

    #[test]
    fn in_development_entries_are_not_installable_choices() {
        let mut entry = module("future-console", &[]);
        entry.readiness = ModuleReadiness::InDevelopment;
        entry.artifact = None;
        assert!(manifest(vec![entry])
            .modules_for_target(ReleaseVersion::parse("1.0.0").unwrap(), target())
            .unwrap()
            .is_empty());
    }
}
