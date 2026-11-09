#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Bounded host-side ownership of independent emulation sessions.

use core::fmt;
use std::collections::BTreeMap;

use crate::content::{ContentFormatProbe, ContentLimits};
use crate::runtime::{
    launch_content, launch_content_path as launch_path, EmulationSession, LaunchError,
};
use crate::{CoreRegistry, EmulationCore};

/// Stable host-assigned identity for one session.
///
/// This identity is independent from a system ID or content identity. IDs are
/// monotonically assigned and are not reused after a session is closed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SessionId(u64);

impl SessionId {
    /// Returns the numeric identifier assigned by the host.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Limits for a host runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostRuntimeConfig {
    /// Maximum number of simultaneously open sessions.
    pub max_sessions: usize,
}

impl Default for HostRuntimeConfig {
    fn default() -> Self {
        Self { max_sessions: 4 }
    }
}

/// Errors returned by the host session manager.
#[derive(Debug)]
pub enum HostRuntimeError {
    /// A zero-session configuration is invalid.
    InvalidConfiguration,
    /// The configured number of active sessions has been reached.
    SessionLimitReached {
        /// Maximum number of active sessions.
        limit: usize,
    },
    /// The monotonic session identifier space has been exhausted.
    SessionIdExhausted,
    /// The requested session does not exist.
    UnknownSession(SessionId),
    /// Content could not be detected, validated, or loaded.
    Launch(LaunchError),
}

impl fmt::Display for HostRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration => f.write_str("max_sessions must be greater than zero"),
            Self::SessionLimitReached { limit } => {
                write!(f, "host session limit reached ({limit})")
            }
            Self::SessionIdExhausted => f.write_str("host session identifier space exhausted"),
            Self::UnknownSession(id) => write!(f, "unknown host session {}", id.get()),
            Self::Launch(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for HostRuntimeError {}

impl From<LaunchError> for HostRuntimeError {
    fn from(error: LaunchError) -> Self {
        Self::Launch(error)
    }
}

/// Owns multiple independent sessions and applies a strict resource policy.
///
/// This manager does not spawn threads: mutation requires exclusive access through
/// `&mut self`, so hosts must serialize lifecycle operations. The session limit
/// is enforced before constructing a core or reading content. Each successful
/// launch creates a fresh core through the registry and receives a unique host ID.
pub struct HostRuntime {
    config: HostRuntimeConfig,
    next_id: u64,
    sessions: BTreeMap<SessionId, EmulationSession>,
}

impl HostRuntime {
    /// Creates an empty host runtime with validated resource limits.
    pub fn new(config: HostRuntimeConfig) -> Result<Self, HostRuntimeError> {
        if config.max_sessions == 0 {
            return Err(HostRuntimeError::InvalidConfiguration);
        }
        Ok(Self {
            config,
            next_id: 1,
            sessions: BTreeMap::new(),
        })
    }

    /// Returns the configured maximum number of simultaneous sessions.
    pub const fn max_sessions(&self) -> usize {
        self.config.max_sessions
    }

    /// Returns the number of currently open sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Returns true when no sessions are currently open.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Returns the IDs of open sessions in stable ascending order.
    pub fn session_ids(&self) -> Vec<SessionId> {
        self.sessions.keys().copied().collect()
    }

    fn preflight_slot(&self) -> Result<(), HostRuntimeError> {
        if self.sessions.len() >= self.config.max_sessions {
            return Err(HostRuntimeError::SessionLimitReached {
                limit: self.config.max_sessions,
            });
        }
        if self.next_id == 0 {
            return Err(HostRuntimeError::SessionIdExhausted);
        }
        Ok(())
    }

    fn insert(&mut self, session: EmulationSession) -> Result<SessionId, HostRuntimeError> {
        self.preflight_slot()?;
        let id = SessionId(self.next_id);
        self.next_id = self.next_id.checked_add(1).unwrap_or(0);
        self.sessions.insert(id, session);
        Ok(id)
    }

    /// Creates an empty session around a caller-provided core.
    ///
    /// Useful when the host manages core selection itself. Prefer
    /// `launch_content` when using the common content-detection pipeline.
    pub fn create_session(
        &mut self,
        core: Box<dyn EmulationCore>,
    ) -> Result<SessionId, HostRuntimeError> {
        self.preflight_slot()?;
        self.insert(EmulationSession::new(core))
    }

    /// Detects, validates, and launches content, then registers the running session.
    ///
    /// Capacity and ID exhaustion are checked before content processing or core
    /// construction. A failed launch never leaves a partially registered session.
    pub fn launch_content(
        &mut self,
        bytes: &[u8],
        limits: ContentLimits,
        probes: &[&dyn ContentFormatProbe],
        registry: &CoreRegistry,
    ) -> Result<SessionId, HostRuntimeError> {
        self.preflight_slot()?;
        let session = launch_content(bytes, limits, probes, registry)?;
        self.insert(session)
    }

    /// Reads, detects, validates, and launches content from a path under the common size limit.
    ///
    /// The session capacity is checked before touching the filesystem. The shared
    /// path loader performs an early metadata check and a bounded read, then uses
    /// the same format detection and core validation as byte-based launches.
    pub fn launch_content_path(
        &mut self,
        path: impl AsRef<std::path::Path>,
        limits: ContentLimits,
        probes: &[&dyn ContentFormatProbe],
        registry: &CoreRegistry,
    ) -> Result<SessionId, HostRuntimeError> {
        self.preflight_slot()?;
        let session = launch_path(path, limits, probes, registry)?;
        self.insert(session)
    }

    /// Borrows a session by its host-assigned ID.
    pub fn session(&self, id: SessionId) -> Result<&EmulationSession, HostRuntimeError> {
        self.sessions
            .get(&id)
            .ok_or(HostRuntimeError::UnknownSession(id))
    }

    /// Mutably borrows a session by its host-assigned ID.
    pub fn session_mut(
        &mut self,
        id: SessionId,
    ) -> Result<&mut EmulationSession, HostRuntimeError> {
        self.sessions
            .get_mut(&id)
            .ok_or(HostRuntimeError::UnknownSession(id))
    }

    /// Closes a session and releases the host's ownership of its core.
    ///
    /// Closing does not reuse its ID. Returns an error when the ID is unknown.
    pub fn close_session(&mut self, id: SessionId) -> Result<EmulationSession, HostRuntimeError> {
        self.sessions
            .remove(&id)
            .ok_or(HostRuntimeError::UnknownSession(id))
    }

    /// Closes every open session and releases ownership of all their cores.
    ///
    /// Returns the number of sessions closed. The host's ID counter is preserved,
    /// so sessions created after shutdown receive fresh IDs rather than reusing
    /// identities that may still be referenced by frontend state.
    pub fn close_all_sessions(&mut self) -> usize {
        let closed = self.sessions.len();
        self.sessions.clear();
        closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::FormatProbeResult;
    use crate::{CoreApiVersion, CoreCapabilities, CoreDescriptor, CoreError, CoreEvent, SystemId};

    struct MatchProbe;

    impl ContentFormatProbe for MatchProbe {
        fn format_id(&self) -> &'static str {
            "test-system"
        }

        fn probe(&self, prefix: &[u8]) -> FormatProbeResult {
            if prefix.starts_with(b"TEST") {
                FormatProbeResult::Match
            } else {
                FormatProbeResult::NoMatch
            }
        }
    }

    fn make_mock_core() -> Box<dyn EmulationCore> {
        Box::new(MockCore)
    }

    struct MockCore;

    impl EmulationCore for MockCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: CoreApiVersion { major: 1, minor: 0 },
                system_id: SystemId::new("test-system").unwrap(),
                display_name: "Test system",
                core_version: "0.1.0",
                capabilities: CoreCapabilities {
                    content_loading: true,
                    ..CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, _: &[u8]) -> Result<(), CoreError> {
            Ok(())
        }

        fn reset(&mut self) -> Result<(), CoreError> {
            Ok(())
        }

        fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
            Ok(CoreEvent::Yielded)
        }
    }

    #[test]
    fn enforces_capacity_and_does_not_reuse_closed_ids() {
        let mut host = HostRuntime::new(HostRuntimeConfig { max_sessions: 2 }).unwrap();
        let first = host.create_session(Box::new(MockCore)).unwrap();
        let second = host.create_session(Box::new(MockCore)).unwrap();
        assert_ne!(first, second);
        assert_eq!(host.session_count(), 2);
        assert!(matches!(
            host.create_session(Box::new(MockCore)),
            Err(HostRuntimeError::SessionLimitReached { limit: 2 })
        ));
        host.close_session(first).unwrap();
        let third = host.create_session(Box::new(MockCore)).unwrap();
        assert!(third.get() > second.get());
        assert_eq!(host.session_ids(), vec![second, third]);
    }

    #[test]
    fn sessions_keep_independent_lifecycle_state() {
        let mut host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();
        let first = host.create_session(Box::new(MockCore)).unwrap();
        let second = host.create_session(Box::new(MockCore)).unwrap();
        host.session_mut(first)
            .unwrap()
            .load_content(b"first")
            .unwrap();
        assert_eq!(
            host.session(first).unwrap().state(),
            crate::runtime::SessionState::Running
        );
        assert_eq!(
            host.session(second).unwrap().state(),
            crate::runtime::SessionState::Empty
        );
    }

    #[test]
    fn successful_content_launch_registers_one_running_session() {
        let mut host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();
        let registry = {
            let mut registry = CoreRegistry::new();
            registry
                .register(SystemId::new("test-system").unwrap(), make_mock_core)
                .unwrap();
            registry
        };
        let probe = MatchProbe;
        let id = host
            .launch_content(b"TEST-rom", ContentLimits::new(1024), &[&probe], &registry)
            .unwrap();

        assert_eq!(host.session_count(), 1);
        assert_eq!(
            host.session(id).unwrap().state(),
            crate::runtime::SessionState::Running
        );
        assert_eq!(
            host.session(id).unwrap().descriptor().system_id.as_str(),
            "test-system"
        );
    }

    #[test]
    fn path_launch_uses_the_bounded_content_pipeline() {
        use std::fs;
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_PATH: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "host-app-host-launch-{}-{}.rom",
            std::process::id(),
            NEXT_PATH.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, b"TEST-rom").unwrap();

        let mut host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();
        let mut registry = CoreRegistry::new();
        registry
            .register(SystemId::new("test-system").unwrap(), make_mock_core)
            .unwrap();
        let probe = MatchProbe;
        let id = host
            .launch_content_path(&path, ContentLimits::new(1024), &[&probe], &registry)
            .unwrap();

        assert_eq!(host.session_count(), 1);
        assert_eq!(
            host.session(id).unwrap().state(),
            crate::runtime::SessionState::Running
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_content_launch_does_not_register_a_partial_session() {
        let mut host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();
        let registry = CoreRegistry::new();
        let result = host.launch_content(&[], ContentLimits::new(1024), &[], &registry);

        assert!(matches!(
            result,
            Err(HostRuntimeError::Launch(LaunchError::Content(
                crate::content::ContentError::Empty
            )))
        ));
        assert!(host.is_empty());
        assert_eq!(host.session_count(), 0);
    }

    #[test]
    fn closing_twice_reports_unknown_session_without_affecting_other_sessions() {
        let mut host = HostRuntime::new(HostRuntimeConfig { max_sessions: 2 }).unwrap();
        let closed = host.create_session(Box::new(MockCore)).unwrap();
        let remaining = host.create_session(Box::new(MockCore)).unwrap();

        host.close_session(closed).unwrap();
        assert!(matches!(
            host.close_session(closed),
            Err(HostRuntimeError::UnknownSession(id)) if id == closed
        ));
        assert_eq!(host.session_ids(), vec![remaining]);
        assert_eq!(host.session_count(), 1);
    }

    #[test]
    fn exhausted_session_ids_never_wrap_or_create_a_partial_session() {
        let mut host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();
        host.next_id = u64::MAX;
        let final_id = host.create_session(Box::new(MockCore)).unwrap();
        assert_eq!(final_id.get(), u64::MAX);
        host.close_session(final_id).unwrap();

        assert!(matches!(
            host.create_session(Box::new(MockCore)),
            Err(HostRuntimeError::SessionIdExhausted)
        ));
        assert!(host.is_empty());
    }

    #[test]
    fn close_all_sessions_releases_every_core_without_reusing_ids() {
        let mut host = HostRuntime::new(HostRuntimeConfig { max_sessions: 3 }).unwrap();
        let first = host.create_session(Box::new(MockCore)).unwrap();
        let second = host.create_session(Box::new(MockCore)).unwrap();

        assert_eq!(host.close_all_sessions(), 2);
        assert!(host.is_empty());
        assert_eq!(host.session_count(), 0);
        assert!(matches!(
            host.session(first),
            Err(HostRuntimeError::UnknownSession(id)) if id == first
        ));
        assert!(matches!(
            host.session(second),
            Err(HostRuntimeError::UnknownSession(id)) if id == second
        ));

        let third = host.create_session(Box::new(MockCore)).unwrap();
        assert!(third.get() > second.get());
        assert_eq!(host.session_ids(), vec![third]);
        assert_eq!(host.close_all_sessions(), 1);
        assert_eq!(host.close_all_sessions(), 0);
    }

    struct P1FlowCore {
        ticks: u8,
    }

    impl EmulationCore for P1FlowCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: crate::CURRENT_CORE_API_VERSION,
                system_id: SystemId::new("test-system").unwrap(),
                display_name: "P1 flow test core",
                core_version: "1.0.0-test",
                capabilities: CoreCapabilities {
                    content_loading: true,
                    input: true,
                    video_output: true,
                    audio_output: true,
                    save_states: true,
                    ..CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
            if !content.starts_with(b"TEST") {
                return Err(CoreError::InvalidContent("invalid P1 flow test image"));
            }
            if content == b"TEST-REJECT" {
                return Err(CoreError::InvalidContent(
                    "deliberately rejected P1 test image",
                ));
            }
            self.ticks = 7;
            Ok(())
        }

        fn reset(&mut self) -> Result<(), CoreError> {
            self.ticks = 0;
            Ok(())
        }

        fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
            self.ticks = self.ticks.saturating_add(1);
            Ok(CoreEvent::Yielded)
        }

        fn save_state(&self) -> Result<Vec<u8>, CoreError> {
            Ok(vec![self.ticks])
        }

        fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError> {
            if state.len() != 1 {
                return Err(CoreError::InvalidState("expected one-byte P1 test state"));
            }
            self.ticks = state[0];
            Ok(())
        }

        fn send_input(&mut self, _: crate::core_io::InputEvent) -> Result<(), CoreError> {
            Ok(())
        }

        fn take_video_frame(&mut self) -> Result<Option<crate::core_io::VideoFrame>, CoreError> {
            let frame = crate::core_io::VideoFrame::new(
                1,
                1,
                crate::core_io::PixelFormat::Rgba8888,
                vec![255, 0, 0, 255],
            )
            .expect("test frame is valid");
            Ok(Some(frame))
        }

        fn take_audio_buffer(&mut self) -> Result<Option<crate::core_io::AudioBuffer>, CoreError> {
            let audio = crate::core_io::AudioBuffer::new(48_000, 2, vec![100, -100])
                .expect("test audio is valid");
            Ok(Some(audio))
        }
    }

    fn make_p1_flow_core() -> Box<dyn EmulationCore> {
        Box::new(P1FlowCore { ticks: 0 })
    }

    #[test]
    fn rejected_core_content_never_leaves_a_registered_session() {
        let mut registry = CoreRegistry::new();
        registry
            .register(SystemId::new("test-system").unwrap(), make_p1_flow_core)
            .unwrap();
        let probe = MatchProbe;
        let mut host = HostRuntime::new(HostRuntimeConfig { max_sessions: 2 }).unwrap();

        let result = host.launch_content(
            b"TEST-REJECT",
            ContentLimits::new(1024),
            &[&probe],
            &registry,
        );

        assert!(matches!(
            result,
            Err(HostRuntimeError::Launch(LaunchError::Session(_)))
        ));
        assert!(host.is_empty());
        assert_eq!(host.session_count(), 0);
    }

    #[test]
    fn p1_runtime_flow_launches_routes_io_media_saves_restores_and_closes() {
        use crate::core_io::{InputControl, InputEvent};
        use crate::runtime::SessionState;

        let mut registry = CoreRegistry::new();
        registry
            .register(SystemId::new("test-system").unwrap(), make_p1_flow_core)
            .unwrap();
        let probe = MatchProbe;
        let mut host = HostRuntime::new(HostRuntimeConfig { max_sessions: 2 }).unwrap();

        // Detect and validate the selected test module before registering a session.
        let id = host
            .launch_content(
                b"TEST-rom-image",
                ContentLimits::new(1024),
                &[&probe],
                &registry,
            )
            .unwrap();
        assert_eq!(host.session(id).unwrap().state(), SessionState::Running);
        assert_eq!(
            host.session(id).unwrap().descriptor().system_id.as_str(),
            "test-system"
        );

        // Deliver a semantic input transition and retrieve owned media payloads.
        host.session_mut(id)
            .unwrap()
            .send_input(InputEvent {
                control: InputControl::Action1,
                pressed: true,
            })
            .unwrap();
        assert!(host
            .session(id)
            .unwrap()
            .is_input_pressed(InputControl::Action1));

        let frame = host
            .session_mut(id)
            .unwrap()
            .take_video_frame()
            .unwrap()
            .unwrap();
        assert_eq!(
            (frame.width(), frame.height(), frame.pixels()),
            (1, 1, &[255, 0, 0, 255][..])
        );
        let audio = host
            .session_mut(id)
            .unwrap()
            .take_audio_buffer()
            .unwrap()
            .unwrap();
        assert_eq!(
            (audio.sample_rate(), audio.channels(), audio.samples()),
            (48_000, 2, &[100, -100][..])
        );

        // The versioned envelope restores the captured core state after execution advances.
        let checkpoint = host.session_mut(id).unwrap().save_state_envelope().unwrap();
        host.session_mut(id).unwrap().run_for_events(1).unwrap();
        assert_eq!(host.session_mut(id).unwrap().save_state().unwrap(), vec![8]);
        host.session_mut(id)
            .unwrap()
            .load_state_envelope(&checkpoint)
            .unwrap();
        assert_eq!(host.session_mut(id).unwrap().save_state().unwrap(), vec![7]);

        // Shutdown must release held controls before the host drops the session.
        host.session_mut(id).unwrap().release_all_inputs().unwrap();
        assert!(!host
            .session(id)
            .unwrap()
            .is_input_pressed(InputControl::Action1));
        host.session_mut(id).unwrap().stop().unwrap();
        assert_eq!(host.session(id).unwrap().state(), SessionState::Stopped);

        let closed = host.close_session(id).unwrap();
        assert_eq!(closed.state(), SessionState::Stopped);
        assert!(!closed.is_input_pressed(InputControl::Action1));
        assert!(host.is_empty());
        assert!(
            matches!(host.session(id), Err(HostRuntimeError::UnknownSession(found)) if found == id)
        );
    }

    #[test]
    fn full_host_rejects_path_launch_before_touching_the_filesystem() {
        let mut host = HostRuntime::new(HostRuntimeConfig { max_sessions: 1 }).unwrap();
        let _occupied = host.create_session(Box::new(MockCore)).unwrap();
        let registry = CoreRegistry::new();
        let probe = MatchProbe;

        // A full host must report capacity exhaustion deterministically, even if
        // the supplied path does not exist. This keeps filesystem errors from
        // obscuring the actionable lifecycle error and avoids needless I/O.
        let missing_path = std::env::temp_dir().join(format!(
            "host-app-must-not-read-{}-missing.rom",
            std::process::id()
        ));
        let result = host.launch_content_path(
            &missing_path,
            ContentLimits::new(1024),
            &[&probe],
            &registry,
        );

        assert!(matches!(
            result,
            Err(HostRuntimeError::SessionLimitReached { limit: 1 })
        ));
        assert_eq!(host.session_count(), 1);
    }

    #[test]
    fn rejects_zero_capacity_and_unknown_ids() {
        assert!(matches!(
            HostRuntime::new(HostRuntimeConfig { max_sessions: 0 }),
            Err(HostRuntimeError::InvalidConfiguration)
        ));
        let host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();
        assert!(matches!(
            host.session(SessionId(42)),
            Err(HostRuntimeError::UnknownSession(SessionId(42)))
        ));
    }
}
