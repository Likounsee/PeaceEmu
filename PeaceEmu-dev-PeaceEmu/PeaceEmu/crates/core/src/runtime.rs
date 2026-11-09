#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Console-independent lifecycle controller for a single emulation session.

use core::fmt;
use std::io::Read;
use std::path::Path;

use crate::content::{
    detect_content_formats, ContentDetection, ContentError, ContentFormatProbe, ContentImage,
    ContentLimits,
};
use crate::core_io::{
    AudioBuffer, CadenceAdvance, FrameCadence, InputControl, InputEvent, InputState, VideoFrame,
};
use crate::save_state::{SaveStateEnvelope, SaveStateError};
use crate::{CoreDescriptor, CoreError, CoreEvent, CoreRegistry, EmulationCore, RegistryError};

/// Failure while launching a session from untrusted content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchError {
    /// Common size or emptiness checks rejected the content.
    Content(ContentError),
    /// Format detection could not select exactly one registered core.
    Registry(RegistryError),
    /// The selected core rejected the content during format-specific validation.
    Session(SessionError),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Content(error) => fmt::Display::fmt(error, formatter),
            Self::Registry(error) => fmt::Display::fmt(error, formatter),
            Self::Session(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for LaunchError {}

/// Detects a content format, constructs its registered core, and loads a bounded image.
///
/// The full input is first copied into a bounded owned image. Probes inspect only
/// those bounded bytes. The registry rejects absent, ambiguous, invalid, or
/// incomplete detections; finally, the selected core must accept the image
/// before this function returns a running session.
pub fn launch_content(
    bytes: &[u8],
    limits: ContentLimits,
    probes: &[&dyn ContentFormatProbe],
    registry: &CoreRegistry,
) -> Result<EmulationSession, LaunchError> {
    let image = ContentImage::from_slice(bytes, limits).map_err(LaunchError::Content)?;
    launch_content_image(image, probes, registry)
}

/// Reads a content file with a strict size limit, detects its format and launches it.
///
/// File metadata is used for early rejection, but the actual read remains bounded
/// so stale metadata or a file that grows during reading cannot bypass the limit.
pub fn launch_content_path(
    path: impl AsRef<Path>,
    limits: ContentLimits,
    probes: &[&dyn ContentFormatProbe],
    registry: &CoreRegistry,
) -> Result<EmulationSession, LaunchError> {
    let image = ContentImage::read_path(path, limits).map_err(LaunchError::Content)?;
    launch_content_image(image, probes, registry)
}

/// Reads content from a stream with a strict size limit before format detection.
///
/// This is suitable for host-provided streams such as archive entries or downloaded
/// payloads. The stream is consumed only up to the configured limit plus one byte.
pub fn launch_content_reader<R: Read>(
    reader: R,
    limits: ContentLimits,
    probes: &[&dyn ContentFormatProbe],
    registry: &CoreRegistry,
) -> Result<EmulationSession, LaunchError> {
    let image = ContentImage::read_from(reader, limits).map_err(LaunchError::Content)?;
    launch_content_image(image, probes, registry)
}

fn launch_content_image(
    image: ContentImage,
    probes: &[&dyn ContentFormatProbe],
    registry: &CoreRegistry,
) -> Result<EmulationSession, LaunchError> {
    let detection: ContentDetection = detect_content_formats(image.as_bytes(), probes);
    let core = registry
        .create_from_detection(&detection)
        .map_err(LaunchError::Registry)?;
    let mut session = EmulationSession::new(core);
    session
        .load_validated_content(&image)
        .map_err(LaunchError::Session)?;
    Ok(session)
}

/// Observable lifecycle state of an emulation session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    /// No content has been loaded successfully.
    Empty,
    /// Content is loaded and execution is enabled.
    Running,
    /// Content is loaded but execution is paused.
    Paused,
    /// The core reported that the emulated machine halted.
    Halted,
    /// Execution was explicitly stopped by the host; loaded content is retained.
    Stopped,
}

/// An invalid lifecycle operation or an error reported by a core.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    /// The operation is not valid in the current session state.
    InvalidTransition {
        /// Name of the requested operation.
        operation: &'static str,
        /// Current lifecycle state.
        state: SessionState,
    },
    /// Common content validation failed before the core was called.
    Content(ContentError),
    /// The core rejected the requested operation.
    Core(CoreError),
    /// Save-state envelope validation or compatibility failed.
    SaveState(SaveStateError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTransition { operation, state } => {
                write!(formatter, "cannot {operation} while session is {state:?}")
            }
            Self::Content(error) => fmt::Display::fmt(error, formatter),
            Self::Core(error) => fmt::Display::fmt(error, formatter),
            Self::SaveState(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<ContentError> for SessionError {
    fn from(error: ContentError) -> Self {
        Self::Content(error)
    }
}

impl From<CoreError> for SessionError {
    fn from(error: CoreError) -> Self {
        Self::Core(error)
    }
}

impl From<SaveStateError> for SessionError {
    fn from(error: SaveStateError) -> Self {
        Self::SaveState(error)
    }
}

/// Summary of a bounded run request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionRunReport {
    /// Number of core execution calls completed by this request.
    pub events_processed: usize,
    /// Most recent event, if at least one execution call completed.
    pub last_event: Option<crate::CoreEvent>,
    /// Whether the request stopped because its execution-call budget was reached.
    ///
    /// This is also true for a zero budget. It does not imply that the core
    /// reached a particular amount of emulated time.
    pub budget_exhausted: bool,
}

/// Owns one core and enforces consistent host-facing lifecycle transitions.
///
/// This type deliberately does not schedule wall-clock time, create threads,
/// render frames, or play audio. It provides a deterministic lifecycle boundary
/// that a future application loop can drive.
pub struct EmulationSession {
    core: Box<dyn EmulationCore>,
    state: SessionState,
    input_state: InputState,
    last_core_error: Option<CoreError>,
}

impl EmulationSession {
    /// Wraps a fresh core in an initially empty session.
    pub fn new(core: Box<dyn EmulationCore>) -> Self {
        Self {
            core,
            state: SessionState::Empty,
            input_state: InputState::new(),
            last_core_error: None,
        }
    }

    /// Returns the most recent error reported by a mutable core operation.
    ///
    /// Successful operations do not erase this diagnostic; a later core error
    /// replaces it. Use `clear_last_core_error` after the host has recorded or
    /// handled the failure.
    pub fn last_core_error(&self) -> Option<CoreError> {
        self.last_core_error.clone()
    }

    /// Clears the retained core-error diagnostic without changing session state.
    pub fn clear_last_core_error(&mut self) {
        self.last_core_error = None;
    }

    fn record_core_result<T>(&mut self, result: Result<T, CoreError>) -> Result<T, SessionError> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                self.last_core_error = Some(error.clone());
                Err(SessionError::Core(error))
            }
        }
    }

    /// Returns the core's immutable descriptor.
    pub fn descriptor(&self) -> CoreDescriptor {
        self.core.descriptor()
    }

    /// Returns the current session lifecycle state.
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Loads content and starts execution if the core accepts it.
    ///
    /// On failure, the session state is left unchanged. Core implementations
    /// are required to validate content before mutating machine state.
    pub fn load_content(&mut self, content: &[u8]) -> Result<(), SessionError> {
        if !self.core.descriptor().capabilities.content_loading {
            return Err(SessionError::Core(CoreError::Unsupported(
                "content loading",
            )));
        }
        let result = self.core.load_content(content);
        self.record_core_result(result)?;
        self.input_state = InputState::new();
        self.state = SessionState::Running;
        Ok(())
    }

    /// Validates borrowed content against common limits before allocating a copy
    /// or invoking the core.
    ///
    /// If validation fails, neither the core nor the session state is changed.
    pub fn load_content_bounded(
        &mut self,
        content: &[u8],
        limits: ContentLimits,
    ) -> Result<(), SessionError> {
        if !self.core.descriptor().capabilities.content_loading {
            return Err(SessionError::Core(CoreError::Unsupported(
                "content loading",
            )));
        }
        let image = ContentImage::from_slice(content, limits)?;
        self.load_validated_content(&image)
    }

    /// Loads a payload that has passed the framework's common size checks.
    ///
    /// The core still validates the content format and system compatibility.
    /// On failure, the session lifecycle and host input state remain unchanged.
    pub fn load_validated_content(
        &mut self,
        content: &crate::content::ContentImage,
    ) -> Result<(), SessionError> {
        self.load_content(content.as_bytes())
    }

    /// Pauses a running session without discarding machine state.
    pub fn pause(&mut self) -> Result<(), SessionError> {
        self.require_state("pause", SessionState::Running)?;
        self.state = SessionState::Paused;
        Ok(())
    }

    /// Resumes a paused session.
    pub fn resume(&mut self) -> Result<(), SessionError> {
        self.require_state("resume", SessionState::Paused)?;
        self.state = SessionState::Running;
        Ok(())
    }

    /// Resets a loaded machine and resumes execution.
    pub fn reset(&mut self) -> Result<(), SessionError> {
        if self.state == SessionState::Empty {
            return Err(self.invalid_transition("reset"));
        }
        let result = self.core.reset();
        self.record_core_result(result)?;
        self.input_state = InputState::new();
        self.state = SessionState::Running;
        Ok(())
    }

    /// Stops host-driven execution while retaining the loaded machine.
    ///
    /// Held controls are released in deterministic order before the state changes.
    /// If a release fails, the session does not enter Stopped; already released
    /// controls stay released and the remaining controls can be retried. Calling
    /// reset or successfully loading new content resumes execution.
    pub fn stop(&mut self) -> Result<(), SessionError> {
        if self.state == SessionState::Empty {
            return Err(self.invalid_transition("stop"));
        }
        let has_held_controls = self.input_state.pressed_count() > 0;
        if has_held_controls && !self.core.descriptor().capabilities.input {
            return Err(SessionError::Core(CoreError::Unsupported("input")));
        }
        for control in InputControl::ALL {
            if self.input_state.is_pressed(control) {
                let event = InputEvent {
                    control,
                    pressed: false,
                };
                let result = self.core.send_input(event);
                self.record_core_result(result)?;
                self.input_state.apply(event);
            }
        }
        self.input_state = InputState::new();
        self.state = SessionState::Stopped;
        Ok(())
    }

    /// Advances host frame cadence only while this session is running.
    ///
    /// Call this on each host-loop tick, passing elapsed time since the prior
    /// tick. While paused, stopped, empty or halted, elapsed time is discarded
    /// and the accumulator is reset so resume never catches up paused time.
    pub fn advance_frame_cadence(
        &self,
        cadence: &mut FrameCadence,
        elapsed: std::time::Duration,
    ) -> CadenceAdvance {
        if self.state == SessionState::Running {
            cadence.advance(elapsed)
        } else {
            cadence.reset();
            CadenceAdvance::default()
        }
    }

    /// Runs the core to its next observable event while the session is running.
    pub fn run_until_event(&mut self) -> Result<CoreEvent, SessionError> {
        self.require_state("run", SessionState::Running)?;
        let result = self.core.run_until_event();
        let event = self.record_core_result(result)?;
        if event == CoreEvent::Halted {
            self.state = SessionState::Halted;
        }
        Ok(event)
    }

    /// Runs a bounded number of core event steps.
    ///
    /// The budget counts calls to `run_until_event`, not cycles or wall-clock
    /// time. The method returns early on any event other than `Yielded`, since
    /// such an event may require host work (media retrieval, debugging, or
    /// stopping). A zero budget performs no core execution. As with the single
    /// step method, the core is responsible for making each individual call
    /// return at an observable boundary.
    pub fn run_for_events(&mut self, max_events: usize) -> Result<SessionRunReport, SessionError> {
        self.require_state("run", SessionState::Running)?;
        let mut report = SessionRunReport {
            events_processed: 0,
            last_event: None,
            budget_exhausted: true,
        };

        while report.events_processed < max_events {
            let event = self.run_until_event()?;
            report.events_processed += 1;
            report.last_event = Some(event);

            if event != CoreEvent::Yielded {
                report.budget_exhausted = false;
                break;
            }
        }

        Ok(report)
    }

    /// Serializes the current machine state when the core advertises save-state support.
    ///
    /// The session must have loaded content. The returned bytes are owned by the
    /// caller and can be persisted or transferred without borrowing the core.
    pub fn save_state(&mut self) -> Result<Vec<u8>, SessionError> {
        self.require_loaded("save state")?;
        if !self.core.descriptor().capabilities.save_states {
            return Err(SessionError::Core(CoreError::Unsupported("save states")));
        }
        let result = self.core.save_state();
        self.record_core_result(result)
    }

    /// Restores a machine state when the core advertises save-state support.
    ///
    /// Content must already be loaded. The core must validate the complete state
    /// before mutating itself; a rejected state must leave the active machine
    /// unchanged. Session lifecycle state is preserved on success and failure.
    pub fn load_state(&mut self, state: &[u8]) -> Result<(), SessionError> {
        self.require_loaded("load state")?;
        if !self.core.descriptor().capabilities.save_states {
            return Err(SessionError::Core(CoreError::Unsupported("save states")));
        }
        let result = self.core.load_state(state);
        self.record_core_result(result)?;
        Ok(())
    }

    /// Captures battery-backed cartridge data without serializing machine execution state.
    pub fn save_game_data(&mut self) -> Result<Vec<u8>, SessionError> {
        self.require_loaded("save game data")?;
        if !self.core.descriptor().capabilities.persistent_saves {
            return Err(SessionError::Core(CoreError::Unsupported(
                "persistent game saves",
            )));
        }
        let result = self.core.save_game_data();
        self.record_core_result(result)
    }

    /// Restores battery-backed cartridge data without changing CPU or session lifecycle state.
    pub fn load_game_data(&mut self, data: &[u8]) -> Result<(), SessionError> {
        self.require_loaded("load game data")?;
        if !self.core.descriptor().capabilities.persistent_saves {
            return Err(SessionError::Core(CoreError::Unsupported(
                "persistent game saves",
            )));
        }
        let result = self.core.load_game_data(data);
        self.record_core_result(result)
    }

    /// Returns whether battery-backed cartridge data needs to be written to durable storage.
    pub fn persistent_save_dirty(&self) -> bool {
        self.core.descriptor().capabilities.persistent_saves && self.core.persistent_save_dirty()
    }

    /// Clears the core's persistent-save dirty flag after a successful durable write.
    pub fn clear_persistent_save_dirty(&mut self) {
        if self.core.descriptor().capabilities.persistent_saves {
            self.core.clear_persistent_save_dirty();
        }
    }

    /// Captures a save state wrapped with the active core's identity and version.
    ///
    /// The payload is opaque to the host. The envelope's size and metadata checks
    /// happen before callers persist or transfer the returned bytes.
    pub fn save_state_envelope(&mut self) -> Result<SaveStateEnvelope, SessionError> {
        let descriptor = self.core.descriptor();
        let payload = self.save_state()?;
        SaveStateEnvelope::new(
            descriptor.system_id.as_str(),
            descriptor.core_version,
            payload,
        )
        .map_err(SessionError::SaveState)
    }

    /// Validates a save-state envelope against the active core before restoration.
    ///
    /// The core still validates its internal state representation. Compatibility
    /// failures are rejected before the core receives any payload bytes.
    pub fn load_state_envelope(
        &mut self,
        envelope: &SaveStateEnvelope,
    ) -> Result<(), SessionError> {
        let descriptor = self.core.descriptor();
        envelope.validate_for(descriptor.system_id.as_str(), descriptor.core_version)?;
        self.load_state(envelope.payload())
    }

    /// Sends a semantic input transition while a loaded machine is active.
    ///
    /// Input is allowed while paused so a host can reconcile held controls
    /// before resuming. Input is rejected before loading or after a halt.
    pub fn send_input(&mut self, event: InputEvent) -> Result<(), SessionError> {
        if !matches!(self.state, SessionState::Running | SessionState::Paused) {
            return Err(self.invalid_transition("send input"));
        }
        if !self.core.descriptor().capabilities.input {
            return Err(SessionError::Core(CoreError::Unsupported("input")));
        }
        if self.input_state.is_pressed(event.control) == event.pressed {
            return Ok(());
        }
        let result = self.core.send_input(event);
        self.record_core_result(result)?;
        self.input_state.apply(event);
        Ok(())
    }

    /// Releases every held control in a deterministic order.
    ///
    /// Hosts should call this when application focus is lost or a controller
    /// disconnects. Successfully released controls are removed from the local
    /// state immediately; if a core rejects a release, that control remains
    /// marked as held so the host can retry.
    pub fn release_all_inputs(&mut self) -> Result<(), SessionError> {
        if !matches!(self.state, SessionState::Running | SessionState::Paused) {
            return Err(self.invalid_transition("release inputs"));
        }
        if !self.core.descriptor().capabilities.input {
            return Err(SessionError::Core(CoreError::Unsupported("input")));
        }

        for control in InputControl::ALL {
            if self.input_state.is_pressed(control) {
                let event = InputEvent {
                    control,
                    pressed: false,
                };
                let result = self.core.send_input(event);
                self.record_core_result(result)?;
                self.input_state.apply(event);
            }
        }
        Ok(())
    }

    /// Returns whether a semantic control is currently held by this session.
    pub const fn is_input_pressed(&self, control: InputControl) -> bool {
        self.input_state.is_pressed(control)
    }

    /// Returns all currently held semantic controls in canonical control order.
    ///
    /// This is a host-side view of successfully delivered input transitions. It
    /// is useful for debugging stuck controls and reconciling frontend state.
    pub fn pressed_inputs(&self) -> Vec<InputControl> {
        InputControl::ALL
            .into_iter()
            .filter(|control| self.input_state.is_pressed(*control))
            .collect()
    }

    /// Retrieves the next video frame from a loaded session, if one is waiting.
    pub fn take_video_frame(&mut self) -> Result<Option<VideoFrame>, SessionError> {
        self.require_loaded("retrieve video")?;
        if !self.core.descriptor().capabilities.video_output {
            return Err(SessionError::Core(CoreError::Unsupported("video output")));
        }
        let result = self.core.take_video_frame();
        self.record_core_result(result)
    }

    /// Retrieves the next audio buffer from a loaded session, if one is waiting.
    pub fn take_audio_buffer(&mut self) -> Result<Option<AudioBuffer>, SessionError> {
        self.require_loaded("retrieve audio")?;
        if !self.core.descriptor().capabilities.audio_output {
            return Err(SessionError::Core(CoreError::Unsupported("audio output")));
        }
        let result = self.core.take_audio_buffer();
        self.record_core_result(result)
    }

    fn require_state(
        &self,
        operation: &'static str,
        expected: SessionState,
    ) -> Result<(), SessionError> {
        if self.state == expected {
            Ok(())
        } else {
            Err(self.invalid_transition(operation))
        }
    }

    fn require_loaded(&self, operation: &'static str) -> Result<(), SessionError> {
        if self.state == SessionState::Empty {
            Err(self.invalid_transition(operation))
        } else {
            Ok(())
        }
    }

    fn invalid_transition(&self, operation: &'static str) -> SessionError {
        SessionError::InvalidTransition {
            operation,
            state: self.state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CoreCapabilities, SystemId};

    struct TestCore {
        loaded: bool,
        calls: usize,
        next_event: CoreEvent,
    }

    impl EmulationCore for TestCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: crate::CURRENT_CORE_API_VERSION,
                system_id: SystemId::new("runtime-test").unwrap(),
                display_name: "Runtime test core",
                core_version: "0.0.0-test",
                capabilities: CoreCapabilities {
                    content_loading: true,
                    ..CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
            if content.is_empty() {
                return Err(CoreError::InvalidContent("empty test content"));
            }
            if content == b"REJECT" {
                return Err(CoreError::InvalidContent("rejected launch test content"));
            }
            self.loaded = true;
            Ok(())
        }

        fn reset(&mut self) -> Result<(), CoreError> {
            if !self.loaded {
                return Err(CoreError::NotInitialized);
            }
            self.calls = 0;
            self.next_event = CoreEvent::Yielded;
            Ok(())
        }

        fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
            if !self.loaded {
                return Err(CoreError::NotInitialized);
            }
            self.calls += 1;
            Ok(self.next_event)
        }
    }

    struct InputTestCore {
        loaded: bool,
        delivered: std::sync::Arc<std::sync::Mutex<Vec<InputEvent>>>,
        fail_release: bool,
    }

    impl EmulationCore for InputTestCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: crate::CURRENT_CORE_API_VERSION,
                system_id: SystemId::new("input-test").unwrap(),
                display_name: "Input test core",
                core_version: "0.0.0-test",
                capabilities: CoreCapabilities {
                    content_loading: true,
                    input: true,
                    ..CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
            if content.is_empty() {
                return Err(CoreError::InvalidContent("empty test content"));
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

        fn send_input(&mut self, event: InputEvent) -> Result<(), CoreError> {
            if self.fail_release && !event.pressed {
                return Err(CoreError::Execution("input release failed"));
            }
            self.delivered.lock().unwrap().push(event);
            Ok(())
        }
    }

    fn input_session(
        fail_release: bool,
    ) -> (
        EmulationSession,
        std::sync::Arc<std::sync::Mutex<Vec<InputEvent>>>,
    ) {
        let delivered = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let core = InputTestCore {
            loaded: false,
            delivered: delivered.clone(),
            fail_release,
        };
        (EmulationSession::new(Box::new(core)), delivered)
    }

    struct MediaTestCore {
        loaded: bool,
        frame: Option<VideoFrame>,
        audio: Option<AudioBuffer>,
    }

    impl EmulationCore for MediaTestCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: crate::CURRENT_CORE_API_VERSION,
                system_id: SystemId::new("media-test").unwrap(),
                display_name: "Media test core",
                core_version: "0.0.0-test",
                capabilities: CoreCapabilities {
                    content_loading: true,
                    video_output: true,
                    audio_output: true,
                    ..CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
            if content.is_empty() {
                return Err(CoreError::InvalidContent("empty test content"));
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

        fn take_video_frame(&mut self) -> Result<Option<VideoFrame>, CoreError> {
            Ok(self.frame.take())
        }

        fn take_audio_buffer(&mut self) -> Result<Option<AudioBuffer>, CoreError> {
            Ok(self.audio.take())
        }
    }

    #[test]
    fn media_retrieval_rejects_undeclared_capabilities_explicitly() {
        let mut session = session();
        assert!(matches!(
            session.take_video_frame(),
            Err(SessionError::InvalidTransition { .. })
        ));
        session.load_content(&[1]).unwrap();
        assert!(matches!(
            session.take_video_frame(),
            Err(SessionError::Core(CoreError::Unsupported("video output")))
        ));
        assert!(matches!(
            session.take_audio_buffer(),
            Err(SessionError::Core(CoreError::Unsupported("audio output")))
        ));
    }

    #[test]
    fn media_retrieval_transfers_owned_payload_once_and_then_returns_empty() {
        let frame = VideoFrame::new(
            2,
            1,
            crate::core_io::PixelFormat::Rgba8888,
            vec![1, 2, 3, 4, 5, 6, 7, 8],
        )
        .unwrap();
        let audio = AudioBuffer::new(48_000, 2, vec![10, -10, 20, -20]).unwrap();
        let mut session = EmulationSession::new(Box::new(MediaTestCore {
            loaded: false,
            frame: Some(frame.clone()),
            audio: Some(audio.clone()),
        }));
        assert!(matches!(
            session.take_video_frame(),
            Err(SessionError::InvalidTransition { .. })
        ));
        session.load_content(&[1]).unwrap();

        assert_eq!(session.take_video_frame().unwrap(), Some(frame));
        assert_eq!(session.take_video_frame().unwrap(), None);
        assert_eq!(session.take_audio_buffer().unwrap(), Some(audio));
        assert_eq!(session.take_audio_buffer().unwrap(), None);
    }

    struct ErrorOnceCore {
        fail_next_run: bool,
    }

    impl EmulationCore for ErrorOnceCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: crate::CURRENT_CORE_API_VERSION,
                system_id: SystemId::new("error-once-test").unwrap(),
                display_name: "Error-once test core",
                core_version: "0.0.0-test",
                capabilities: CoreCapabilities {
                    content_loading: true,
                    ..CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
            if content.is_empty() {
                return Err(CoreError::InvalidContent("empty test content"));
            }
            Ok(())
        }

        fn reset(&mut self) -> Result<(), CoreError> {
            self.fail_next_run = false;
            Ok(())
        }

        fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
            if self.fail_next_run {
                self.fail_next_run = false;
                return Err(CoreError::Execution("transient test failure"));
            }
            Ok(CoreEvent::Yielded)
        }
    }

    fn session() -> EmulationSession {
        EmulationSession::new(Box::new(TestCore {
            loaded: false,
            calls: 0,
            next_event: CoreEvent::Yielded,
        }))
    }

    struct PrefixProbe {
        id: &'static str,
        prefix: &'static [u8],
    }

    impl crate::content::ContentFormatProbe for PrefixProbe {
        fn format_id(&self) -> &'static str {
            self.id
        }

        fn probe(&self, bytes: &[u8]) -> crate::content::FormatProbeResult {
            if bytes.len() < self.prefix.len() && self.prefix.starts_with(bytes) {
                return crate::content::FormatProbeResult::NeedMoreData {
                    minimum_bytes: self.prefix.len(),
                };
            }
            if bytes.starts_with(self.prefix) {
                crate::content::FormatProbeResult::Match
            } else {
                crate::content::FormatProbeResult::NoMatch
            }
        }
    }

    fn make_runtime_test_core() -> Box<dyn EmulationCore> {
        Box::new(TestCore {
            loaded: false,
            calls: 0,
            next_event: CoreEvent::Yielded,
        })
    }

    #[test]
    fn held_input_inspection_starts_empty_and_preserves_canonical_order() {
        let mut session = session();
        assert!(session.pressed_inputs().is_empty());
        session.load_content(&[1]).unwrap();
        assert!(session.pressed_inputs().is_empty());
    }

    #[test]
    fn input_transitions_are_deduplicated_and_released_in_canonical_order() {
        let (mut session, delivered) = input_session(false);
        session.load_content(&[1]).unwrap();

        let up = InputEvent {
            control: InputControl::Up,
            pressed: true,
        };
        let action = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        session.send_input(up).unwrap();
        session.send_input(up).unwrap();
        session.send_input(action).unwrap();
        assert_eq!(
            session.pressed_inputs(),
            vec![InputControl::Up, InputControl::Action1]
        );
        assert_eq!(delivered.lock().unwrap().as_slice(), &[up, action]);

        session.release_all_inputs().unwrap();
        assert!(session.pressed_inputs().is_empty());
        assert_eq!(
            delivered.lock().unwrap().as_slice(),
            &[
                up,
                action,
                InputEvent {
                    control: InputControl::Up,
                    pressed: false
                },
                InputEvent {
                    control: InputControl::Action1,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn paused_session_accepts_input_and_ignores_duplicate_releases() {
        let (mut session, delivered) = input_session(false);
        session.load_content(&[1]).unwrap();
        session.pause().unwrap();

        let release = InputEvent {
            control: InputControl::Select,
            pressed: false,
        };
        session.send_input(release).unwrap();
        let press = InputEvent {
            control: InputControl::Select,
            pressed: true,
        };
        session.send_input(press).unwrap();
        session.send_input(press).unwrap();
        session.send_input(release).unwrap();
        session.send_input(release).unwrap();

        assert!(session.pressed_inputs().is_empty());
        assert_eq!(delivered.lock().unwrap().as_slice(), &[press, release]);
        assert_eq!(session.state(), SessionState::Paused);
    }

    #[test]
    fn failed_input_release_preserves_held_state_for_retry() {
        let (mut session, delivered) = input_session(true);
        session.load_content(&[1]).unwrap();
        let up = InputEvent {
            control: InputControl::Up,
            pressed: true,
        };
        session.send_input(up).unwrap();

        assert!(matches!(
            session.release_all_inputs(),
            Err(SessionError::Core(CoreError::Execution(
                "input release failed"
            )))
        ));
        assert!(session.is_input_pressed(InputControl::Up));
        assert_eq!(delivered.lock().unwrap().as_slice(), &[up]);
    }

    #[test]
    fn explicit_stop_retains_content_and_reset_resumes_execution() {
        let mut session = session();
        session.load_content(&[1]).unwrap();
        session.stop().unwrap();
        assert_eq!(session.state(), SessionState::Stopped);
        assert!(matches!(
            session.run_until_event(),
            Err(SessionError::InvalidTransition {
                operation: "run",
                state: SessionState::Stopped
            })
        ));
        session.reset().unwrap();
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.run_until_event(), Ok(CoreEvent::Yielded));
    }

    #[test]
    fn long_lifecycle_sequence_includes_stop_and_reload_recovery() {
        let mut session = session();
        session.load_content(&[1]).unwrap();
        for _ in 0..3 {
            session.pause().unwrap();
            session.resume().unwrap();
        }
        assert_eq!(session.run_for_events(2).unwrap().events_processed, 2);
        session.pause().unwrap();
        session.reset().unwrap();
        assert_eq!(session.state(), SessionState::Running);
        session.stop().unwrap();
        assert_eq!(session.state(), SessionState::Stopped);
        session.load_content(&[2]).unwrap();
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.run_until_event(), Ok(CoreEvent::Yielded));
    }

    #[test]
    fn bounded_path_and_reader_launch_share_the_detection_pipeline() {
        let probe = PrefixProbe {
            id: "runtime-test",
            prefix: b"GAME",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 1] = [&probe];
        let id = SystemId::new("runtime-test").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_runtime_test_core).unwrap();

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "host-app-launch-path-{}-{unique}.bin",
            std::process::id()
        ));
        std::fs::write(&path, b"GAME payload").unwrap();

        let path_result = launch_content_path(&path, ContentLimits::new(64), &probes, &registry);
        let _ = std::fs::remove_file(&path);
        let path_session = path_result.unwrap();
        assert_eq!(path_session.descriptor().system_id, id);
        assert_eq!(path_session.state(), SessionState::Running);

        let reader_session = launch_content_reader(
            std::io::Cursor::new(b"GAME payload".to_vec()),
            ContentLimits::new(64),
            &probes,
            &registry,
        )
        .unwrap();
        assert_eq!(reader_session.descriptor().system_id, id);
        assert_eq!(reader_session.state(), SessionState::Running);
    }

    #[test]
    fn bounded_path_and_reader_launch_reject_oversized_content_before_detection() {
        let probe = PrefixProbe {
            id: "runtime-test",
            prefix: b"GAME",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 1] = [&probe];
        let registry = CoreRegistry::new();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "host-app-launch-oversized-{}-{unique}.bin",
            std::process::id()
        ));
        std::fs::write(&path, b"GAME").unwrap();

        let path_result = launch_content_path(&path, ContentLimits::new(3), &probes, &registry);
        let _ = std::fs::remove_file(&path);
        assert!(matches!(
            path_result,
            Err(LaunchError::Content(ContentError::TooLarge {
                max_bytes: 3,
                actual_bytes: 4,
            }))
        ));
        assert!(matches!(
            launch_content_reader(
                std::io::Cursor::new(b"GAME".to_vec()),
                ContentLimits::new(3),
                &probes,
                &registry,
            ),
            Err(LaunchError::Content(ContentError::TooLarge {
                max_bytes: 3,
                actual_bytes: 4,
            }))
        ));
    }

    #[test]
    fn launch_pipeline_detects_selects_validates_and_starts_session() {
        let probe = PrefixProbe {
            id: "runtime-test",
            prefix: b"GAME",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 1] = [&probe];
        let id = SystemId::new("runtime-test").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_runtime_test_core).unwrap();

        let session =
            launch_content(b"GAME payload", ContentLimits::new(64), &probes, &registry).unwrap();

        assert_eq!(session.descriptor().system_id, id);
        assert_eq!(session.state(), SessionState::Running);
    }

    #[test]
    fn launch_pipeline_rejects_size_before_running_any_probe_or_core() {
        let probe = PrefixProbe {
            id: "runtime-test",
            prefix: b"GAME",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 1] = [&probe];
        let registry = CoreRegistry::new();

        assert!(matches!(
            launch_content(b"GAME", ContentLimits::new(3), &probes, &registry),
            Err(LaunchError::Content(ContentError::TooLarge {
                max_bytes: 3,
                actual_bytes: 4,
            }))
        ));
    }

    #[test]
    fn launch_pipeline_does_not_guess_when_detection_is_ambiguous() {
        let first = PrefixProbe {
            id: "runtime-test",
            prefix: b"GAME",
        };
        let second = PrefixProbe {
            id: "runtime-test",
            prefix: b"GAME",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 2] = [&first, &second];
        let id = SystemId::new("runtime-test").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_runtime_test_core).unwrap();

        assert!(matches!(
            launch_content(b"GAME", ContentLimits::new(64), &probes, &registry),
            Err(LaunchError::Registry(
                RegistryError::AmbiguousDetectedFormats(2)
            ))
        ));
    }

    #[test]
    fn launch_pipeline_rejects_detected_system_without_registered_core() {
        let probe = PrefixProbe {
            id: "unknown-system",
            prefix: b"GAME",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 1] = [&probe];
        let registry = CoreRegistry::new();

        assert!(matches!(
            launch_content(b"GAME", ContentLimits::new(64), &probes, &registry),
            Err(LaunchError::Registry(RegistryError::UnknownSystem(id)))
                if id.as_str() == "unknown-system"
        ));
    }

    #[test]
    fn launch_pipeline_waits_for_all_probes_to_decide() {
        let probe = PrefixProbe {
            id: "runtime-test",
            prefix: b"GAME",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 1] = [&probe];
        let id = SystemId::new("runtime-test").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_runtime_test_core).unwrap();

        assert!(matches!(
            launch_content(b"GA", ContentLimits::new(64), &probes, &registry),
            Err(LaunchError::Registry(RegistryError::IncompleteDetection(1)))
        ));
    }

    #[test]
    fn launch_pipeline_returns_core_validation_failure_instead_of_running() {
        let probe = PrefixProbe {
            id: "runtime-test",
            prefix: b"REJECT",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 1] = [&probe];
        let id = SystemId::new("runtime-test").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_runtime_test_core).unwrap();

        assert!(matches!(
            launch_content(b"REJECT", ContentLimits::new(64), &probes, &registry),
            Err(LaunchError::Session(SessionError::Core(
                CoreError::InvalidContent("rejected launch test content")
            )))
        ));
    }

    #[test]
    fn bounded_path_and_reader_launch_reject_ambiguous_detection() {
        let first = PrefixProbe {
            id: "runtime-test",
            prefix: b"GAME",
        };
        let second = PrefixProbe {
            id: "runtime-test",
            prefix: b"GAME",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 2] = [&first, &second];
        let id = SystemId::new("runtime-test").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_runtime_test_core).unwrap();

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "host-app-launch-ambiguous-{}-{unique}.bin",
            std::process::id()
        ));
        std::fs::write(&path, b"GAME payload").unwrap();
        let path_result = launch_content_path(&path, ContentLimits::new(64), &probes, &registry);
        let _ = std::fs::remove_file(&path);

        for result in [
            path_result,
            launch_content_reader(
                std::io::Cursor::new(b"GAME payload".to_vec()),
                ContentLimits::new(64),
                &probes,
                &registry,
            ),
        ] {
            assert!(matches!(
                result,
                Err(LaunchError::Registry(
                    RegistryError::AmbiguousDetectedFormats(2)
                ))
            ));
        }
    }

    #[test]
    fn bounded_path_and_reader_launch_preserve_core_validation_errors() {
        let probe = PrefixProbe {
            id: "runtime-test",
            prefix: b"REJECT",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 1] = [&probe];
        let id = SystemId::new("runtime-test").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_runtime_test_core).unwrap();

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "host-app-launch-rejected-{}-{unique}.bin",
            std::process::id()
        ));
        std::fs::write(&path, b"REJECT").unwrap();
        let path_result = launch_content_path(&path, ContentLimits::new(64), &probes, &registry);
        let _ = std::fs::remove_file(&path);

        for result in [
            path_result,
            launch_content_reader(
                std::io::Cursor::new(b"REJECT".to_vec()),
                ContentLimits::new(64),
                &probes,
                &registry,
            ),
        ] {
            assert!(matches!(
                result,
                Err(LaunchError::Session(SessionError::Core(
                    CoreError::InvalidContent("rejected launch test content")
                )))
            ));
        }
    }

    #[test]
    fn bounded_path_and_reader_launch_reject_empty_and_truncated_content() {
        let probe = PrefixProbe {
            id: "runtime-test",
            prefix: b"GAME",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 1] = [&probe];
        let id = SystemId::new("runtime-test").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_runtime_test_core).unwrap();

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "host-app-launch-truncated-{}-{unique}.bin",
            std::process::id()
        ));

        std::fs::write(&path, b"GA").unwrap();
        let truncated_path = launch_content_path(&path, ContentLimits::new(64), &probes, &registry);
        let truncated_reader = launch_content_reader(
            std::io::Cursor::new(b"GA".to_vec()),
            ContentLimits::new(64),
            &probes,
            &registry,
        );
        assert!(matches!(
            truncated_path,
            Err(LaunchError::Registry(RegistryError::IncompleteDetection(1)))
        ));
        assert!(matches!(
            truncated_reader,
            Err(LaunchError::Registry(RegistryError::IncompleteDetection(1)))
        ));

        std::fs::write(&path, b"").unwrap();
        let empty_path = launch_content_path(&path, ContentLimits::new(64), &probes, &registry);
        let empty_reader = launch_content_reader(
            std::io::Cursor::new(Vec::<u8>::new()),
            ContentLimits::new(64),
            &probes,
            &registry,
        );
        let _ = std::fs::remove_file(&path);
        assert!(matches!(
            empty_path,
            Err(LaunchError::Content(ContentError::Empty))
        ));
        assert!(matches!(
            empty_reader,
            Err(LaunchError::Content(ContentError::Empty))
        ));
    }

    #[test]
    fn bounded_path_and_reader_launch_propagate_source_read_errors() {
        struct FailingReader;

        impl std::io::Read for FailingReader {
            fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "intentional content source failure",
                ))
            }
        }

        let probe = PrefixProbe {
            id: "runtime-test",
            prefix: b"GAME",
        };
        let probes: [&dyn crate::content::ContentFormatProbe; 1] = [&probe];
        let id = SystemId::new("runtime-test").unwrap();
        let mut registry = CoreRegistry::new();
        registry.register(id, make_runtime_test_core).unwrap();

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let missing_path = std::env::temp_dir().join(format!(
            "host-app-launch-missing-{}-{unique}.bin",
            std::process::id()
        ));
        let path_result =
            launch_content_path(&missing_path, ContentLimits::new(64), &probes, &registry);
        let reader_result =
            launch_content_reader(FailingReader, ContentLimits::new(64), &probes, &registry);

        assert!(matches!(
            path_result,
            Err(LaunchError::Content(ContentError::ReadFailed(
                std::io::ErrorKind::NotFound
            )))
        ));
        assert!(matches!(
            reader_result,
            Err(LaunchError::Content(ContentError::ReadFailed(
                std::io::ErrorKind::PermissionDenied
            )))
        ));
    }

    #[test]
    fn bounded_load_rejects_invalid_size_without_calling_the_core() {
        let mut session = session();
        assert_eq!(
            session.load_content_bounded(&[1, 2], ContentLimits::new(1)),
            Err(SessionError::Content(ContentError::TooLarge {
                max_bytes: 1,
                actual_bytes: 2,
            }))
        );
        assert_eq!(session.state(), SessionState::Empty);
        assert!(matches!(
            session.load_content_bounded(&[], ContentLimits::new(1)),
            Err(SessionError::Content(ContentError::Empty))
        ));
        assert_eq!(session.state(), SessionState::Empty);

        session
            .load_content_bounded(&[1], ContentLimits::new(1))
            .unwrap();
        assert_eq!(session.state(), SessionState::Running);

        assert_eq!(
            session.load_content_bounded(&[1, 2], ContentLimits::new(1)),
            Err(SessionError::Content(ContentError::TooLarge {
                max_bytes: 1,
                actual_bytes: 2,
            }))
        );
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.run_until_event(), Ok(CoreEvent::Yielded));
    }

    #[test]
    fn oversized_bounded_reload_preserves_halted_state() {
        let core = TestCore {
            loaded: false,
            calls: 0,
            next_event: CoreEvent::Halted,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();
        assert_eq!(session.run_until_event(), Ok(CoreEvent::Halted));
        assert_eq!(session.state(), SessionState::Halted);

        assert_eq!(
            session.load_content_bounded(&[1, 2], ContentLimits::new(1)),
            Err(SessionError::Content(ContentError::TooLarge {
                max_bytes: 1,
                actual_bytes: 2,
            }))
        );
        assert_eq!(session.state(), SessionState::Halted);
        assert_eq!(
            session.run_until_event(),
            Err(SessionError::InvalidTransition {
                operation: "run",
                state: SessionState::Halted,
            })
        );
    }

    #[test]
    fn execution_is_blocked_until_content_is_loaded() {
        let mut session = session();
        assert_eq!(session.state(), SessionState::Empty);
        assert_eq!(
            session.run_until_event(),
            Err(SessionError::InvalidTransition {
                operation: "run",
                state: SessionState::Empty
            })
        );
        assert!(session.load_content(&[]).is_err());
        assert_eq!(session.state(), SessionState::Empty);
    }

    #[test]
    fn lifecycle_transition_matrix_covers_pause_resume_and_execution_in_every_state() {
        fn session_in_state(target: SessionState) -> EmulationSession {
            let next_event = if target == SessionState::Halted {
                CoreEvent::Halted
            } else {
                CoreEvent::Yielded
            };
            let mut session = EmulationSession::new(Box::new(TestCore {
                loaded: false,
                calls: 0,
                next_event,
            }));

            match target {
                SessionState::Empty => {}
                SessionState::Running => session.load_content(&[1]).unwrap(),
                SessionState::Paused => {
                    session.load_content(&[1]).unwrap();
                    session.pause().unwrap();
                }
                SessionState::Halted => {
                    session.load_content(&[1]).unwrap();
                    assert_eq!(session.run_until_event(), Ok(CoreEvent::Halted));
                }
                SessionState::Stopped => {
                    session.load_content(&[1]).unwrap();
                    session.stop().unwrap();
                }
            }
            assert_eq!(session.state(), target);
            session
        }

        let states = [
            SessionState::Empty,
            SessionState::Running,
            SessionState::Paused,
            SessionState::Halted,
            SessionState::Stopped,
        ];

        for state in states {
            let mut pausing = session_in_state(state);
            let pause_result = pausing.pause();
            if state == SessionState::Running {
                assert_eq!(pause_result, Ok(()), "pause from {state:?}");
                assert_eq!(pausing.state(), SessionState::Paused);
            } else {
                assert_eq!(
                    pause_result,
                    Err(SessionError::InvalidTransition {
                        operation: "pause",
                        state,
                    }),
                    "pause from {state:?}"
                );
                assert_eq!(pausing.state(), state);
            }

            let mut resuming = session_in_state(state);
            let resume_result = resuming.resume();
            if state == SessionState::Paused {
                assert_eq!(resume_result, Ok(()), "resume from {state:?}");
                assert_eq!(resuming.state(), SessionState::Running);
            } else {
                assert_eq!(
                    resume_result,
                    Err(SessionError::InvalidTransition {
                        operation: "resume",
                        state,
                    }),
                    "resume from {state:?}"
                );
                assert_eq!(resuming.state(), state);
            }

            let mut stepping = session_in_state(state);
            let step_result = stepping.run_until_event();
            if state == SessionState::Running {
                assert_eq!(step_result, Ok(CoreEvent::Yielded), "run from {state:?}");
            } else {
                assert_eq!(
                    step_result,
                    Err(SessionError::InvalidTransition {
                        operation: "run",
                        state,
                    }),
                    "run from {state:?}"
                );
            }
            assert_eq!(stepping.state(), state, "single-step state {state:?}");

            let mut bounded = session_in_state(state);
            let bounded_result = bounded.run_for_events(1);
            if state == SessionState::Running {
                assert_eq!(
                    bounded_result,
                    Ok(SessionRunReport {
                        events_processed: 1,
                        last_event: Some(CoreEvent::Yielded),
                        budget_exhausted: true,
                    }),
                    "bounded run from {state:?}"
                );
            } else {
                assert_eq!(
                    bounded_result,
                    Err(SessionError::InvalidTransition {
                        operation: "run",
                        state,
                    }),
                    "bounded run from {state:?}"
                );
            }
            assert_eq!(bounded.state(), state, "bounded-run state {state:?}");
        }
    }

    #[test]
    fn invalid_lifecycle_transitions_are_rejected_without_state_changes() {
        let mut session = session();

        assert!(matches!(
            session.pause(),
            Err(SessionError::InvalidTransition {
                operation: "pause",
                state: SessionState::Empty
            })
        ));
        assert!(matches!(
            session.resume(),
            Err(SessionError::InvalidTransition {
                operation: "resume",
                state: SessionState::Empty
            })
        ));
        assert!(matches!(
            session.reset(),
            Err(SessionError::InvalidTransition {
                operation: "reset",
                state: SessionState::Empty
            })
        ));
        assert_eq!(session.state(), SessionState::Empty);

        session.load_content(&[1]).unwrap();
        assert!(matches!(
            session.resume(),
            Err(SessionError::InvalidTransition {
                operation: "resume",
                state: SessionState::Running
            })
        ));
        session.pause().unwrap();
        assert!(matches!(
            session.pause(),
            Err(SessionError::InvalidTransition {
                operation: "pause",
                state: SessionState::Paused
            })
        ));
        assert_eq!(session.state(), SessionState::Paused);
    }

    #[test]
    fn input_operations_are_rejected_before_load_and_after_halt() {
        let mut empty = session();
        let press = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };

        assert_eq!(
            empty.send_input(press),
            Err(SessionError::InvalidTransition {
                operation: "send input",
                state: SessionState::Empty,
            })
        );
        assert_eq!(
            empty.release_all_inputs(),
            Err(SessionError::InvalidTransition {
                operation: "release inputs",
                state: SessionState::Empty,
            })
        );
        assert_eq!(empty.state(), SessionState::Empty);
        assert!(!empty.is_input_pressed(InputControl::Action1));

        let core = TestCore {
            loaded: false,
            calls: 0,
            next_event: CoreEvent::Halted,
        };
        let mut halted = EmulationSession::new(Box::new(core));
        halted.load_content(&[1]).unwrap();
        assert_eq!(halted.run_until_event(), Ok(CoreEvent::Halted));

        assert_eq!(
            halted.send_input(press),
            Err(SessionError::InvalidTransition {
                operation: "send input",
                state: SessionState::Halted,
            })
        );
        assert_eq!(
            halted.release_all_inputs(),
            Err(SessionError::InvalidTransition {
                operation: "release inputs",
                state: SessionState::Halted,
            })
        );
        assert_eq!(halted.state(), SessionState::Halted);
        assert!(!halted.is_input_pressed(InputControl::Action1));
    }

    #[test]
    fn long_lifecycle_sequence_keeps_state_and_bounded_execution_consistent() {
        let mut session = session();
        assert_eq!(session.state(), SessionState::Empty);

        session.load_content(&[1]).unwrap();
        assert_eq!(session.state(), SessionState::Running);

        session.pause().unwrap();
        assert_eq!(session.state(), SessionState::Paused);
        session.resume().unwrap();
        assert_eq!(session.state(), SessionState::Running);

        let report = session.run_for_events(3).unwrap();
        assert_eq!(
            report,
            SessionRunReport {
                events_processed: 3,
                last_event: Some(CoreEvent::Yielded),
                budget_exhausted: true,
            }
        );

        session.pause().unwrap();
        session.reset().unwrap();
        assert_eq!(session.state(), SessionState::Running);

        session.pause().unwrap();
        session.resume().unwrap();
        session.load_content(&[2]).unwrap();
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.run_until_event().unwrap(), CoreEvent::Yielded);
        assert_eq!(session.state(), SessionState::Running);
    }

    #[test]
    fn release_all_inputs_rejects_cores_without_input_capability() {
        let mut session = session();
        session.load_content(&[1]).unwrap();

        assert_eq!(
            session.release_all_inputs(),
            Err(SessionError::Core(CoreError::Unsupported("input")))
        );
        assert_eq!(session.state(), SessionState::Running);
    }

    #[test]
    fn zero_event_budget_does_not_advance_a_running_session() {
        let mut session = session();
        session.load_content(&[1]).unwrap();

        assert_eq!(
            session.run_for_events(0),
            Ok(SessionRunReport {
                events_processed: 0,
                last_event: None,
                budget_exhausted: true,
            })
        );
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.run_until_event(), Ok(CoreEvent::Yielded));
    }

    #[test]
    fn bounded_run_stops_at_halt_and_reports_budget_remaining() {
        let core = TestCore {
            loaded: false,
            calls: 0,
            next_event: CoreEvent::Halted,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        assert_eq!(
            session.run_for_events(8),
            Ok(SessionRunReport {
                events_processed: 1,
                last_event: Some(CoreEvent::Halted),
                budget_exhausted: false,
            })
        );
        assert_eq!(session.state(), SessionState::Halted);
        assert!(matches!(
            session.run_for_events(1),
            Err(SessionError::InvalidTransition {
                operation: "run",
                state: SessionState::Halted,
            })
        ));
    }

    #[test]
    fn bounded_runner_recovers_after_a_transient_execution_error() {
        let core = ErrorOnceCore {
            fail_next_run: true,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        let transient_error = CoreError::Execution("transient test failure");
        assert_eq!(
            session.run_for_events(2),
            Err(SessionError::Core(transient_error.clone()))
        );
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.last_core_error(), Some(transient_error.clone()));

        assert_eq!(
            session.run_for_events(2),
            Ok(SessionRunReport {
                events_processed: 2,
                last_event: Some(CoreEvent::Yielded),
                budget_exhausted: true,
            })
        );
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.last_core_error(), Some(transient_error));
    }

    #[test]
    fn halted_event_blocks_execution_until_reset_recovers_session() {
        let core = TestCore {
            loaded: false,
            calls: 0,
            next_event: CoreEvent::Halted,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        assert_eq!(session.run_until_event(), Ok(CoreEvent::Halted));
        assert_eq!(session.state(), SessionState::Halted);
        assert_eq!(
            session.run_until_event(),
            Err(SessionError::InvalidTransition {
                operation: "run",
                state: SessionState::Halted,
            })
        );
        assert_eq!(
            session.pause(),
            Err(SessionError::InvalidTransition {
                operation: "pause",
                state: SessionState::Halted,
            })
        );
        assert_eq!(
            session.resume(),
            Err(SessionError::InvalidTransition {
                operation: "resume",
                state: SessionState::Halted,
            })
        );

        session.reset().unwrap();
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.run_until_event(), Ok(CoreEvent::Yielded));
        assert_eq!(session.state(), SessionState::Running);
    }

    #[test]
    fn core_error_diagnostic_persists_until_explicitly_cleared() {
        let core = ErrorOnceCore {
            fail_next_run: true,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        let error = CoreError::Execution("transient test failure");
        assert_eq!(
            session.run_until_event(),
            Err(SessionError::Core(error.clone()))
        );
        assert_eq!(session.last_core_error(), Some(error.clone()));
        assert_eq!(session.state(), SessionState::Running);

        assert_eq!(session.run_until_event(), Ok(CoreEvent::Yielded));
        assert_eq!(session.last_core_error(), Some(error));
        session.clear_last_core_error();
        assert_eq!(session.last_core_error(), None);
    }

    #[test]
    fn lifecycle_precondition_error_does_not_replace_core_diagnostic() {
        let core = ErrorOnceCore {
            fail_next_run: true,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        let core_error = CoreError::Execution("transient test failure");
        assert_eq!(
            session.run_until_event(),
            Err(SessionError::Core(core_error.clone()))
        );
        assert_eq!(session.last_core_error(), Some(core_error.clone()));

        session.pause().unwrap();
        assert_eq!(
            session.run_until_event(),
            Err(SessionError::InvalidTransition {
                operation: "run",
                state: SessionState::Paused,
            })
        );
        assert_eq!(session.state(), SessionState::Paused);
        assert_eq!(session.last_core_error(), Some(core_error));
    }

    #[test]
    fn bounded_content_precondition_preserves_core_diagnostic() {
        let core = ErrorOnceCore {
            fail_next_run: true,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        let core_error = CoreError::Execution("transient test failure");
        assert_eq!(
            session.run_until_event(),
            Err(SessionError::Core(core_error.clone()))
        );
        session.pause().unwrap();

        assert_eq!(
            session.load_content_bounded(&[1, 2], ContentLimits::new(1)),
            Err(SessionError::Content(ContentError::TooLarge {
                max_bytes: 1,
                actual_bytes: 2,
            }))
        );
        assert_eq!(session.state(), SessionState::Paused);
        assert_eq!(session.last_core_error(), Some(core_error));
    }

    #[test]
    fn execution_error_preserves_session_state_and_allows_recovery() {
        let core = ErrorOnceCore {
            fail_next_run: true,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        assert_eq!(
            session.run_until_event(),
            Err(SessionError::Core(CoreError::Execution(
                "transient test failure"
            )))
        );
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("transient test failure"))
        );
        assert_eq!(session.run_until_event().unwrap(), CoreEvent::Yielded);
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("transient test failure"))
        );
    }

    #[test]
    fn failed_content_reload_preserves_the_existing_session_state() {
        let mut session = session();
        session.load_content(&[1]).unwrap();

        assert_eq!(
            session.load_content(&[]),
            Err(SessionError::Core(CoreError::InvalidContent(
                "empty test content"
            )))
        );
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::InvalidContent("empty test content"))
        );
        assert_eq!(session.run_until_event().unwrap(), CoreEvent::Yielded);
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::InvalidContent("empty test content"))
        );
    }

    #[test]
    fn failed_reload_preserves_paused_and_halted_lifecycle_states() {
        let mut paused = session();
        paused.load_content(&[1]).unwrap();
        paused.pause().unwrap();

        assert!(paused.load_content(&[]).is_err());
        assert_eq!(paused.state(), SessionState::Paused);
        assert_eq!(
            paused.last_core_error(),
            Some(CoreError::InvalidContent("empty test content"))
        );
        paused.resume().unwrap();
        assert_eq!(paused.run_until_event(), Ok(CoreEvent::Yielded));
        assert_eq!(
            paused.last_core_error(),
            Some(CoreError::InvalidContent("empty test content"))
        );

        let core = TestCore {
            loaded: false,
            calls: 0,
            next_event: CoreEvent::Halted,
        };
        let mut halted = EmulationSession::new(Box::new(core));
        halted.load_content(&[1]).unwrap();
        assert_eq!(halted.run_until_event(), Ok(CoreEvent::Halted));
        assert_eq!(halted.state(), SessionState::Halted);

        assert!(halted.load_content(&[]).is_err());
        assert_eq!(halted.state(), SessionState::Halted);
        assert_eq!(
            halted.last_core_error(),
            Some(CoreError::InvalidContent("empty test content"))
        );
        halted.reset().unwrap();
        assert_eq!(halted.state(), SessionState::Running);
        assert_eq!(
            halted.last_core_error(),
            Some(CoreError::InvalidContent("empty test content"))
        );
    }

    #[test]
    fn paused_time_and_fractional_cadence_are_discarded_before_resume() {
        let mut session = session();
        session.load_content(&[1]).unwrap();
        let mut cadence = FrameCadence::new(60, 4).unwrap();
        let period = cadence.frame_period();

        assert_eq!(
            session
                .advance_frame_cadence(&mut cadence, period / 2)
                .due_frames,
            0
        );
        session.pause().unwrap();
        assert_eq!(
            session
                .advance_frame_cadence(&mut cadence, period * 4)
                .due_frames,
            0
        );
        session.resume().unwrap();

        assert_eq!(
            session
                .advance_frame_cadence(&mut cadence, period - std::time::Duration::from_nanos(1),)
                .due_frames,
            0
        );
        assert_eq!(
            session
                .advance_frame_cadence(&mut cadence, std::time::Duration::from_nanos(1))
                .due_frames,
            1
        );
    }

    #[test]
    fn pause_resume_and_reset_follow_valid_transitions() {
        let mut session = session();
        session.load_content(&[1]).unwrap();
        assert_eq!(session.state(), SessionState::Running);
        session.pause().unwrap();
        assert_eq!(session.state(), SessionState::Paused);
        assert!(session.run_until_event().is_err());
        session.resume().unwrap();
        assert_eq!(session.run_until_event().unwrap(), CoreEvent::Yielded);
        session.reset().unwrap();
        assert_eq!(session.state(), SessionState::Running);
    }

    #[test]
    fn halted_event_updates_session_state() {
        let mut core = TestCore {
            loaded: false,
            calls: 0,
            next_event: CoreEvent::Halted,
        };
        core.load_content(&[1]).unwrap();
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();
        assert_eq!(session.run_until_event().unwrap(), CoreEvent::Halted);
        assert_eq!(session.state(), SessionState::Halted);
        assert!(session.run_until_event().is_err());
        assert!(session
            .send_input(InputEvent {
                control: crate::core_io::InputControl::Action1,
                pressed: true,
            })
            .is_err());
        session.reset().unwrap();
        assert_eq!(session.state(), SessionState::Running);
    }

    #[test]
    fn bounded_runner_stops_at_budget_or_host_visible_event() {
        let mut session = session();
        session.load_content(&[1]).unwrap();

        let zero = session.run_for_events(0).unwrap();
        assert_eq!(
            zero,
            SessionRunReport {
                events_processed: 0,
                last_event: None,
                budget_exhausted: true,
            }
        );

        let yielded = session.run_for_events(3).unwrap();
        assert_eq!(yielded.events_processed, 3);
        assert_eq!(yielded.last_event, Some(CoreEvent::Yielded));
        assert!(yielded.budget_exhausted);

        session.pause().unwrap();
        assert!(session.run_for_events(1).is_err());
    }

    #[test]
    fn bounded_runner_stops_on_frame_and_halt_events() {
        let mut core = TestCore {
            loaded: false,
            calls: 0,
            next_event: CoreEvent::FrameReady,
        };
        core.load_content(&[1]).unwrap();
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        let report = session.run_for_events(100).unwrap();
        assert_eq!(report.events_processed, 1);
        assert_eq!(report.last_event, Some(CoreEvent::FrameReady));
        assert!(!report.budget_exhausted);

        session.reset().unwrap();
        // This test core emits FrameReady until reset; a separate halted core
        // verifies that a halt remains terminal even inside the bounded runner.
        let mut halted = TestCore {
            loaded: false,
            calls: 0,
            next_event: CoreEvent::Halted,
        };
        halted.load_content(&[1]).unwrap();
        let mut halted_session = EmulationSession::new(Box::new(halted));
        halted_session.load_content(&[1]).unwrap();
        let report = halted_session.run_for_events(100).unwrap();
        assert_eq!(report.events_processed, 1);
        assert_eq!(report.last_event, Some(CoreEvent::Halted));
        assert_eq!(halted_session.state(), SessionState::Halted);
        assert!(!report.budget_exhausted);
    }

    #[test]
    fn successful_content_reload_resumes_a_paused_session() {
        let mut session = session();
        session.load_content(&[1]).unwrap();
        session.pause().unwrap();

        session.load_content(&[2]).unwrap();

        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.run_until_event().unwrap(), CoreEvent::Yielded);
    }

    #[test]
    fn successful_content_reload_recovers_a_halted_session() {
        let core = TestCore {
            loaded: false,
            calls: 0,
            next_event: CoreEvent::Halted,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();
        assert_eq!(session.run_until_event().unwrap(), CoreEvent::Halted);
        assert_eq!(session.state(), SessionState::Halted);

        session.load_content(&[2]).unwrap();

        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.run_until_event().unwrap(), CoreEvent::Halted);
    }

    struct ResetFailureCore {
        halts: bool,
    }

    impl EmulationCore for ResetFailureCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: crate::CURRENT_CORE_API_VERSION,
                system_id: SystemId::new("reset-failure-test").unwrap(),
                display_name: "Reset failure test core",
                core_version: "0.0.0-test",
                capabilities: CoreCapabilities {
                    content_loading: true,
                    ..CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
            if content.is_empty() {
                Err(CoreError::InvalidContent("empty test content"))
            } else {
                Ok(())
            }
        }

        fn reset(&mut self) -> Result<(), CoreError> {
            Err(CoreError::Execution("reset failed"))
        }

        fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
            Ok(if self.halts {
                CoreEvent::Halted
            } else {
                CoreEvent::Yielded
            })
        }
    }

    #[test]
    fn failed_reset_preserves_the_previous_lifecycle_state() {
        let mut running = EmulationSession::new(Box::new(ResetFailureCore { halts: false }));
        running.load_content(&[1]).unwrap();
        assert_eq!(
            running.reset(),
            Err(SessionError::Core(CoreError::Execution("reset failed")))
        );
        assert_eq!(running.state(), SessionState::Running);

        let mut paused = EmulationSession::new(Box::new(ResetFailureCore { halts: false }));
        paused.load_content(&[1]).unwrap();
        paused.pause().unwrap();
        assert_eq!(
            paused.reset(),
            Err(SessionError::Core(CoreError::Execution("reset failed")))
        );
        assert_eq!(paused.state(), SessionState::Paused);
        assert!(paused.run_until_event().is_err());

        let mut halted = EmulationSession::new(Box::new(ResetFailureCore { halts: true }));
        halted.load_content(&[1]).unwrap();
        assert_eq!(halted.run_until_event(), Ok(CoreEvent::Halted));
        assert_eq!(
            halted.reset(),
            Err(SessionError::Core(CoreError::Execution("reset failed")))
        );
        assert_eq!(halted.state(), SessionState::Halted);
        assert!(halted.run_until_event().is_err());
    }

    #[test]
    fn save_state_round_trip_restores_machine_data_without_changing_lifecycle() {
        struct StatefulCore {
            value: u8,
        }

        impl EmulationCore for StatefulCore {
            fn descriptor(&self) -> CoreDescriptor {
                CoreDescriptor {
                    api_version: crate::CURRENT_CORE_API_VERSION,
                    system_id: SystemId::new("stateful-test").unwrap(),
                    display_name: "Stateful test core",
                    core_version: "0.0.0-test",
                    capabilities: CoreCapabilities {
                        content_loading: true,
                        save_states: true,
                        ..CoreCapabilities::default()
                    },
                }
            }

            fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
                let Some(value) = content.first() else {
                    return Err(CoreError::InvalidContent("empty test content"));
                };
                self.value = *value;
                Ok(())
            }

            fn reset(&mut self) -> Result<(), CoreError> {
                self.value = 0;
                Ok(())
            }

            fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
                self.value = self.value.wrapping_add(1);
                Ok(CoreEvent::Yielded)
            }

            fn save_state(&self) -> Result<Vec<u8>, CoreError> {
                Ok(vec![self.value])
            }

            fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError> {
                if state.len() != 1 {
                    return Err(CoreError::InvalidState("expected one byte"));
                }
                self.value = state[0];
                Ok(())
            }
        }

        let mut session = EmulationSession::new(Box::new(StatefulCore { value: 0 }));
        session.load_content(&[7]).unwrap();
        let saved = session.save_state().unwrap();
        session.run_until_event().unwrap();
        assert_eq!(session.save_state().unwrap(), vec![8]);

        session.pause().unwrap();
        session.load_state(&saved).unwrap();

        assert_eq!(session.state(), SessionState::Paused);
        assert_eq!(session.save_state().unwrap(), vec![7]);
        assert_eq!(
            session.load_state(&[1, 2]),
            Err(SessionError::Core(CoreError::InvalidState(
                "expected one byte"
            )))
        );
        assert_eq!(session.state(), SessionState::Paused);
        assert_eq!(session.save_state().unwrap(), vec![7]);
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::InvalidState("expected one byte"))
        );
    }

    #[test]
    fn failed_save_state_is_retained_as_a_core_diagnostic() {
        struct FailingSaveCore;

        impl EmulationCore for FailingSaveCore {
            fn descriptor(&self) -> CoreDescriptor {
                CoreDescriptor {
                    api_version: crate::CURRENT_CORE_API_VERSION,
                    system_id: SystemId::new("failing-save-test").unwrap(),
                    display_name: "Failing save test core",
                    core_version: "0.0.0-test",
                    capabilities: CoreCapabilities {
                        content_loading: true,
                        save_states: true,
                        ..CoreCapabilities::default()
                    },
                }
            }

            fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
                if content.is_empty() {
                    return Err(CoreError::InvalidContent("empty test content"));
                }
                Ok(())
            }

            fn reset(&mut self) -> Result<(), CoreError> {
                Ok(())
            }

            fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
                Ok(CoreEvent::Yielded)
            }

            fn save_state(&self) -> Result<Vec<u8>, CoreError> {
                Err(CoreError::Execution("save serialization failed"))
            }
        }

        let mut session = EmulationSession::new(Box::new(FailingSaveCore));
        session.load_content(&[1]).unwrap();
        assert_eq!(
            session.save_state(),
            Err(SessionError::Core(CoreError::Execution(
                "save serialization failed"
            )))
        );
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("save serialization failed"))
        );
        assert_eq!(session.state(), SessionState::Running);
        session.clear_last_core_error();
        assert_eq!(session.last_core_error(), None);
    }

    #[test]
    fn failed_media_retrieval_is_retained_as_a_core_diagnostic() {
        struct FailingMediaCore;

        impl EmulationCore for FailingMediaCore {
            fn descriptor(&self) -> CoreDescriptor {
                CoreDescriptor {
                    api_version: crate::CURRENT_CORE_API_VERSION,
                    system_id: SystemId::new("failing-media-test").unwrap(),
                    display_name: "Failing media test core",
                    core_version: "0.0.0-test",
                    capabilities: CoreCapabilities {
                        content_loading: true,
                        video_output: true,
                        audio_output: true,
                        ..CoreCapabilities::default()
                    },
                }
            }

            fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
                if content.is_empty() {
                    return Err(CoreError::InvalidContent("empty test content"));
                }
                Ok(())
            }

            fn reset(&mut self) -> Result<(), CoreError> {
                Ok(())
            }

            fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
                Ok(CoreEvent::Yielded)
            }

            fn take_video_frame(
                &mut self,
            ) -> Result<Option<crate::core_io::VideoFrame>, CoreError> {
                Err(CoreError::Execution("video retrieval failed"))
            }

            fn take_audio_buffer(
                &mut self,
            ) -> Result<Option<crate::core_io::AudioBuffer>, CoreError> {
                Err(CoreError::Execution("audio retrieval failed"))
            }
        }

        let mut session = EmulationSession::new(Box::new(FailingMediaCore));
        session.load_content(&[1]).unwrap();

        assert_eq!(
            session.take_video_frame(),
            Err(SessionError::Core(CoreError::Execution(
                "video retrieval failed"
            )))
        );
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("video retrieval failed"))
        );
        assert_eq!(
            session.take_audio_buffer(),
            Err(SessionError::Core(CoreError::Execution(
                "audio retrieval failed"
            )))
        );
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("audio retrieval failed"))
        );
        assert_eq!(session.state(), SessionState::Running);
    }

    #[test]
    fn save_states_require_a_loaded_session_and_advertised_capability() {
        let mut session = session();
        assert!(session.save_state().is_err());
        assert!(session.load_state(&[1]).is_err());
        session.load_content(&[1]).unwrap();
        assert_eq!(
            session.save_state(),
            Err(SessionError::Core(CoreError::Unsupported("save states")))
        );
        assert_eq!(
            session.load_state(&[1]),
            Err(SessionError::Core(CoreError::Unsupported("save states")))
        );
    }

    #[test]
    fn content_loading_is_rejected_when_capability_is_not_advertised() {
        struct NoContentLoadingCore;

        impl EmulationCore for NoContentLoadingCore {
            fn descriptor(&self) -> CoreDescriptor {
                CoreDescriptor {
                    api_version: crate::CURRENT_CORE_API_VERSION,
                    system_id: SystemId::new("no-content-loading-test").unwrap(),
                    display_name: "No content loading test core",
                    core_version: "0.0.0-test",
                    capabilities: CoreCapabilities::default(),
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

        let mut session = EmulationSession::new(Box::new(NoContentLoadingCore));
        assert_eq!(
            session.load_content(&[1]),
            Err(SessionError::Core(CoreError::Unsupported(
                "content loading"
            )))
        );
        assert_eq!(session.state(), SessionState::Empty);
        assert_eq!(
            session.load_content_bounded(&[1], ContentLimits::new(1)),
            Err(SessionError::Core(CoreError::Unsupported(
                "content loading"
            )))
        );
        assert_eq!(
            session.load_content_bounded(&[1, 2], ContentLimits::new(1)),
            Err(SessionError::Core(CoreError::Unsupported(
                "content loading"
            )))
        );
        assert_eq!(session.state(), SessionState::Empty);
    }

    #[test]
    fn runtime_rejects_operations_not_advertised_by_the_core() {
        struct UnderreportedCore;

        impl EmulationCore for UnderreportedCore {
            fn descriptor(&self) -> CoreDescriptor {
                CoreDescriptor {
                    api_version: crate::CURRENT_CORE_API_VERSION,
                    system_id: SystemId::new("underreported-test").unwrap(),
                    display_name: "Underreported test core",
                    core_version: "0.0.0-test",
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

            fn send_input(&mut self, _: InputEvent) -> Result<(), CoreError> {
                Ok(())
            }

            fn take_video_frame(&mut self) -> Result<Option<VideoFrame>, CoreError> {
                Ok(Some(
                    VideoFrame::new(1, 1, crate::core_io::PixelFormat::Rgba8888, vec![0; 4])
                        .unwrap(),
                ))
            }

            fn take_audio_buffer(&mut self) -> Result<Option<AudioBuffer>, CoreError> {
                Ok(Some(AudioBuffer::new(48_000, 1, vec![0]).unwrap()))
            }
        }

        let mut session = EmulationSession::new(Box::new(UnderreportedCore));
        session.load_content(&[1]).unwrap();

        assert_eq!(
            session.send_input(InputEvent {
                control: InputControl::Action1,
                pressed: true,
            }),
            Err(SessionError::Core(CoreError::Unsupported("input")))
        );
        assert_eq!(
            session.take_video_frame(),
            Err(SessionError::Core(CoreError::Unsupported("video output")))
        );
        assert_eq!(
            session.take_audio_buffer(),
            Err(SessionError::Core(CoreError::Unsupported("audio output")))
        );
        assert!(!session.is_input_pressed(InputControl::Action1));
    }

    #[test]
    fn media_cannot_be_retrieved_before_content_is_loaded() {
        let mut session = session();
        assert!(session.take_video_frame().is_err());
        assert!(session.take_audio_buffer().is_err());
        session.load_content(&[1]).unwrap();
        assert!(matches!(
            session.take_video_frame(),
            Err(SessionError::Core(CoreError::Unsupported("video output")))
        ));
    }
}

/// Errors returned while selecting or creating a core for a runtime session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    /// The requested core could not be found or constructed correctly.
    Registry(crate::RegistryError),
    /// The core requires a contract API version this host does not support.
    IncompatibleApiVersion {
        /// Contract version required by the core.
        required: crate::CoreApiVersion,
        /// Contract version implemented by this host.
        supported: crate::CoreApiVersion,
    },
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry(error) => fmt::Display::fmt(error, formatter),
            Self::IncompatibleApiVersion {
                required,
                supported,
            } => write!(
                formatter,
                "core contract API {}.{} is incompatible with host API {}.{}",
                required.major, required.minor, supported.major, supported.minor
            ),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<crate::RegistryError> for RuntimeError {
    fn from(error: crate::RegistryError) -> Self {
        Self::Registry(error)
    }
}

/// Shared, console-independent entry point for discovering cores and opening sessions.
///
/// The runtime owns the registry, not active sessions. Each call to
/// `create_session` constructs a fresh core, so separate sessions never
/// accidentally share mutable machine state.
#[derive(Default)]
pub struct EmulationRuntime {
    registry: crate::CoreRegistry,
}

impl EmulationRuntime {
    /// Creates an empty runtime with no installed core factories.
    pub const fn new() -> Self {
        Self {
            registry: crate::CoreRegistry::new(),
        }
    }

    /// Registers a core factory under its stable system identifier.
    pub fn register_core(
        &mut self,
        id: crate::SystemId,
        factory: crate::CoreFactory,
    ) -> Result<(), crate::RegistryError> {
        self.registry.register(id, factory)
    }

    /// Returns registered system identifiers in stable registration order.
    pub fn available_systems(&self) -> impl Iterator<Item = crate::SystemId> + '_ {
        self.registry.system_ids()
    }

    /// Creates a fresh, initially empty session for a registered system.
    pub fn create_session(&self, id: crate::SystemId) -> Result<EmulationSession, RuntimeError> {
        let core = self.registry.create(id).map_err(|error| match error {
            crate::RegistryError::IncompatibleApiVersion {
                required,
                supported,
            } => RuntimeError::IncompatibleApiVersion {
                required,
                supported,
            },
            other => RuntimeError::Registry(other),
        })?;
        Ok(EmulationSession::new(core))
    }
}

#[cfg(test)]
mod runtime_manager_tests {
    use super::*;
    use crate::{CoreCapabilities, CoreDescriptor, CoreError, CoreEvent, EmulationCore, SystemId};

    struct ManagerTestCore {
        api_version: crate::CoreApiVersion,
    }

    impl EmulationCore for ManagerTestCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: self.api_version,
                system_id: SystemId::new("manager-test").unwrap(),
                display_name: "Manager test core",
                core_version: "0.0.0-test",
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

    fn manager_test_factory() -> Box<dyn EmulationCore> {
        Box::new(ManagerTestCore {
            api_version: crate::CURRENT_CORE_API_VERSION,
        })
    }

    #[test]
    fn runtime_lists_registered_systems_and_opens_fresh_sessions() {
        let id = SystemId::new("manager-test").unwrap();
        let mut runtime = EmulationRuntime::new();
        assert_eq!(runtime.available_systems().count(), 0);
        runtime.register_core(id, manager_test_factory).unwrap();

        assert_eq!(runtime.available_systems().collect::<Vec<_>>(), vec![id]);
        let first = runtime.create_session(id).unwrap();
        let second = runtime.create_session(id).unwrap();
        assert_eq!(first.state(), SessionState::Empty);
        assert_eq!(second.state(), SessionState::Empty);
        assert_eq!(first.descriptor().system_id, id);
        assert_eq!(second.descriptor().system_id, id);
    }

    fn incompatible_manager_test_factory() -> Box<dyn EmulationCore> {
        Box::new(ManagerTestCore {
            api_version: crate::CoreApiVersion { major: 2, minor: 0 },
        })
    }

    #[test]
    fn runtime_rejects_incompatible_core_contract_versions() {
        let id = SystemId::new("manager-test").unwrap();
        let mut runtime = EmulationRuntime::new();
        runtime
            .register_core(id, incompatible_manager_test_factory)
            .unwrap();

        assert_eq!(
            runtime.create_session(id).err(),
            Some(RuntimeError::IncompatibleApiVersion {
                required: crate::CoreApiVersion { major: 2, minor: 0 },
                supported: crate::CURRENT_CORE_API_VERSION,
            })
        );
    }

    #[test]
    fn runtime_reports_unknown_system_without_creating_a_session() {
        let runtime = EmulationRuntime::new();
        let id = SystemId::new("missing").unwrap();
        assert!(matches!(
            runtime.create_session(id),
            Err(RuntimeError::Registry(crate::RegistryError::UnknownSystem(found))) if found == id
        ));
    }
}

#[cfg(test)]
mod shared_input_runtime_tests {
    use super::*;
    use crate::{CoreCapabilities, SystemId};
    use std::sync::{Arc, Mutex};

    struct InputRecordingCore {
        events: Arc<Mutex<Vec<InputEvent>>>,
    }

    impl EmulationCore for InputRecordingCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: crate::CURRENT_CORE_API_VERSION,
                system_id: SystemId::new("input-recording").unwrap(),
                display_name: "Input recording test core",
                core_version: "0.0.0-test",
                capabilities: CoreCapabilities {
                    content_loading: true,
                    input: true,
                    ..CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
            if content.is_empty() {
                return Err(CoreError::InvalidContent("empty test content"));
            }
            Ok(())
        }

        fn reset(&mut self) -> Result<(), CoreError> {
            Ok(())
        }

        fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
            Ok(CoreEvent::Yielded)
        }

        fn send_input(&mut self, event: InputEvent) -> Result<(), CoreError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    #[test]
    fn session_deduplicates_input_and_releases_held_controls() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let core = InputRecordingCore {
            events: Arc::clone(&events),
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        let press = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        session.send_input(press).unwrap();
        session.send_input(press).unwrap();
        assert!(session.is_input_pressed(InputControl::Action1));
        assert_eq!(events.lock().unwrap().as_slice(), &[press]);

        session.release_all_inputs().unwrap();
        assert!(!session.is_input_pressed(InputControl::Action1));
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[
                press,
                InputEvent {
                    control: InputControl::Action1,
                    pressed: false,
                },
            ]
        );
        session.release_all_inputs().unwrap();
        assert_eq!(events.lock().unwrap().len(), 2);
    }

    #[test]
    fn oversized_bounded_reload_preserves_paused_input_state() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let core = InputRecordingCore {
            events: Arc::clone(&events),
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();
        let press = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        session.send_input(press).unwrap();
        session.pause().unwrap();

        assert_eq!(
            session.load_content_bounded(&[1, 2], ContentLimits::new(1)),
            Err(SessionError::Content(ContentError::TooLarge {
                max_bytes: 1,
                actual_bytes: 2,
            }))
        );
        assert_eq!(session.state(), SessionState::Paused);
        assert!(session.is_input_pressed(InputControl::Action1));
        assert_eq!(events.lock().unwrap().as_slice(), &[press]);

        session.resume().unwrap();
        assert_eq!(session.run_until_event(), Ok(CoreEvent::Yielded));
    }

    #[test]
    fn sessions_keep_input_state_and_events_independent() {
        let first_events = Arc::new(Mutex::new(Vec::new()));
        let second_events = Arc::new(Mutex::new(Vec::new()));
        let mut first = EmulationSession::new(Box::new(InputRecordingCore {
            events: Arc::clone(&first_events),
        }));
        let mut second = EmulationSession::new(Box::new(InputRecordingCore {
            events: Arc::clone(&second_events),
        }));
        first.load_content(&[1]).unwrap();
        second.load_content(&[1]).unwrap();

        first
            .send_input(InputEvent {
                control: InputControl::Action1,
                pressed: true,
            })
            .unwrap();

        assert!(first.is_input_pressed(InputControl::Action1));
        assert!(!second.is_input_pressed(InputControl::Action1));
        assert_eq!(first_events.lock().unwrap().len(), 1);
        assert!(second_events.lock().unwrap().is_empty());

        second
            .send_input(InputEvent {
                control: InputControl::Start,
                pressed: true,
            })
            .unwrap();
        assert!(second.is_input_pressed(InputControl::Start));
        assert!(!first.is_input_pressed(InputControl::Start));
        assert_eq!(first_events.lock().unwrap().len(), 1);
        assert_eq!(second_events.lock().unwrap().len(), 1);
    }

    struct RejectSpecificInputCore {
        rejected_event: InputEvent,
        reset_fails: bool,
    }

    impl EmulationCore for RejectSpecificInputCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: crate::CURRENT_CORE_API_VERSION,
                system_id: SystemId::new("reject-input-test").unwrap(),
                display_name: "Reject input test core",
                core_version: "0.0.0-test",
                capabilities: CoreCapabilities {
                    content_loading: true,
                    input: true,
                    ..CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
            if content.is_empty() {
                return Err(CoreError::InvalidContent("empty test content"));
            }
            Ok(())
        }

        fn reset(&mut self) -> Result<(), CoreError> {
            if self.reset_fails {
                return Err(CoreError::Execution("reset rejected for test"));
            }
            Ok(())
        }

        fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
            Ok(CoreEvent::Yielded)
        }

        fn send_input(&mut self, event: InputEvent) -> Result<(), CoreError> {
            if event == self.rejected_event {
                return Err(CoreError::Execution("input rejected for test"));
            }
            Ok(())
        }
    }

    #[test]
    fn failed_stop_reconciles_released_inputs_without_stopping_session() {
        let rejected_release = InputEvent {
            control: InputControl::Action2,
            pressed: false,
        };
        let core = RejectSpecificInputCore {
            rejected_event: rejected_release,
            reset_fails: false,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        let first_press = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        let second_press = InputEvent {
            control: InputControl::Action2,
            pressed: true,
        };
        session.send_input(first_press).unwrap();
        session.send_input(second_press).unwrap();

        assert_eq!(
            session.stop(),
            Err(SessionError::Core(CoreError::Execution(
                "input rejected for test"
            )))
        );
        assert_eq!(session.state(), SessionState::Running);
        assert!(!session.is_input_pressed(InputControl::Action1));
        assert!(session.is_input_pressed(InputControl::Action2));
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("input rejected for test"))
        );
    }

    #[test]
    fn failed_input_press_does_not_mark_control_as_held() {
        let press = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        let core = RejectSpecificInputCore {
            rejected_event: press,
            reset_fails: false,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        assert_eq!(
            session.send_input(press),
            Err(SessionError::Core(CoreError::Execution(
                "input rejected for test"
            )))
        );
        assert!(!session.is_input_pressed(InputControl::Action1));
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("input rejected for test"))
        );
    }

    #[test]
    fn successful_input_after_failure_preserves_last_core_error() {
        let rejected_press = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        let accepted_press = InputEvent {
            control: InputControl::Action2,
            pressed: true,
        };
        let core = RejectSpecificInputCore {
            rejected_event: rejected_press,
            reset_fails: false,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        assert!(matches!(
            session.send_input(rejected_press),
            Err(SessionError::Core(CoreError::Execution(
                "input rejected for test"
            )))
        ));
        session.send_input(accepted_press).unwrap();

        assert!(!session.is_input_pressed(InputControl::Action1));
        assert!(session.is_input_pressed(InputControl::Action2));
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("input rejected for test"))
        );
    }

    #[test]
    fn clearing_core_diagnostic_preserves_session_and_held_input_state() {
        let rejected_press = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        let accepted_press = InputEvent {
            control: InputControl::Action2,
            pressed: true,
        };
        let core = RejectSpecificInputCore {
            rejected_event: rejected_press,
            reset_fails: false,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        assert!(session.send_input(rejected_press).is_err());
        session.send_input(accepted_press).unwrap();
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("input rejected for test"))
        );
        session.pause().unwrap();
        let state_before_clear = session.state();

        session.clear_last_core_error();

        assert_eq!(session.last_core_error(), None);
        assert_eq!(state_before_clear, SessionState::Paused);
        assert_eq!(session.state(), state_before_clear);
        assert!(session.is_input_pressed(InputControl::Action2));
        assert!(!session.is_input_pressed(InputControl::Action1));
        assert_eq!(
            session.run_until_event(),
            Err(SessionError::InvalidTransition {
                operation: "run",
                state: SessionState::Paused,
            })
        );
        session.resume().unwrap();
        assert_eq!(session.run_until_event(), Ok(CoreEvent::Yielded));
    }

    #[test]
    fn cleared_core_diagnostic_is_recorded_again_after_a_new_failure() {
        let rejected_press = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        let core = RejectSpecificInputCore {
            rejected_event: rejected_press,
            reset_fails: false,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        assert!(session.send_input(rejected_press).is_err());
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("input rejected for test"))
        );

        session.clear_last_core_error();
        assert_eq!(session.last_core_error(), None);

        assert!(session.send_input(rejected_press).is_err());
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("input rejected for test"))
        );
        assert_eq!(session.state(), SessionState::Running);
        assert!(!session.is_input_pressed(InputControl::Action1));
    }

    #[test]
    fn failed_input_release_keeps_control_marked_as_held() {
        let release = InputEvent {
            control: InputControl::Action1,
            pressed: false,
        };
        let core = RejectSpecificInputCore {
            rejected_event: release,
            reset_fails: false,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();
        let press = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        session.send_input(press).unwrap();

        assert_eq!(
            session.release_all_inputs(),
            Err(SessionError::Core(CoreError::Execution(
                "input rejected for test"
            )))
        );
        assert!(session.is_input_pressed(InputControl::Action1));
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("input rejected for test"))
        );
    }

    #[test]
    fn failed_bulk_release_preserves_only_controls_not_yet_released() {
        let rejected_release = InputEvent {
            control: InputControl::Action2,
            pressed: false,
        };
        let core = RejectSpecificInputCore {
            rejected_event: rejected_release,
            reset_fails: false,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();

        for control in [InputControl::Action1, InputControl::Action2] {
            session
                .send_input(InputEvent {
                    control,
                    pressed: true,
                })
                .unwrap();
        }

        assert_eq!(
            session.release_all_inputs(),
            Err(SessionError::Core(CoreError::Execution(
                "input rejected for test"
            )))
        );
        assert!(!session.is_input_pressed(InputControl::Action1));
        assert!(session.is_input_pressed(InputControl::Action2));
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("input rejected for test"))
        );
    }

    #[test]
    fn failed_stop_keeps_state_and_only_unreleased_controls_held() {
        let rejected_release = InputEvent {
            control: InputControl::Action2,
            pressed: false,
        };
        let core = RejectSpecificInputCore {
            rejected_event: rejected_release,
            reset_fails: false,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();
        for control in [InputControl::Action1, InputControl::Action2] {
            session
                .send_input(InputEvent {
                    control,
                    pressed: true,
                })
                .unwrap();
        }

        assert_eq!(
            session.stop(),
            Err(SessionError::Core(CoreError::Execution(
                "input rejected for test"
            )))
        );
        assert_eq!(session.state(), SessionState::Running);
        assert!(!session.is_input_pressed(InputControl::Action1));
        assert!(session.is_input_pressed(InputControl::Action2));
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("input rejected for test"))
        );
    }

    #[test]
    fn failed_reset_preserves_held_input_for_recovery() {
        let press = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        let core = RejectSpecificInputCore {
            rejected_event: InputEvent {
                control: InputControl::Start,
                pressed: true,
            },
            reset_fails: true,
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();
        session.send_input(press).unwrap();
        session.pause().unwrap();

        assert_eq!(
            session.reset(),
            Err(SessionError::Core(CoreError::Execution(
                "reset rejected for test"
            )))
        );
        assert_eq!(session.state(), SessionState::Paused);
        assert!(session.is_input_pressed(InputControl::Action1));
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Execution("reset rejected for test"))
        );
    }

    #[test]
    fn failed_content_reload_preserves_held_input_state() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let core = InputRecordingCore {
            events: Arc::clone(&events),
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();
        let press = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        session.send_input(press).unwrap();

        assert!(session.load_content(&[]).is_err());

        assert_eq!(session.state(), SessionState::Running);
        assert!(session.is_input_pressed(InputControl::Action1));
        assert_eq!(events.lock().unwrap().as_slice(), &[press]);
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::InvalidContent("empty test content"))
        );
    }

    #[test]
    fn successful_reset_clears_host_input_state() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let core = InputRecordingCore {
            events: Arc::clone(&events),
        };
        let mut session = EmulationSession::new(Box::new(core));
        session.load_content(&[1]).unwrap();
        session
            .send_input(InputEvent {
                control: InputControl::Start,
                pressed: true,
            })
            .unwrap();

        session.reset().unwrap();

        assert!(!session.is_input_pressed(InputControl::Start));
        assert_eq!(events.lock().unwrap().len(), 1);
    }
    #[test]
    fn not_initialized_core_error_is_retained_and_session_recovers() {
        struct NotInitializedOnceCore {
            fail_next_run: bool,
        }

        impl EmulationCore for NotInitializedOnceCore {
            fn descriptor(&self) -> CoreDescriptor {
                CoreDescriptor {
                    api_version: crate::CURRENT_CORE_API_VERSION,
                    system_id: SystemId::new("not-initialized-once-test").unwrap(),
                    display_name: "Not-initialized-once test core",
                    core_version: "0.0.0-test",
                    capabilities: CoreCapabilities {
                        content_loading: true,
                        ..CoreCapabilities::default()
                    },
                }
            }

            fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
                if content.is_empty() {
                    return Err(CoreError::InvalidContent("empty test content"));
                }
                Ok(())
            }

            fn reset(&mut self) -> Result<(), CoreError> {
                self.fail_next_run = false;
                Ok(())
            }

            fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
                if self.fail_next_run {
                    self.fail_next_run = false;
                    return Err(CoreError::NotInitialized);
                }
                Ok(CoreEvent::Yielded)
            }
        }

        let mut session = EmulationSession::new(Box::new(NotInitializedOnceCore {
            fail_next_run: true,
        }));
        session.load_content(&[1]).unwrap();

        assert_eq!(
            session.run_until_event(),
            Err(SessionError::Core(CoreError::NotInitialized))
        );
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.last_core_error(), Some(CoreError::NotInitialized));

        assert_eq!(session.run_until_event(), Ok(CoreEvent::Yielded));
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.last_core_error(), Some(CoreError::NotInitialized));
    }
    #[test]
    fn advertised_save_state_unsupported_error_is_retained() {
        struct UnsupportedSaveCore;

        impl EmulationCore for UnsupportedSaveCore {
            fn descriptor(&self) -> CoreDescriptor {
                CoreDescriptor {
                    api_version: crate::CURRENT_CORE_API_VERSION,
                    system_id: SystemId::new("unsupported-save-test").unwrap(),
                    display_name: "Unsupported save test core",
                    core_version: "0.0.0-test",
                    capabilities: CoreCapabilities {
                        content_loading: true,
                        save_states: true,
                        ..CoreCapabilities::default()
                    },
                }
            }

            fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
                if content.is_empty() {
                    return Err(CoreError::InvalidContent("empty test content"));
                }
                Ok(())
            }

            fn reset(&mut self) -> Result<(), CoreError> {
                Ok(())
            }

            fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
                Ok(CoreEvent::Yielded)
            }
        }

        let mut session = EmulationSession::new(Box::new(UnsupportedSaveCore));
        session.load_content(&[1]).unwrap();

        assert_eq!(
            session.save_state(),
            Err(SessionError::Core(CoreError::Unsupported("save states")))
        );
        assert_eq!(
            session.last_core_error(),
            Some(CoreError::Unsupported("save states"))
        );
        assert_eq!(session.state(), SessionState::Running);
    }
}

#[cfg(test)]
mod save_state_session_tests {
    use super::*;
    use crate::{CoreCapabilities, SystemId};

    struct StateCore {
        state: Vec<u8>,
    }

    impl EmulationCore for StateCore {
        fn descriptor(&self) -> CoreDescriptor {
            CoreDescriptor {
                api_version: crate::CURRENT_CORE_API_VERSION,
                system_id: SystemId::new("state-test").unwrap(),
                display_name: "Save state test core",
                core_version: "2.3.4",
                capabilities: CoreCapabilities {
                    content_loading: true,
                    save_states: true,
                    ..CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
            if content.is_empty() {
                return Err(CoreError::InvalidContent("empty test content"));
            }
            self.state = content.to_vec();
            Ok(())
        }

        fn reset(&mut self) -> Result<(), CoreError> {
            self.state.clear();
            Ok(())
        }

        fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
            Ok(CoreEvent::Yielded)
        }

        fn save_state(&self) -> Result<Vec<u8>, CoreError> {
            Ok(self.state.clone())
        }

        fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError> {
            if state.is_empty() {
                return Err(CoreError::InvalidState("empty test state"));
            }
            self.state = state.to_vec();
            Ok(())
        }
    }

    #[test]
    fn frame_cadence_discards_paused_time_and_resumes_without_backlog() {
        let mut session = EmulationSession::new(Box::new(StateCore { state: Vec::new() }));
        let mut cadence = FrameCadence::new(60, 3).unwrap();
        session.load_content(&[1]).unwrap();

        assert_eq!(
            session.advance_frame_cadence(&mut cadence, std::time::Duration::from_millis(8)),
            CadenceAdvance::default()
        );
        session.pause().unwrap();
        assert_eq!(
            session.advance_frame_cadence(&mut cadence, std::time::Duration::from_secs(10)),
            CadenceAdvance::default()
        );
        session.resume().unwrap();
        assert_eq!(
            session.advance_frame_cadence(&mut cadence, std::time::Duration::from_millis(17)),
            CadenceAdvance {
                due_frames: 1,
                dropped_frames: 0,
            }
        );

        session.stop().unwrap();
        assert_eq!(
            session.advance_frame_cadence(&mut cadence, std::time::Duration::from_secs(2)),
            CadenceAdvance::default()
        );
    }

    #[test]
    fn session_save_envelope_round_trips_through_core_restore() {
        let mut session = EmulationSession::new(Box::new(StateCore { state: Vec::new() }));
        session.load_content(&[1, 2, 3]).unwrap();
        let envelope = session.save_state_envelope().unwrap();

        session.load_content(&[9]).unwrap();
        session.load_state_envelope(&envelope).unwrap();

        assert_eq!(session.save_state().unwrap(), &[1, 2, 3]);
        assert_eq!(session.state(), SessionState::Running);
    }

    #[test]
    fn successful_restore_preserves_paused_lifecycle() {
        let mut session = EmulationSession::new(Box::new(StateCore { state: Vec::new() }));
        session.load_content(&[1, 2]).unwrap();
        session.pause().unwrap();
        session.load_state(&[3, 4, 5]).unwrap();
        assert_eq!(session.save_state().unwrap(), &[3, 4, 5]);
        assert_eq!(session.state(), SessionState::Paused);
    }

    #[test]
    fn rejected_state_preserves_machine_and_running_lifecycle() {
        let mut session = EmulationSession::new(Box::new(StateCore { state: Vec::new() }));
        session.load_content(&[7, 8]).unwrap();

        assert_eq!(
            session.load_state(&[]),
            Err(SessionError::Core(CoreError::InvalidState(
                "empty test state"
            )))
        );
        assert_eq!(session.save_state().unwrap(), &[7, 8]);
        assert_eq!(session.state(), SessionState::Running);
    }

    #[test]
    fn rejected_state_preserves_machine_and_paused_lifecycle() {
        let mut session = EmulationSession::new(Box::new(StateCore { state: Vec::new() }));
        session.load_content(&[7, 8]).unwrap();
        session.pause().unwrap();

        assert_eq!(
            session.load_state(&[]),
            Err(SessionError::Core(CoreError::InvalidState(
                "empty test state"
            )))
        );
        assert_eq!(session.save_state().unwrap(), &[7, 8]);
        assert_eq!(session.state(), SessionState::Paused);
    }
    #[test]
    fn incompatible_envelope_is_rejected_before_core_restore() {
        let mut session = EmulationSession::new(Box::new(StateCore { state: Vec::new() }));
        session.load_content(&[7, 8]).unwrap();
        let incompatible = SaveStateEnvelope::new("another-system", "2.3.4", vec![4, 5]).unwrap();

        assert_eq!(
            session.load_state_envelope(&incompatible),
            Err(SessionError::SaveState(SaveStateError::IncompatibleSystem))
        );
        assert_eq!(session.save_state().unwrap(), &[7, 8]);
    }
}
