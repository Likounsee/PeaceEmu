#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Platform-neutral contracts shared by PeaceEmu and its emulation cores.
//!
//! This crate deliberately contains no console implementation. A system is not
//! supported merely because it can be represented by these types.

/// Generic bounded content envelopes for untrusted payloads.
pub mod content;

/// Platform-neutral input and owned audio/video payload contracts.
pub mod core_io;

/// Explicit CPU execution strategy planning for heterogeneous console modules.
pub mod execution_strategy;

/// Bounded, versioned save-state envelopes and compatibility checks.
pub mod save_state;

/// Crash-conscious persistence for save-state envelopes.
pub mod save_state_store;

/// Host capability facts and console-independent performance planning.
pub mod host_profile;

/// Bounded host-side orchestration and independent session ownership.
pub mod host_runtime;

/// Console-independent lifecycle controller for an active emulation session.
pub mod runtime;

/// Runtime-facing module metadata, capability and provenance contract.
pub mod module_descriptor;

/// Dependency-aware activation planning for runtime module descriptors.
pub mod module_runtime_plan;

/// Dependency-aware planning for optional console and shared-runtime modules.
pub mod module_management;

/// Schema and compatibility validation for versioned module manifests.
pub mod module_manifest;

/// Pinned-key trust policy and anti-rollback checks for signed manifests.
pub mod module_manifest_trust;

/// First-launch decisions and installed module state.
pub mod module_state;

/// Versioned disk persistence for first-launch and module state.
pub mod module_state_store;

/// Bounded package artifact staging and digest-verification boundary.
pub mod artifact_staging;

/// Cross-platform archive path validation before package extraction.
pub mod archive_safety;

/// Crash-recovery journal for package installation transactions.
pub mod module_install_journal;

/// Host-driven orchestration for journaled module installation.
pub mod module_install;

/// Bounded, versioned IPC framing for isolated external core workers.
pub mod module_worker_protocol;

use core::fmt;

/// Stable machine-readable identifier for an emulated system.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SystemId(&'static str);

impl SystemId {
    /// Creates an identifier after validating its syntax.
    ///
    /// Identifiers are lowercase ASCII words separated by single hyphens,
    /// for example `"example-console"`.
    pub const fn new(value: &'static str) -> Result<Self, IdentifierError> {
        let bytes = value.as_bytes();
        if bytes.is_empty() {
            return Err(IdentifierError::Empty);
        }

        let mut index = 0;
        let mut previous_hyphen = true;
        while index < bytes.len() {
            let byte = bytes[index];
            let is_lower = byte >= b'a' && byte <= b'z';
            let is_digit = byte >= b'0' && byte <= b'9';
            let is_hyphen = byte == b'-';
            if !(is_lower || is_digit || is_hyphen) {
                return Err(IdentifierError::InvalidCharacter);
            }
            if is_hyphen && previous_hyphen {
                return Err(IdentifierError::InvalidSeparator);
            }
            previous_hyphen = is_hyphen;
            index += 1;
        }

        if previous_hyphen {
            return Err(IdentifierError::InvalidSeparator);
        }
        Ok(Self(value))
    }

    /// Returns the canonical identifier string.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Failure while validating a system identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentifierError {
    /// The identifier is empty.
    Empty,
    /// A character other than lowercase ASCII letters, digits, or hyphens was used.
    InvalidCharacter,
    /// A hyphen appears at the beginning/end or two hyphens are adjacent.
    InvalidSeparator,
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("system identifier cannot be empty"),
            Self::InvalidCharacter => formatter.write_str(
                "system identifier must contain only lowercase ASCII letters, digits, and hyphens",
            ),
            Self::InvalidSeparator => formatter.write_str(
                "system identifier cannot start or end with a hyphen or contain adjacent hyphens",
            ),
        }
    }
}

impl std::error::Error for IdentifierError {}

/// Features a core actually implements. These are declarations, not inferred
/// from the console name or from whether the crate compiles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CoreCapabilities {
    /// Core can load content into a fresh or reset machine.
    pub content_loading: bool,
    /// Core supports persistent battery-backed or equivalent data.
    pub persistent_saves: bool,
    /// Core can serialize and restore a complete deterministic machine state.
    pub save_states: bool,
    /// Core can emit video frames.
    pub video_output: bool,
    /// Core can emit audio samples.
    pub audio_output: bool,
    /// Core supports input events.
    pub input: bool,
}

/// Version of the host/core contract, independent of the core implementation version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreApiVersion {
    /// Breaking-contract generation. Must match the host's major version.
    pub major: u16,
    /// Backward-compatible additions within a major generation.
    pub minor: u16,
}

impl CoreApiVersion {
    /// Returns whether this core's contract requirements are supported by the host.
    ///
    /// A matching major version is required; a core cannot require a newer minor
    /// version than the host provides.
    pub const fn is_compatible_with(self, host: Self) -> bool {
        self.major == host.major && self.minor <= host.minor
    }
}

/// Contract version implemented by this source tree.
pub const CURRENT_CORE_API_VERSION: CoreApiVersion = CoreApiVersion { major: 1, minor: 0 };

#[cfg(test)]
mod api_version_tests {
    use super::CoreApiVersion;

    #[test]
    fn compatibility_requires_same_major_and_no_newer_minor_requirement() {
        let host = CoreApiVersion { major: 1, minor: 4 };

        assert!(CoreApiVersion { major: 1, minor: 0 }.is_compatible_with(host));
        assert!(CoreApiVersion { major: 1, minor: 4 }.is_compatible_with(host));
        assert!(!CoreApiVersion { major: 1, minor: 5 }.is_compatible_with(host));
        assert!(!CoreApiVersion { major: 2, minor: 0 }.is_compatible_with(host));
        assert!(!CoreApiVersion { major: 0, minor: 4 }.is_compatible_with(host));
    }
}

/// Human- and machine-readable metadata for one concrete core.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreDescriptor {
    /// Host/core contract version implemented by this core.
    pub api_version: CoreApiVersion,
    /// Stable identifier used in configuration and diagnostics.
    pub system_id: SystemId,
    /// Display name shown to users.
    pub display_name: &'static str,
    /// Version of this core implementation.
    pub core_version: &'static str,
    /// Features implemented by this core.
    pub capabilities: CoreCapabilities,
}

/// Errors returned when a requested core operation cannot be completed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoreError {
    /// Content is malformed, unsupported, or rejected by the core.
    InvalidContent(&'static str),
    /// The operation requires a content image or initialized machine.
    NotInitialized,
    /// The requested capability is not implemented by this core.
    Unsupported(&'static str),
    /// A state file is invalid or incompatible with this core version.
    InvalidState(&'static str),
    /// An execution error with a stable, human-readable explanation.
    Execution(&'static str),
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidContent(reason) => write!(formatter, "invalid content: {reason}"),
            Self::NotInitialized => formatter.write_str("core is not initialized"),
            Self::Unsupported(feature) => write!(formatter, "unsupported capability: {feature}"),
            Self::InvalidState(reason) => write!(formatter, "invalid save state: {reason}"),
            Self::Execution(reason) => write!(formatter, "emulation execution failed: {reason}"),
        }
    }
}

impl std::error::Error for CoreError {}

/// Event emitted when a core reaches an observable boundary.
///
/// Cores may define richer platform-specific event types internally. The common
/// contract intentionally does not force every system into a frame-based model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreEvent {
    /// The core produced a complete video frame.
    FrameReady,
    /// The core produced audio that is ready for the host to consume.
    AudioReady,
    /// The core reached a debugger breakpoint.
    Breakpoint,
    /// The emulated machine halted or stopped executing.
    Halted,
    /// The core reached an implementation-defined scheduling boundary.
    Yielded,
}

/// Lifecycle and execution contract implemented by a concrete emulation core.
pub trait EmulationCore {
    /// Returns immutable metadata for this implementation.
    fn descriptor(&self) -> CoreDescriptor;

    /// Loads content, validating its structure before mutating machine state.
    fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError>;

    /// Resets the initialized machine to its documented reset state.
    fn reset(&mut self) -> Result<(), CoreError>;

    /// Executes until one observable event or scheduling boundary is reached.
    fn run_until_event(&mut self) -> Result<CoreEvent, CoreError>;

    /// Serializes a complete restorable state if save states are supported.
    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        Err(CoreError::Unsupported("save states"))
    }

    /// Restores a state only after validating its format and compatibility.
    fn load_state(&mut self, _state: &[u8]) -> Result<(), CoreError> {
        Err(CoreError::Unsupported("save states"))
    }

    /// Returns the current battery-backed game data when persistent cartridge saves are supported.
    ///
    /// This is distinct from a full machine save state: it contains only the
    /// data a real cartridge would retain when the console is powered off.
    fn save_game_data(&self) -> Result<Vec<u8>, CoreError> {
        Err(CoreError::Unsupported("persistent game saves"))
    }

    /// Restores battery-backed game data after validating its exact format and size.
    ///
    /// Implementations must validate the entire payload before changing
    /// persistent memory. This operation must not restore CPU or peripheral state.
    fn load_game_data(&mut self, _data: &[u8]) -> Result<(), CoreError> {
        Err(CoreError::Unsupported("persistent game saves"))
    }

    /// Reports whether persistent cartridge data changed since the last successful host save.
    fn persistent_save_dirty(&self) -> bool {
        false
    }

    /// Marks persistent cartridge data as durably saved by the host.
    ///
    /// The host calls this only after the write succeeds; implementations must
    /// not clear dirty state before persistence has completed.
    fn clear_persistent_save_dirty(&mut self) {}

    /// Sends a platform-neutral input transition to the emulated machine.
    ///
    /// Cores should map semantic controls to their own hardware and handle
    /// repeated or already-released transitions deterministically. The default
    /// implementation makes unsupported input explicit.
    fn send_input(&mut self, _event: crate::core_io::InputEvent) -> Result<(), CoreError> {
        Err(CoreError::Unsupported("input"))
    }

    /// Transfers ownership of the next complete video frame to the host.
    ///
    /// Returns `Ok(None)` when no frame is waiting. The host must not assume
    /// a frame exists merely because the core supports video output.
    fn take_video_frame(&mut self) -> Result<Option<crate::core_io::VideoFrame>, CoreError> {
        Err(CoreError::Unsupported("video output"))
    }

    /// Transfers ownership of the next audio buffer to the host.
    ///
    /// Returns `Ok(None)` when no audio is waiting. Buffers use the common
    /// PCM contract; the host remains responsible for playback and resampling.
    fn take_audio_buffer(&mut self) -> Result<Option<crate::core_io::AudioBuffer>, CoreError> {
        Err(CoreError::Unsupported("audio output"))
    }
}

/// Factory function used to construct a fresh instance of a registered core.
pub type CoreFactory = fn() -> Box<dyn EmulationCore>;

/// Error returned by the core registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    /// Format detection found no matching candidate.
    NoDetectedFormat,
    /// At least one probe needs more bytes, so the candidate set is incomplete.
    IncompleteDetection(usize),
    /// A detected format ID is not a valid canonical system identifier.
    InvalidDetectedFormat {
        /// ID returned by the format probe.
        format_id: &'static str,
        /// Reason the ID is not canonical.
        reason: IdentifierError,
    },
    /// Format detection found multiple candidates, so automatic selection is unsafe.
    AmbiguousDetectedFormats(usize),
    /// A core with this identifier is already registered.
    DuplicateSystem(SystemId),
    /// A factory returned a core whose identifier does not match its registration.
    DescriptorMismatch {
        /// Identifier used to select the factory.
        registered: SystemId,
        /// Identifier declared by the constructed core.
        actual: SystemId,
    },
    /// A factory returned metadata different from the descriptor registered by the host.
    DescriptorMetadataMismatch {
        /// Identifier whose registered descriptor was violated.
        registered: SystemId,
    },
    /// A core requires an incompatible host/core contract API version.
    IncompatibleApiVersion {
        /// Contract version required by the core.
        required: CoreApiVersion,
        /// Contract version supported by this host.
        supported: CoreApiVersion,
    },
    /// No core is registered for the requested identifier.
    UnknownSystem(SystemId),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDetectedFormat => formatter.write_str("content format was not detected"),
            Self::IncompleteDetection(count) => write!(
                formatter,
                "content format detection is incomplete: {count} probes need more data"
            ),
            Self::InvalidDetectedFormat { format_id, reason } => {
                write!(formatter, "detected format ID '{format_id}' is invalid: {reason}")
            }
            Self::AmbiguousDetectedFormats(count) => write!(
                formatter,
                "content format detection returned {count} candidates; explicit selection is required"
            ),
            Self::DuplicateSystem(id) => {
                write!(formatter, "system '{}' is already registered", id.as_str())
            }
            Self::DescriptorMismatch { registered, actual } => write!(
                formatter,
                "registered system '{}' produced core '{}'",
                registered.as_str(),
                actual.as_str()
            ),
            Self::DescriptorMetadataMismatch { registered } => write!(
                formatter,
                "registered system '{}' produced a core that differs from its registered descriptor",
                registered.as_str()
            ),
            Self::IncompatibleApiVersion {
                required,
                supported,
            } => write!(
                formatter,
                "core contract API {}.{} is incompatible with host API {}.{}",
                required.major, required.minor, supported.major, supported.minor
            ),
            Self::UnknownSystem(id) => {
                write!(formatter, "no core is registered for '{}'", id.as_str())
            }
        }
    }
}

impl std::error::Error for RegistryError {}

struct CoreRegistration {
    id: SystemId,
    factory: CoreFactory,
    descriptor: Option<CoreDescriptor>,
}

/// Small deterministic registry mapping stable system IDs to core factories.
///
/// Registration order is preserved. Factories are called only when a caller
/// requests an instance, and each request creates a new core.
#[derive(Default)]
pub struct CoreRegistry {
    registrations: Vec<CoreRegistration>,
}

impl CoreRegistry {
    /// Creates an empty registry.
    pub const fn new() -> Self {
        Self {
            registrations: Vec::new(),
        }
    }

    /// Registers a factory under a unique system identifier.
    pub fn register(&mut self, id: SystemId, factory: CoreFactory) -> Result<(), RegistryError> {
        if self.registrations.iter().any(|entry| entry.id == id) {
            return Err(RegistryError::DuplicateSystem(id));
        }
        self.registrations.push(CoreRegistration {
            id,
            factory,
            descriptor: None,
        });
        Ok(())
    }

    /// Registers a factory with host-visible metadata, validating compatibility before construction.
    ///
    /// This is the preferred entry point when the host needs to enumerate and
    /// verify capabilities without invoking the factory. The created instance
    /// is checked against the registered descriptor again when requested.
    pub fn register_with_descriptor(
        &mut self,
        descriptor: CoreDescriptor,
        factory: CoreFactory,
    ) -> Result<(), RegistryError> {
        let id = descriptor.system_id;
        if self.registrations.iter().any(|entry| entry.id == id) {
            return Err(RegistryError::DuplicateSystem(id));
        }
        let supported = CURRENT_CORE_API_VERSION;
        if !descriptor.api_version.is_compatible_with(supported) {
            return Err(RegistryError::IncompatibleApiVersion {
                required: descriptor.api_version,
                supported,
            });
        }
        self.registrations.push(CoreRegistration {
            id,
            factory,
            descriptor: Some(descriptor),
        });
        Ok(())
    }

    /// Returns a pre-registered descriptor without constructing a core.
    ///
    /// Returns None for unknown IDs and for legacy registrations made with
    /// register(), which do not provide metadata for preflight verification.
    pub fn registered_descriptor(&self, id: SystemId) -> Option<CoreDescriptor> {
        self.registrations
            .iter()
            .find(|entry| entry.id == id)
            .and_then(|entry| entry.descriptor)
    }

    /// Returns whether the registry contains the given system identifier.
    pub fn contains(&self, id: SystemId) -> bool {
        self.registrations.iter().any(|entry| entry.id == id)
    }

    /// Returns registered system identifiers in registration order.
    pub fn system_ids(&self) -> impl Iterator<Item = SystemId> + '_ {
        self.registrations.iter().map(|entry| entry.id)
    }

    /// Constructs a core from one unambiguous format-detection result.
    ///
    /// The detector's format ID must use the same stable identifier as the
    /// corresponding registered system. This only selects and constructs the
    /// core: callers must still load the content and handle format-specific
    /// validation errors before treating a session as launched.
    pub fn create_from_detection(
        &self,
        detection: &content::ContentDetection,
    ) -> Result<Box<dyn EmulationCore>, RegistryError> {
        if !detection.needs_more_data.is_empty() {
            return Err(RegistryError::IncompleteDetection(
                detection.needs_more_data.len(),
            ));
        }
        let id = match detection.matches.as_slice() {
            [] => return Err(RegistryError::NoDetectedFormat),
            [format_id] => SystemId::new(format_id)
                .map_err(|reason| RegistryError::InvalidDetectedFormat { format_id, reason })?,
            candidates => {
                return Err(RegistryError::AmbiguousDetectedFormats(candidates.len()));
            }
        };
        self.create(id)
    }

    /// Constructs a fresh core and verifies its descriptor matches its key.
    pub fn create(&self, id: SystemId) -> Result<Box<dyn EmulationCore>, RegistryError> {
        let registration = self
            .registrations
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(RegistryError::UnknownSystem(id))?;
        let core = (registration.factory)();
        let descriptor = core.descriptor();
        if descriptor.system_id != registration.id {
            return Err(RegistryError::DescriptorMismatch {
                registered: registration.id,
                actual: descriptor.system_id,
            });
        }
        if registration
            .descriptor
            .is_some_and(|registered| registered != descriptor)
        {
            return Err(RegistryError::DescriptorMetadataMismatch {
                registered: registration.id,
            });
        }
        let supported = CURRENT_CORE_API_VERSION;
        if !descriptor.api_version.is_compatible_with(supported) {
            return Err(RegistryError::IncompatibleApiVersion {
                required: descriptor.api_version,
                supported,
            });
        }
        Ok(core)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_system_ids() {
        assert_eq!(SystemId::new("gb").unwrap().as_str(), "gb");
        assert_eq!(
            SystemId::new("console-64-v2").unwrap().as_str(),
            "console-64-v2"
        );
    }

    #[test]
    fn rejects_empty_or_noncanonical_system_ids() {
        assert_eq!(SystemId::new(""), Err(IdentifierError::Empty));
        assert_eq!(SystemId::new("-gb"), Err(IdentifierError::InvalidSeparator));
        assert_eq!(SystemId::new("gb-"), Err(IdentifierError::InvalidSeparator));
        assert_eq!(
            SystemId::new("gb--color"),
            Err(IdentifierError::InvalidSeparator)
        );
        assert_eq!(
            SystemId::new("GameBoy"),
            Err(IdentifierError::InvalidCharacter)
        );
        assert_eq!(
            SystemId::new("gb_color"),
            Err(IdentifierError::InvalidCharacter)
        );
        assert_eq!(
            SystemId::new("gb/color"),
            Err(IdentifierError::InvalidCharacter)
        );
        assert_eq!(
            SystemId::new("gb color"),
            Err(IdentifierError::InvalidCharacter)
        );
        assert_eq!(
            SystemId::new("gb\n"),
            Err(IdentifierError::InvalidCharacter)
        );
        assert_eq!(
            SystemId::new("écran"),
            Err(IdentifierError::InvalidCharacter)
        );
    }

    #[test]
    fn capabilities_default_to_not_claiming_support() {
        assert_eq!(
            CoreCapabilities::default(),
            CoreCapabilities {
                content_loading: false,
                persistent_saves: false,
                save_states: false,
                video_output: false,
                audio_output: false,
                input: false,
            }
        );
    }

    struct RegistryTestCore {
        id: SystemId,
    }

    impl EmulationCore for RegistryTestCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: CURRENT_CORE_API_VERSION,
                system_id: self.id,
                display_name: "Registry test core",
                core_version: "0.0.0-test",
                capabilities: CoreCapabilities::default(),
            }
        }
        fn load_content(&mut self, _: &[u8]) -> Result<(), CoreError> {
            Err(CoreError::Unsupported("test core"))
        }
        fn reset(&mut self) -> Result<(), CoreError> {
            Err(CoreError::NotInitialized)
        }
        fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
            Err(CoreError::NotInitialized)
        }
    }

    fn make_example_core() -> Box<dyn EmulationCore> {
        Box::new(RegistryTestCore {
            id: SystemId::new("example").unwrap(),
        })
    }

    fn make_mismatched_core() -> Box<dyn EmulationCore> {
        Box::new(RegistryTestCore {
            id: SystemId::new("other").unwrap(),
        })
    }

    struct IncompatibleRegistryTestCore;

    impl EmulationCore for IncompatibleRegistryTestCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: CoreApiVersion { major: 2, minor: 0 },
                system_id: SystemId::new("incompatible").unwrap(),
                display_name: "Incompatible registry test core",
                core_version: "0.0.0-test",
                capabilities: CoreCapabilities::default(),
            }
        }
        fn load_content(&mut self, _: &[u8]) -> Result<(), CoreError> {
            Err(CoreError::Unsupported("test core"))
        }
        fn reset(&mut self) -> Result<(), CoreError> {
            Err(CoreError::NotInitialized)
        }
        fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
            Err(CoreError::NotInitialized)
        }
    }

    fn make_incompatible_core() -> Box<dyn EmulationCore> {
        Box::new(IncompatibleRegistryTestCore)
    }

    #[test]
    fn empty_registry_has_no_systems_and_rejects_queries() {
        let registry = CoreRegistry::new();
        let id = SystemId::new("example").unwrap();

        assert!(!registry.contains(id));
        assert_eq!(registry.system_ids().count(), 0);
        assert!(matches!(
            registry.create(id),
            Err(RegistryError::UnknownSystem(found)) if found == id
        ));
    }

    #[test]
    fn registry_creates_core_from_single_detected_candidate() {
        let id = SystemId::new("example").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_example_core).unwrap();
        let detection = content::ContentDetection {
            matches: vec!["example"],
            needs_more_data: vec![],
        };

        assert_eq!(
            registry
                .create_from_detection(&detection)
                .unwrap()
                .descriptor()
                .system_id,
            id
        );
    }

    #[test]
    fn registry_rejects_noncanonical_detected_format_ids_without_panicking() {
        let registry = CoreRegistry::new();
        let detection = content::ContentDetection {
            matches: vec!["Not Canonical"],
            needs_more_data: vec![],
        };
        assert!(matches!(
            registry.create_from_detection(&detection),
            Err(RegistryError::InvalidDetectedFormat {
                format_id: "Not Canonical",
                reason: IdentifierError::InvalidCharacter,
            })
        ));
    }

    #[test]
    fn registry_rejects_incomplete_detection_even_when_one_candidate_matches() {
        let id = SystemId::new("example").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_example_core).unwrap();
        let detection = content::ContentDetection {
            matches: vec!["example"],
            needs_more_data: vec![("possible-other-format", 16)],
        };

        assert!(matches!(
            registry.create_from_detection(&detection),
            Err(RegistryError::IncompleteDetection(1))
        ));
    }

    #[test]
    fn registry_rejects_missing_or_ambiguous_detection_without_guessing() {
        let id = SystemId::new("example").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_example_core).unwrap();

        assert!(matches!(
            registry.create_from_detection(&content::ContentDetection::default()),
            Err(RegistryError::NoDetectedFormat)
        ));
        let ambiguous = content::ContentDetection {
            matches: vec!["example", "other"],
            needs_more_data: vec![],
        };
        assert!(matches!(
            registry.create_from_detection(&ambiguous),
            Err(RegistryError::AmbiguousDetectedFormats(2))
        ));
    }

    #[test]
    fn registry_creates_cores_by_stable_id() {
        let id = SystemId::new("example").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_example_core).unwrap();

        assert!(registry.contains(id));
        assert_eq!(registry.system_ids().collect::<Vec<_>>(), vec![id]);
        assert_eq!(registry.create(id).unwrap().descriptor().system_id, id);
    }

    #[test]
    fn registry_rejects_duplicate_and_unknown_ids() {
        let id = SystemId::new("example").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_example_core).unwrap();
        assert_eq!(
            registry.register(id, make_example_core).unwrap_err(),
            RegistryError::DuplicateSystem(id)
        );
        let missing = SystemId::new("missing").unwrap();
        assert!(matches!(
            registry.create(missing),
            Err(RegistryError::UnknownSystem(found)) if found == missing
        ));
    }

    #[test]
    fn registry_preserves_registration_order_and_remains_unchanged_on_duplicate() {
        let first = SystemId::new("first").unwrap();
        let second = SystemId::new("second").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(first, make_example_core).unwrap();
        registry.register(second, make_example_core).unwrap();

        assert_eq!(
            registry.system_ids().collect::<Vec<_>>(),
            vec![first, second]
        );
        assert!(matches!(
            registry.register(first, make_mismatched_core),
            Err(RegistryError::DuplicateSystem(found)) if found == first
        ));
        assert_eq!(
            registry.system_ids().collect::<Vec<_>>(),
            vec![first, second]
        );
    }

    #[test]
    fn registry_invokes_factory_for_each_creation_request() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static FACTORY_CALLS: AtomicUsize = AtomicUsize::new(0);

        fn counted_factory() -> Box<dyn EmulationCore> {
            FACTORY_CALLS.fetch_add(1, Ordering::SeqCst);
            make_example_core()
        }

        let id = SystemId::new("example").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, counted_factory).unwrap();

        let before = FACTORY_CALLS.load(Ordering::SeqCst);
        let first = registry.create(id).unwrap();
        let second = registry.create(id).unwrap();
        let after = FACTORY_CALLS.load(Ordering::SeqCst);

        assert_eq!(first.descriptor().system_id, id);
        assert_eq!(second.descriptor().system_id, id);
        assert_eq!(after - before, 2);
    }

    #[test]
    fn registry_rejects_incompatible_api_versions_for_every_creation_path() {
        let id = SystemId::new("incompatible").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_incompatible_core).unwrap();

        let expected = RegistryError::IncompatibleApiVersion {
            required: CoreApiVersion { major: 2, minor: 0 },
            supported: CURRENT_CORE_API_VERSION,
        };
        assert_eq!(registry.create(id).err().unwrap(), expected);
        assert_eq!(
            registry
                .create_from_detection(&content::ContentDetection {
                    matches: vec!["incompatible"],
                    needs_more_data: vec![],
                })
                .err()
                .unwrap(),
            expected
        );
    }

    #[test]
    fn registry_rejects_factories_with_mismatched_descriptors() {
        let registered = SystemId::new("example").unwrap();
        let actual = SystemId::new("other").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(registered, make_mismatched_core).unwrap();
        assert!(matches!(
            registry.create(registered),
            Err(RegistryError::DescriptorMismatch { registered: found_registered, actual: found_actual })
                if found_registered == registered && found_actual == actual
        ));
    }

    #[test]
    fn registered_descriptor_is_available_without_invoking_factory() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static FACTORY_CALLS: AtomicUsize = AtomicUsize::new(0);

        fn counted_factory() -> Box<dyn EmulationCore> {
            FACTORY_CALLS.fetch_add(1, Ordering::SeqCst);
            make_example_core()
        }

        let id = SystemId::new("example").unwrap();
        let descriptor = RegistryTestCore { id }.descriptor();
        let mut registry = CoreRegistry::new();
        let before = FACTORY_CALLS.load(Ordering::SeqCst);

        registry
            .register_with_descriptor(descriptor, counted_factory)
            .unwrap();

        assert_eq!(registry.registered_descriptor(id), Some(descriptor));
        assert_eq!(FACTORY_CALLS.load(Ordering::SeqCst), before);
        assert_eq!(registry.system_ids().collect::<Vec<_>>(), vec![id]);
    }

    #[test]
    fn registry_rejects_incompatible_registered_descriptor_before_instantiation() {
        let descriptor = IncompatibleRegistryTestCore.descriptor();
        let id = descriptor.system_id;
        let mut registry = CoreRegistry::new();

        assert_eq!(
            registry
                .register_with_descriptor(descriptor, make_incompatible_core)
                .unwrap_err(),
            RegistryError::IncompatibleApiVersion {
                required: CoreApiVersion { major: 2, minor: 0 },
                supported: CURRENT_CORE_API_VERSION,
            }
        );
        assert!(!registry.contains(id));
        assert_eq!(registry.registered_descriptor(id), None);
    }

    #[test]
    fn registry_rejects_factory_metadata_that_differs_from_preflight_descriptor() {
        let id = SystemId::new("example").unwrap();
        let mut descriptor = RegistryTestCore { id }.descriptor();
        descriptor.display_name = "Different declared name";
        let mut registry = CoreRegistry::new();
        registry
            .register_with_descriptor(descriptor, make_example_core)
            .unwrap();

        assert_eq!(
            registry.create(id).err().unwrap(),
            RegistryError::DescriptorMetadataMismatch { registered: id }
        );
    }

    #[test]
    fn optional_host_io_methods_fail_explicitly_by_default() {
        let id = SystemId::new("example").unwrap();
        let mut core = RegistryTestCore { id };
        assert_eq!(
            core.send_input(crate::core_io::InputEvent {
                control: crate::core_io::InputControl::Action1,
                pressed: true,
            }),
            Err(CoreError::Unsupported("input"))
        );
        assert_eq!(
            core.take_video_frame(),
            Err(CoreError::Unsupported("video output"))
        );
        assert_eq!(
            core.take_audio_buffer(),
            Err(CoreError::Unsupported("audio output"))
        );
    }

    #[test]
    fn core_errors_have_stable_display_messages() {
        assert_eq!(
            CoreError::NotInitialized.to_string(),
            "core is not initialized"
        );
        assert_eq!(
            CoreError::InvalidContent("bad header").to_string(),
            "invalid content: bad header"
        );
        assert_eq!(
            CoreError::Unsupported("video").to_string(),
            "unsupported capability: video"
        );
        assert_eq!(
            CoreError::InvalidState("wrong version").to_string(),
            "invalid save state: wrong version"
        );
        assert_eq!(
            CoreError::Execution("bus fault").to_string(),
            "emulation execution failed: bus fault"
        );

        let gb = SystemId::new("gb").unwrap();
        let other = SystemId::new("other").unwrap();
        assert_eq!(
            RegistryError::NoDetectedFormat.to_string(),
            "content format was not detected"
        );
        assert_eq!(
            RegistryError::IncompleteDetection(1).to_string(),
            "content format detection is incomplete: 1 probes need more data"
        );
        assert_eq!(
            RegistryError::InvalidDetectedFormat {
                format_id: "Not Canonical",
                reason: IdentifierError::InvalidCharacter,
            }
            .to_string(),
            "detected format ID 'Not Canonical' is invalid: system identifier must contain only lowercase ASCII letters, digits, and hyphens"
        );
        assert_eq!(
            RegistryError::AmbiguousDetectedFormats(2).to_string(),
            "content format detection returned 2 candidates; explicit selection is required"
        );
        assert_eq!(
            RegistryError::DuplicateSystem(gb).to_string(),
            "system 'gb' is already registered"
        );
        assert_eq!(
            RegistryError::DescriptorMismatch {
                registered: gb,
                actual: other
            }
            .to_string(),
            "registered system 'gb' produced core 'other'"
        );
        assert_eq!(
            RegistryError::UnknownSystem(gb).to_string(),
            "no core is registered for 'gb'"
        );

        assert_eq!(
            IdentifierError::Empty.to_string(),
            "system identifier cannot be empty"
        );
        assert_eq!(
            IdentifierError::InvalidCharacter.to_string(),
            "system identifier must contain only lowercase ASCII letters, digits, and hyphens"
        );
        assert_eq!(
            IdentifierError::InvalidSeparator.to_string(),
            "system identifier cannot start or end with a hyphen or contain adjacent hyphens"
        );
    }

    #[test]
    fn unsupported_save_states_fail_explicitly() {
        struct MetadataOnlyCore;
        impl EmulationCore for MetadataOnlyCore {
            fn descriptor(&self) -> CoreDescriptor {
                CoreDescriptor {
                    api_version: CURRENT_CORE_API_VERSION,
                    system_id: SystemId::new("example").unwrap(),
                    display_name: "Example (contract test only)",
                    core_version: "0.0.0-test",
                    capabilities: CoreCapabilities::default(),
                }
            }
            fn load_content(&mut self, _: &[u8]) -> Result<(), CoreError> {
                Err(CoreError::Unsupported("content loading"))
            }
            fn reset(&mut self) -> Result<(), CoreError> {
                Err(CoreError::NotInitialized)
            }
            fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
                Err(CoreError::NotInitialized)
            }
        }

        let mut core = MetadataOnlyCore;
        assert_eq!(
            core.save_state(),
            Err(CoreError::Unsupported("save states"))
        );
        assert_eq!(
            core.save_game_data(),
            Err(CoreError::Unsupported("persistent game saves"))
        );
        assert_eq!(
            core.load_game_data(&[1, 2, 3]),
            Err(CoreError::Unsupported("persistent game saves"))
        );
        assert_eq!(
            core.load_state(&[0, 1, 2]),
            Err(CoreError::Unsupported("save states"))
        );
        assert!(!core.descriptor().capabilities.save_states);
    }
}

#[cfg(test)]
mod core_instance_isolation_tests {
    use super::*;

    struct StatefulIsolationCore {
        system_id: &'static str,
        display_name: &'static str,
        loaded: bool,
    }

    impl EmulationCore for StatefulIsolationCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: CURRENT_CORE_API_VERSION,
                system_id: SystemId::new(self.system_id).unwrap(),
                display_name: self.display_name,
                core_version: "0.0.0-test",
                capabilities: CoreCapabilities {
                    content_loading: true,
                    ..CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
            if content.is_empty() {
                return Err(CoreError::InvalidContent("empty isolation test content"));
            }
            self.loaded = true;
            Ok(())
        }

        fn reset(&mut self) -> Result<(), CoreError> {
            if !self.loaded {
                return Err(CoreError::NotInitialized);
            }
            Ok(())
        }

        fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
            if !self.loaded {
                return Err(CoreError::NotInitialized);
            }
            Ok(CoreEvent::Yielded)
        }
    }

    fn make_first_isolation_module() -> Box<dyn EmulationCore> {
        Box::new(StatefulIsolationCore {
            system_id: "isolation-test",
            display_name: "First fake isolation module",
            loaded: false,
        })
    }

    fn make_second_isolation_module() -> Box<dyn EmulationCore> {
        Box::new(StatefulIsolationCore {
            system_id: "isolation-test-two",
            display_name: "Second fake isolation module",
            loaded: false,
        })
    }

    #[test]
    fn distinct_fake_modules_do_not_share_mutable_machine_state() {
        let first_id = SystemId::new("isolation-test").unwrap();
        let second_id = SystemId::new("isolation-test-two").unwrap();
        let mut registry = CoreRegistry::new();
        registry
            .register(first_id, make_first_isolation_module)
            .unwrap();
        registry
            .register(second_id, make_second_isolation_module)
            .unwrap();

        let mut first = registry.create(first_id).unwrap();
        let mut second = registry.create(second_id).unwrap();

        assert_eq!(first.descriptor().system_id, first_id);
        assert_eq!(second.descriptor().system_id, second_id);
        first.load_content(&[1]).unwrap();
        assert_eq!(first.run_until_event(), Ok(CoreEvent::Yielded));
        assert_eq!(second.run_until_event(), Err(CoreError::NotInitialized));

        second.load_content(&[2]).unwrap();
        assert_eq!(second.run_until_event(), Ok(CoreEvent::Yielded));
        assert_eq!(first.run_until_event(), Ok(CoreEvent::Yielded));
    }
}
