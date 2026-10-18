#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Frontend-independent application state shared by terminal and desktop hosts.

use std::error::Error;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use emulation_core::content::{detect_content_formats, ContentFormatProbe, ContentLimits};
use emulation_core::core_io::{AudioBuffer, FrameCadence, InputEvent, VideoFrame};
use emulation_core::host_runtime::{HostRuntime, HostRuntimeError, SessionId};
use emulation_core::runtime::{SessionError, SessionState};
use emulation_core::CoreRegistry;

use crate::library::{
    ContentEntry, ContentFormatDetection, ContentLibrary, ContentLibraryError, ContentValidation,
};

/// Lifecycle command accepted by the shared application controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionCommand {
    /// Pause a running session.
    Pause,
    /// Resume a paused session.
    Resume,
    /// Stop execution and release held controls.
    Stop,
    /// Reset a loaded session and resume execution.
    Reset,
}

/// Failure while sending a typed command to a session.
#[derive(Debug)]
pub enum SessionCommandError {
    /// The requested session does not exist.
    Host(HostRuntimeError),
    /// The session rejected the command for its current state or core contract.
    Session(SessionError),
}

impl fmt::Display for SessionCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(error) => fmt::Display::fmt(error, f),
            Self::Session(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl Error for SessionCommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Host(error) => Some(error),
            Self::Session(error) => Some(error),
        }
    }
}

/// A bounded update produced by the application backend for a desktop frame tick.
#[derive(Debug)]
pub struct SessionAdvance {
    /// Session state after execution and media collection.
    pub state: SessionState,
    /// Whether the active core advertises video output.
    pub video_available: bool,
    /// Whether the active core advertises audio output.
    pub audio_available: bool,
    /// Newest video frame collected during this tick, if any.
    pub newest_frame: Option<VideoFrame>,
    /// Audio buffers collected from the core during this tick, in playback order.
    pub audio_buffers: Vec<AudioBuffer>,
    /// First execution or media error observed during this tick.
    pub error: Option<String>,
}

/// Read-only session information intended for frontend view models.
#[derive(Clone, Debug)]
pub struct SessionSummary {
    /// Stable session identifier.
    pub id: SessionId,
    /// Current lifecycle state.
    pub state: SessionState,
    /// Whether content has been loaded into this session.
    pub content_loaded: bool,
    /// Human-readable name of the registered core/system.
    pub system_name: String,
    /// Whether the core advertises video output.
    pub video_available: bool,
    /// Whether the core advertises audio output.
    pub audio_available: bool,
    /// Whether the core supports complete machine save states.
    pub save_states: bool,
    /// Whether the core supports battery-backed persistent game saves.
    pub persistent_saves: bool,
    /// Most recent core error, if any.
    pub last_error: Option<String>,
}

/// Detects formats with an adaptive prefix while respecting the launch size limit.
/// Probes can request more bytes than the initial prefix; satisfy the largest
/// request when it fits within the configured limit, rather than leaving a
/// resolvable format permanently marked as needing more data.
fn detect_content_formats_from_path(
    path: &Path,
    limits: ContentLimits,
    probes: &[&dyn ContentFormatProbe],
) -> Option<emulation_core::content::ContentDetection> {
    // Keep one file handle for every staged read. Reopening the path for each
    // probe request could combine bytes from different files if the path is
    // replaced between reads.
    let mut file = std::fs::File::open(path).ok()?;
    let mut prefix = Vec::with_capacity(limits.max_bytes().min(4096));
    let initial_limit = limits.max_bytes().min(4096);
    file.by_ref()
        .take(initial_limit as u64)
        .read_to_end(&mut prefix)
        .ok()?;

    loop {
        let detection = detect_content_formats(&prefix, probes);
        if detection.needs_more_data.is_empty() {
            return Some(detection);
        }
        let requested_bytes = detection
            .needs_more_data
            .iter()
            .map(|(_, minimum_bytes)| *minimum_bytes)
            .max()
            .unwrap_or(prefix.len());
        if requested_bytes <= prefix.len() || requested_bytes > limits.max_bytes() {
            return Some(detection);
        }

        let previous_len = prefix.len();
        let additional_bytes = requested_bytes - previous_len;
        file.by_ref()
            .take(additional_bytes as u64)
            .read_to_end(&mut prefix)
            .ok()?;
        if prefix.len() <= previous_len {
            // The file ended before the requested signature bytes were available.
            return Some(detect_content_formats(&prefix, probes));
        }
    }
}

/// Converts probe results without claiming a definitive match while any probe is undecided.
fn content_format_detection(
    detection: emulation_core::content::ContentDetection,
) -> ContentFormatDetection {
    if !detection.needs_more_data.is_empty() {
        ContentFormatDetection::NeedsMoreData
    } else if detection.matches.len() > 1 {
        ContentFormatDetection::Ambiguous(
            detection
                .matches
                .iter()
                .map(|format| (*format).to_owned())
                .collect(),
        )
    } else if let Some(format) = detection.matches.first() {
        ContentFormatDetection::Detected((*format).to_owned())
    } else {
        ContentFormatDetection::NoMatch
    }
}

/// Owns the state that every host-app frontend must share.
pub struct ApplicationController {
    host: HostRuntime,
    library: ContentLibrary,
    active_session: Option<SessionId>,
}

/// Errors returned when a frontend asks the controller to launch a library item.
#[derive(Debug)]
pub enum ApplicationError {
    /// The requested one-based library index does not exist.
    UnknownLibraryEntry(usize),
    /// The shared host rejected a session operation.
    Host(HostRuntimeError),
}

impl fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLibraryEntry(index) => {
                write!(f, "no content library entry at index {index}")
            }
            Self::Host(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl Error for ApplicationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnknownLibraryEntry(_) => None,
            Self::Host(error) => Some(error),
        }
    }
}

impl From<HostRuntimeError> for ApplicationError {
    fn from(error: HostRuntimeError) -> Self {
        Self::Host(error)
    }
}

impl ApplicationController {
    /// Creates a frontend-neutral application state around an empty host runtime.
    pub fn new(host: HostRuntime) -> Self {
        Self {
            host,
            library: ContentLibrary::new(),
            active_session: None,
        }
    }

    /// Borrows the host runtime for read-only frontend diagnostics.
    pub fn host(&self) -> &HostRuntime {
        &self.host
    }

    /// Mutably borrows the host runtime for capability-specific commands.
    ///
    /// Prefer controller lifecycle methods for launching, selecting, and closing
    /// sessions so that the active-session selection remains synchronized.
    pub fn host_mut(&mut self) -> &mut HostRuntime {
        &mut self.host
    }

    /// Borrows the content catalog.
    pub fn library(&self) -> &ContentLibrary {
        &self.library
    }

    /// Mutably borrows the content catalog for import and removal operations.
    pub fn library_mut(&mut self) -> &mut ContentLibrary {
        &mut self.library
    }

    /// Returns the number of currently open sessions.
    pub fn session_count(&self) -> usize {
        self.host.session_count()
    }

    /// Returns stable read-only summaries for every currently open session.
    ///
    /// Frontends should use these summaries instead of reaching into the runtime
    /// to inspect session internals while rendering views.
    pub fn session_summaries(&self) -> Vec<SessionSummary> {
        self.host
            .session_ids()
            .into_iter()
            .filter_map(|id| self.session_summary(id).ok())
            .collect()
    }

    /// Returns a read-only summary for a session, rejecting stale identifiers.
    pub fn session_summary(&self, id: SessionId) -> Result<SessionSummary, HostRuntimeError> {
        let session = self.host.session(id)?;
        let capabilities = session.descriptor().capabilities;
        Ok(SessionSummary {
            id,
            state: session.state(),
            content_loaded: session.state() != SessionState::Empty,
            system_name: session.descriptor().display_name.to_owned(),
            video_available: capabilities.video_output,
            audio_available: capabilities.audio_output,
            save_states: capabilities.save_states,
            persistent_saves: capabilities.persistent_saves,
            last_error: session.last_core_error().map(|error| error.to_string()),
        })
    }

    /// Returns the currently selected session, if any.
    pub const fn active_session(&self) -> Option<SessionId> {
        self.active_session
    }

    /// Returns the stable path at a one-based library index.
    pub fn library_entry_path(&self, index: usize) -> Option<PathBuf> {
        if index == 0 {
            return None;
        }
        self.library
            .entries()
            .nth(index - 1)
            .map(|entry| entry.path().to_path_buf())
    }

    /// Imports content into the catalog without starting an emulation session.
    pub fn import_content(
        &mut self,
        path: impl AsRef<Path>,
        max_bytes: u64,
    ) -> Result<&ContentEntry, ContentLibraryError> {
        self.library.import(path, max_bytes)
    }

    /// Removes a content entry without affecting any session already using it.
    pub fn forget_content(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<ContentEntry, ContentLibraryError> {
        self.library.remove(path)
    }

    /// Launches a content path through the shared validation pipeline and selects it.
    pub fn launch_content_path(
        &mut self,
        path: impl AsRef<Path>,
        limits: ContentLimits,
        probes: &[&dyn ContentFormatProbe],
        registry: &CoreRegistry,
    ) -> Result<SessionId, HostRuntimeError> {
        // If the file has disappeared since import, keep its absolute catalog key
        // so the failed launch still updates the entry's validation status.
        let canonical_path = std::fs::canonicalize(path.as_ref()).ok().or_else(|| {
            let path = path.as_ref();
            path.is_absolute().then(|| path.to_path_buf())
        });
        let detection = detect_content_formats_from_path(path.as_ref(), limits, probes);
        if let Some(canonical_path) = canonical_path.as_ref() {
            // A failed read must not leave a previous probe result looking current.
            // Pending means no bytes were available for this launch attempt.
            let detection = detection
                .map(content_format_detection)
                .unwrap_or(ContentFormatDetection::Pending);
            self.library
                .record_format_detection(canonical_path, detection);
        }
        let result = self
            .host
            .launch_content_path(path, limits, probes, registry);
        if let Some(canonical_path) = canonical_path {
            let validation = match &result {
                Ok(_) => ContentValidation::Accepted,
                Err(error) => ContentValidation::Rejected(error.to_string()),
            };
            self.library.record_validation(canonical_path, validation);
        }
        let id = result?;
        self.active_session = Some(id);
        Ok(id)
    }

    /// Launches a catalog item by its one-based index and selects its new session.
    pub fn launch_library_entry(
        &mut self,
        index: usize,
        limits: ContentLimits,
        probes: &[&dyn ContentFormatProbe],
        registry: &CoreRegistry,
    ) -> Result<SessionId, ApplicationError> {
        let path = self
            .library_entry_path(index)
            .ok_or(ApplicationError::UnknownLibraryEntry(index))?;
        self.launch_content_path(path, limits, probes, registry)
            .map_err(ApplicationError::Host)
    }

    /// Selects an existing session, rejecting stale IDs without changing selection.
    pub fn select_session(&mut self, id: SessionId) -> Result<(), HostRuntimeError> {
        self.host.session(id)?;
        self.active_session = Some(id);
        Ok(())
    }

    /// Closes a session and selects the oldest remaining session when necessary.
    pub fn close_session(&mut self, id: SessionId) -> Result<(), HostRuntimeError> {
        self.host.close_session(id)?;
        let active_selection_is_stale = self
            .active_session
            .is_some_and(|active_id| self.host.session(active_id).is_err());
        if self.active_session == Some(id) || active_selection_is_stale {
            self.active_session = self.host.session_ids().into_iter().next();
        }
        Ok(())
    }

    /// Closes every session and clears the active selection.
    pub fn close_all_sessions(&mut self) -> usize {
        self.active_session = None;
        self.host.close_all_sessions()
    }

    /// Advances one session tick and drains a bounded amount of media output.
    ///
    /// Scheduling, core execution, and queue draining stay behind the backend boundary;
    /// frontends receive only the latest frame and status needed to render the result.
    pub fn advance_session(
        &mut self,
        id: SessionId,
        elapsed: Duration,
        cadence: &mut FrameCadence,
    ) -> Result<SessionAdvance, HostRuntimeError> {
        let session = self.host.session_mut(id)?;
        let initial_state = session.state();
        let capabilities = session.descriptor().capabilities;
        let due = session.advance_frame_cadence(cadence, elapsed);
        let mut error = None;

        if initial_state == SessionState::Running {
            for _ in 0..due.due_frames {
                if let Err(run_error) = session.run_for_events(1024) {
                    error = Some(format!("Execution error: {run_error}"));
                    break;
                }
                if session.state() != SessionState::Running {
                    break;
                }
            }
        }

        let mut newest_frame = None;
        if capabilities.video_output && session.state() != SessionState::Empty {
            // Bound queue draining and retain only the newest frame for presentation.
            for _ in 0..64 {
                match session.take_video_frame() {
                    Ok(Some(frame)) => newest_frame = Some(frame),
                    Ok(None) => break,
                    Err(frame_error) => {
                        if error.is_none() {
                            error = Some(format!("Video output error: {frame_error}"));
                        }
                        break;
                    }
                }
            }
        }

        let mut audio_buffers = Vec::new();
        if capabilities.audio_output && session.state() != SessionState::Empty {
            // Bound per-tick work and preserve buffer order for the host audio device.
            for _ in 0..16 {
                match session.take_audio_buffer() {
                    Ok(Some(buffer)) => audio_buffers.push(buffer),
                    Ok(None) => break,
                    Err(audio_error) => {
                        if error.is_none() {
                            error = Some(format!("Audio output error: {audio_error}"));
                        }
                        break;
                    }
                }
            }
        }

        Ok(SessionAdvance {
            state: session.state(),
            video_available: capabilities.video_output,
            audio_available: capabilities.audio_output,
            newest_frame,
            audio_buffers,
            error,
        })
    }

    /// Returns whether a session has unsaved persistent game data.
    pub fn persistent_save_dirty(&self, id: SessionId) -> Result<bool, String> {
        self.host
            .session(id)
            .map(|session| session.persistent_save_dirty())
            .map_err(|error| error.to_string())
    }

    /// Creates a versioned envelope from a session's battery-backed game data.
    pub fn save_game_data_envelope(
        &mut self,
        id: SessionId,
    ) -> Result<emulation_core::save_state::SaveStateEnvelope, String> {
        let session = self
            .host
            .session_mut(id)
            .map_err(|error| error.to_string())?;
        let descriptor = session.descriptor();
        let system_id = descriptor.system_id.as_str().to_owned();
        let core_version = descriptor.core_version.to_owned();
        let data = session
            .save_game_data()
            .map_err(|error| error.to_string())?;
        emulation_core::save_state::SaveStateEnvelope::new(system_id, core_version, data)
            .map_err(|error| error.to_string())
    }

    /// Validates and loads battery-backed game data into a session.
    pub fn load_game_data_envelope(
        &mut self,
        id: SessionId,
        envelope: &emulation_core::save_state::SaveStateEnvelope,
    ) -> Result<(), String> {
        let session = self
            .host
            .session_mut(id)
            .map_err(|error| error.to_string())?;
        let descriptor = session.descriptor();
        envelope
            .validate_for(descriptor.system_id.as_str(), descriptor.core_version)
            .map_err(|error| error.to_string())?;
        session
            .load_game_data(envelope.payload())
            .map_err(|error| error.to_string())
    }

    /// Clears a session's persistent-save dirty flag after its data is safely stored.
    pub fn clear_persistent_save_dirty(&mut self, id: SessionId) -> Result<(), String> {
        self.host
            .session_mut(id)
            .map_err(|error| error.to_string())?
            .clear_persistent_save_dirty();
        Ok(())
    }

    /// Captures the current machine state in a versioned save-state envelope.
    pub fn save_state_envelope(
        &mut self,
        id: SessionId,
    ) -> Result<emulation_core::save_state::SaveStateEnvelope, String> {
        self.host
            .session_mut(id)
            .map_err(|error| error.to_string())?
            .save_state_envelope()
            .map_err(|error| error.to_string())
    }

    /// Validates and restores a versioned machine-state envelope.
    pub fn load_state_envelope(
        &mut self,
        id: SessionId,
        envelope: &emulation_core::save_state::SaveStateEnvelope,
    ) -> Result<(), String> {
        self.host
            .session_mut(id)
            .map_err(|error| error.to_string())?
            .load_state_envelope(envelope)
            .map_err(|error| error.to_string())
    }

    /// Sends a semantic input transition to a session through the shared controller.
    ///
    /// The session validates its own lifecycle state and core input capability.
    pub fn send_input(
        &mut self,
        id: SessionId,
        event: InputEvent,
    ) -> Result<(), SessionCommandError> {
        let session = self
            .host
            .session_mut(id)
            .map_err(SessionCommandError::Host)?;
        session
            .send_input(event)
            .map_err(SessionCommandError::Session)
    }

    /// Releases every held input on a session when its lifecycle permits input.
    pub fn release_inputs(&mut self, id: SessionId) -> Result<(), SessionCommandError> {
        let session = self
            .host
            .session_mut(id)
            .map_err(SessionCommandError::Host)?;
        if matches!(
            session.state(),
            SessionState::Running | SessionState::Paused
        ) {
            session
                .release_all_inputs()
                .map_err(SessionCommandError::Session)?;
        }
        Ok(())
    }

    /// Applies a typed lifecycle command without exposing the mutable host runtime.
    pub fn command_session(
        &mut self,
        id: SessionId,
        command: SessionCommand,
    ) -> Result<(), SessionCommandError> {
        let session = self
            .host
            .session_mut(id)
            .map_err(SessionCommandError::Host)?;
        let result = match command {
            SessionCommand::Pause => session.pause(),
            SessionCommand::Resume => session.resume(),
            SessionCommand::Stop => session.stop(),
            SessionCommand::Reset => session.reset(),
        };
        result.map_err(SessionCommandError::Session)
    }

    /// Provides disjoint mutable access for a frontend command dispatcher.
    ///
    /// Prefer the typed controller methods for lifecycle operations. This escape
    /// hatch exists for commands that need direct access to capability-specific APIs.
    pub fn parts_mut(
        &mut self,
    ) -> (
        &mut HostRuntime,
        &mut ContentLibrary,
        &mut Option<SessionId>,
    ) {
        (&mut self.host, &mut self.library, &mut self.active_session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_boy::GameBoyColorCore;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn controller() -> ApplicationController {
        ApplicationController::new(HostRuntime::new(Default::default()).unwrap())
    }

    #[test]
    fn save_controller_apis_report_stale_sessions_without_mutating_state() {
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();

        assert!(!app.persistent_save_dirty(id).unwrap());
        let envelope = emulation_core::save_state::SaveStateEnvelope::new(
            "game-boy-color".to_owned(),
            "test-version".to_owned(),
            vec![0],
        )
        .unwrap();

        app.close_session(id).unwrap();
        assert!(app.persistent_save_dirty(id).is_err());
        assert!(app.save_game_data_envelope(id).is_err());
        assert!(app.load_game_data_envelope(id, &envelope).is_err());
        assert!(app.clear_persistent_save_dirty(id).is_err());
        assert!(app.save_state_envelope(id).is_err());
        assert!(app.load_state_envelope(id, &envelope).is_err());
        assert_eq!(app.session_count(), 0);
        assert_eq!(app.active_session(), None);
    }

    #[test]
    fn failed_launch_of_removed_import_updates_library_validation() {
        let mut app = controller();
        let directory = std::env::temp_dir().join(format!(
            "host-app-controller-removed-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("removed.gb");
        std::fs::write(&path, b"rom-data").unwrap();
        let canonical = std::fs::canonicalize(&path).unwrap();
        app.import_content(&path, 64).unwrap();
        app.library_mut().record_format_detection(
            &canonical,
            ContentFormatDetection::Detected("previous-format".to_owned()),
        );
        std::fs::remove_file(&path).unwrap();

        let result = app.launch_content_path(
            &canonical,
            ContentLimits::new(64),
            &[],
            &CoreRegistry::new(),
        );
        assert!(result.is_err());
        let entry = app.library().get(&canonical).unwrap();
        assert!(matches!(entry.validation(), ContentValidation::Rejected(_)));
        assert_eq!(entry.format_detection(), &ContentFormatDetection::Pending);

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn format_detection_reads_more_bytes_when_a_probe_requests_them() {
        struct LongHeaderProbe;

        impl ContentFormatProbe for LongHeaderProbe {
            fn format_id(&self) -> &'static str {
                "long-header"
            }

            fn probe(&self, prefix: &[u8]) -> emulation_core::content::FormatProbeResult {
                if prefix.len() < 8192 {
                    emulation_core::content::FormatProbeResult::NeedMoreData {
                        minimum_bytes: 8192,
                    }
                } else if prefix[8191] == b'X' {
                    emulation_core::content::FormatProbeResult::Match
                } else {
                    emulation_core::content::FormatProbeResult::NoMatch
                }
            }
        }

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "host-app-long-probe-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("long-header.bin");
        let mut bytes = vec![b'A'; 8192];
        bytes[8191] = b'X';
        std::fs::write(&path, bytes).unwrap();

        let probe = LongHeaderProbe;
        let probes: [&dyn ContentFormatProbe; 1] = [&probe];
        let detection =
            detect_content_formats_from_path(&path, ContentLimits::new(16 * 1024), &probes)
                .unwrap();

        assert_eq!(detection.matches, vec!["long-header"]);
        assert!(detection.needs_more_data.is_empty());

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn format_detection_satisfies_staged_probe_requests() {
        struct StagedHeaderProbe;

        impl ContentFormatProbe for StagedHeaderProbe {
            fn format_id(&self) -> &'static str {
                "staged-header"
            }

            fn probe(&self, prefix: &[u8]) -> emulation_core::content::FormatProbeResult {
                if prefix.len() < 8192 {
                    emulation_core::content::FormatProbeResult::NeedMoreData {
                        minimum_bytes: 8192,
                    }
                } else if prefix.len() < 12 * 1024 {
                    emulation_core::content::FormatProbeResult::NeedMoreData {
                        minimum_bytes: 12 * 1024,
                    }
                } else if prefix[12 * 1024 - 1] == b'X' {
                    emulation_core::content::FormatProbeResult::Match
                } else {
                    emulation_core::content::FormatProbeResult::NoMatch
                }
            }
        }

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "host-app-staged-probe-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("staged-header.bin");
        let mut bytes = vec![b'A'; 12 * 1024];
        bytes[12 * 1024 - 1] = b'X';
        std::fs::write(&path, bytes).unwrap();

        let probe = StagedHeaderProbe;
        let probes: [&dyn ContentFormatProbe; 1] = [&probe];
        let detection =
            detect_content_formats_from_path(&path, ContentLimits::new(16 * 1024), &probes)
                .unwrap();

        assert_eq!(detection.matches, vec!["staged-header"]);
        assert!(detection.needs_more_data.is_empty());

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn format_detection_does_not_read_past_the_content_limit() {
        struct LongHeaderProbe;

        impl ContentFormatProbe for LongHeaderProbe {
            fn format_id(&self) -> &'static str {
                "long-header"
            }

            fn probe(&self, prefix: &[u8]) -> emulation_core::content::FormatProbeResult {
                emulation_core::content::FormatProbeResult::NeedMoreData {
                    minimum_bytes: prefix.len().saturating_add(1).max(8192),
                }
            }
        }

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "host-app-limited-probe-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("limited-header.bin");
        std::fs::write(&path, vec![b'A'; 8192]).unwrap();

        let probe = LongHeaderProbe;
        let probes: [&dyn ContentFormatProbe; 1] = [&probe];
        let detection =
            detect_content_formats_from_path(&path, ContentLimits::new(4096), &probes).unwrap();

        assert!(detection.matches.is_empty());
        assert_eq!(detection.needs_more_data, vec![("long-header", 8192)]);

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn format_detection_stays_incomplete_when_a_probe_needs_more_data() {
        let detection = emulation_core::content::ContentDetection {
            matches: vec!["known-format"],
            needs_more_data: vec![("possible-format", 8192)],
        };

        assert_eq!(
            content_format_detection(detection),
            ContentFormatDetection::NeedsMoreData
        );
    }

    #[test]
    fn format_detection_reports_ambiguity_when_multiple_probes_match() {
        let detection = emulation_core::content::ContentDetection {
            matches: vec!["format-a", "format-b"],
            needs_more_data: Vec::new(),
        };

        assert_eq!(
            content_format_detection(detection),
            ContentFormatDetection::Ambiguous(vec!["format-a".to_owned(), "format-b".to_owned()])
        );
    }

    #[test]
    fn format_detection_reports_no_match_only_when_all_probes_decide() {
        let detection = emulation_core::content::ContentDetection::default();

        assert_eq!(
            content_format_detection(detection),
            ContentFormatDetection::NoMatch
        );
    }

    #[test]
    fn selecting_and_closing_sessions_keeps_active_selection_consistent() {
        let mut app = controller();
        let first = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        let second = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();

        app.select_session(first).unwrap();
        app.select_session(second).unwrap();
        assert_eq!(app.active_session(), Some(second));

        app.close_session(second).unwrap();
        assert_eq!(app.active_session(), Some(first));
        app.close_session(first).unwrap();
        assert_eq!(app.active_session(), None);
        assert!(app.host().is_empty());
    }

    #[test]
    fn closing_a_non_active_session_preserves_selection_and_closed_ids_are_not_reused() {
        let mut app = controller();
        let first = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        let second = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();

        app.select_session(second).unwrap();
        app.close_session(first).unwrap();
        assert_eq!(app.active_session(), Some(second));

        let third = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        assert_ne!(third, first);

        app.close_session(second).unwrap();
        assert_eq!(app.active_session(), Some(third));
        app.close_session(third).unwrap();
        assert_eq!(app.active_session(), None);
    }

    #[test]
    fn closing_a_session_repairs_selection_if_an_escape_hatch_left_it_stale() {
        let mut app = controller();
        let first = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        let second = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(first).unwrap();

        // A capability-specific host operation can bypass controller bookkeeping.
        app.host_mut().close_session(first).unwrap();
        app.close_session(second).unwrap();

        assert_eq!(app.active_session(), None);
        assert!(app.session_summaries().is_empty());
    }

    #[test]
    fn closing_all_sessions_clears_multiple_sessions_and_is_idempotent() {
        let mut app = controller();
        let first = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        let second = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(second).unwrap();
        assert_eq!(app.session_count(), 2);

        assert_eq!(app.close_all_sessions(), 2);
        assert_eq!(app.active_session(), None);
        assert_eq!(app.session_count(), 0);
        assert!(app.session_summaries().is_empty());
        assert_eq!(app.close_all_sessions(), 0);

        // Closing all sessions also makes every previous identifier stale.
        assert!(app.select_session(first).is_err());
        assert!(app.select_session(second).is_err());
    }

    #[test]
    fn selecting_unknown_session_does_not_change_the_active_session() {
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        let stale = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(id).unwrap();
        app.close_session(stale).unwrap();

        assert!(app.select_session(stale).is_err());
        assert_eq!(app.active_session(), Some(id));
    }

    #[test]
    fn lifecycle_commands_reject_invalid_or_closed_sessions_without_changing_selection() {
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(id).unwrap();

        // An empty session cannot be paused; the rejection must not corrupt its state.
        assert!(app.command_session(id, SessionCommand::Pause).is_err());
        assert_eq!(app.active_session(), Some(id));
        assert_eq!(app.session_summary(id).unwrap().state, SessionState::Empty);

        app.close_session(id).unwrap();
        assert!(app.command_session(id, SessionCommand::Stop).is_err());
        assert_eq!(app.active_session(), None);
        assert!(app.session_summaries().is_empty());
    }

    #[test]
    fn advancing_a_closed_session_returns_an_error_without_changing_selection() {
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(id).unwrap();
        app.close_session(id).unwrap();
        let mut cadence = FrameCadence::new(60, 3).unwrap();

        assert!(app
            .advance_session(id, Duration::from_millis(17), &mut cadence)
            .is_err());
        assert_eq!(app.active_session(), None);
        assert_eq!(app.session_count(), 0);
    }

    #[test]
    fn session_summaries_expose_view_state_without_runtime_access() {
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(id).unwrap();

        let summaries = app.session_summaries();
        assert_eq!(app.session_count(), 1);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, id);
        assert_eq!(summaries[0].state, SessionState::Empty);
        assert!(!summaries[0].content_loaded);
        assert!(!summaries[0].system_name.is_empty());
        assert!(app.session_summary(id).is_ok());
        app.close_session(id).unwrap();
        assert!(app.session_summary(id).is_err());
        assert!(app.session_summaries().is_empty());
    }

    #[test]
    fn releasing_inputs_on_empty_session_is_safe_and_closed_session_is_reported() {
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();

        // An empty session cannot have held controls, so release is a safe no-op.
        app.release_inputs(id)
            .expect("releasing inputs from an empty session should be harmless");
        assert_eq!(app.session_summary(id).unwrap().state, SessionState::Empty);

        app.close_session(id).unwrap();
        assert!(
            app.release_inputs(id).is_err(),
            "a stale session ID must not be silently accepted"
        );
        assert_eq!(app.session_count(), 0);
        assert_eq!(app.active_session(), None);
    }

    struct AudioQueueCore {
        loaded: bool,
        pending_audio: VecDeque<AudioBuffer>,
        run_count: Arc<AtomicUsize>,
    }

    impl emulation_core::EmulationCore for AudioQueueCore {
        fn descriptor(&self) -> emulation_core::CoreDescriptor {
            emulation_core::CoreDescriptor {
                api_version: emulation_core::CURRENT_CORE_API_VERSION,
                system_id: emulation_core::SystemId::new("audio-test").unwrap(),
                display_name: "Audio test core",
                core_version: "0.0.0-test",
                capabilities: emulation_core::CoreCapabilities {
                    content_loading: true,
                    audio_output: true,
                    ..emulation_core::CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), emulation_core::CoreError> {
            if content.is_empty() {
                return Err(emulation_core::CoreError::InvalidContent(
                    "empty test content",
                ));
            }
            self.loaded = true;
            Ok(())
        }

        fn reset(&mut self) -> Result<(), emulation_core::CoreError> {
            self.loaded = true;
            Ok(())
        }

        fn run_until_event(
            &mut self,
        ) -> Result<emulation_core::CoreEvent, emulation_core::CoreError> {
            if !self.loaded {
                return Err(emulation_core::CoreError::NotInitialized);
            }
            self.run_count.fetch_add(1, Ordering::SeqCst);
            Ok(emulation_core::CoreEvent::Yielded)
        }

        fn take_audio_buffer(&mut self) -> Result<Option<AudioBuffer>, emulation_core::CoreError> {
            Ok(self.pending_audio.pop_front())
        }
    }

    struct FailingMediaCore {
        loaded: bool,
    }

    impl emulation_core::EmulationCore for FailingMediaCore {
        fn descriptor(&self) -> emulation_core::CoreDescriptor {
            emulation_core::CoreDescriptor {
                api_version: emulation_core::CURRENT_CORE_API_VERSION,
                system_id: emulation_core::SystemId::new("failing-media-test").unwrap(),
                display_name: "Failing media test core",
                core_version: "0.0.0-test",
                capabilities: emulation_core::CoreCapabilities {
                    content_loading: true,
                    video_output: true,
                    audio_output: true,
                    ..emulation_core::CoreCapabilities::default()
                },
            }
        }

        fn load_content(&mut self, content: &[u8]) -> Result<(), emulation_core::CoreError> {
            if content.is_empty() {
                return Err(emulation_core::CoreError::InvalidContent(
                    "empty test content",
                ));
            }
            self.loaded = true;
            Ok(())
        }

        fn reset(&mut self) -> Result<(), emulation_core::CoreError> {
            self.loaded = true;
            Ok(())
        }

        fn run_until_event(
            &mut self,
        ) -> Result<emulation_core::CoreEvent, emulation_core::CoreError> {
            if !self.loaded {
                return Err(emulation_core::CoreError::NotInitialized);
            }
            Err(emulation_core::CoreError::Execution(
                "primary execution failure",
            ))
        }

        fn take_video_frame(&mut self) -> Result<Option<VideoFrame>, emulation_core::CoreError> {
            Err(emulation_core::CoreError::Execution(
                "secondary video failure",
            ))
        }

        fn take_audio_buffer(&mut self) -> Result<Option<AudioBuffer>, emulation_core::CoreError> {
            Err(emulation_core::CoreError::Execution(
                "secondary audio failure",
            ))
        }
    }

    #[test]
    fn backend_tick_collects_core_audio_buffers_in_order() {
        let expected = AudioBuffer::new(48_000, 2, vec![0, 1, -1, 2]).unwrap();
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(AudioQueueCore {
                loaded: false,
                pending_audio: std::iter::once(expected.clone()).collect(),
                run_count: Arc::new(AtomicUsize::new(0)),
            }))
            .unwrap();
        app.host_mut()
            .session_mut(id)
            .unwrap()
            .load_content(&[1])
            .unwrap();
        let mut cadence = FrameCadence::new(60, 3).unwrap();

        let update = app
            .advance_session(id, Duration::ZERO, &mut cadence)
            .unwrap();

        assert!(update.audio_available);
        assert_eq!(update.audio_buffers, vec![expected]);
        assert!(update.error.is_none());
    }

    #[test]
    fn backend_tick_preserves_the_first_error_when_media_collection_also_fails() {
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(FailingMediaCore { loaded: false }))
            .unwrap();
        app.host_mut()
            .session_mut(id)
            .unwrap()
            .load_content(&[1])
            .unwrap();
        let mut cadence = FrameCadence::new(60, 3).unwrap();

        let update = app
            .advance_session(id, Duration::from_millis(17), &mut cadence)
            .unwrap();

        assert_eq!(update.state, SessionState::Running);
        assert_eq!(
            app.session_summary(id).unwrap().last_error.as_deref(),
            Some("emulation execution failed: secondary audio failure")
        );
        assert_eq!(
            update.error.as_deref(),
            Some("Execution error: emulation execution failed: primary execution failure")
        );
        assert!(update.video_available);
        assert!(update.audio_available);
    }

    #[test]
    fn backend_tick_bounds_audio_drain_and_preserves_remaining_buffer_order() {
        let buffers: Vec<_> = (0..20)
            .map(|sample| AudioBuffer::new(48_000, 1, vec![sample]).unwrap())
            .collect();
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(AudioQueueCore {
                loaded: false,
                pending_audio: buffers.clone().into_iter().collect(),
                run_count: Arc::new(AtomicUsize::new(0)),
            }))
            .unwrap();
        app.host_mut()
            .session_mut(id)
            .unwrap()
            .load_content(&[1])
            .unwrap();
        let mut cadence = FrameCadence::new(60, 3).unwrap();

        let first = app
            .advance_session(id, Duration::ZERO, &mut cadence)
            .unwrap();
        assert_eq!(first.audio_buffers, buffers[..16].to_vec());

        let second = app
            .advance_session(id, Duration::ZERO, &mut cadence)
            .unwrap();
        assert_eq!(second.audio_buffers, buffers[16..].to_vec());
    }

    #[test]
    fn media_session_does_not_advance_while_paused_and_resumes_cleanly() {
        let mut app = controller();
        let run_count = Arc::new(AtomicUsize::new(0));
        let id = app
            .host_mut()
            .create_session(Box::new(AudioQueueCore {
                loaded: false,
                pending_audio: VecDeque::new(),
                run_count: Arc::clone(&run_count),
            }))
            .unwrap();
        app.host_mut()
            .session_mut(id)
            .unwrap()
            .load_content(&[1])
            .unwrap();
        let mut cadence = FrameCadence::new(60, 3).unwrap();

        app.advance_session(id, Duration::from_millis(17), &mut cadence)
            .unwrap();
        let after_first_tick = run_count.load(Ordering::SeqCst);
        assert!(after_first_tick > 0, "a running session must advance");
        app.command_session(id, SessionCommand::Pause).unwrap();
        app.advance_session(id, Duration::from_secs(10), &mut cadence)
            .unwrap();
        assert_eq!(
            run_count.load(Ordering::SeqCst),
            after_first_tick,
            "a paused session must not execute additional core events"
        );

        app.command_session(id, SessionCommand::Resume).unwrap();
        app.advance_session(id, Duration::from_millis(17), &mut cadence)
            .unwrap();
        assert!(
            run_count.load(Ordering::SeqCst) > after_first_tick,
            "a resumed session must continue executing core events"
        );
    }

    #[test]
    fn backend_tick_keeps_empty_session_state_and_returns_no_frame() {
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        let mut cadence = FrameCadence::new(60, 3).unwrap();

        let update = app
            .advance_session(id, Duration::from_millis(16), &mut cadence)
            .unwrap();

        assert_eq!(update.state, SessionState::Empty);
        assert!(update.newest_frame.is_none());
        assert!(update.audio_buffers.is_empty());
        assert!(update.error.is_none());
    }

    #[test]
    fn typed_session_commands_report_invalid_transitions_without_panicking() {
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();

        for (command, expected_operation) in [
            (SessionCommand::Pause, "pause"),
            (SessionCommand::Resume, "resume"),
            (SessionCommand::Stop, "stop"),
            (SessionCommand::Reset, "reset"),
        ] {
            let error = app.command_session(id, command).unwrap_err();
            assert!(matches!(
                error,
                SessionCommandError::Session(SessionError::InvalidTransition {
                    operation,
                    state: SessionState::Empty,
                }) if operation == expected_operation
            ));
            assert_eq!(app.session_summary(id).unwrap().state, SessionState::Empty);
        }

        app.close_session(id).unwrap();
        let error = app.command_session(id, SessionCommand::Stop).unwrap_err();
        assert!(matches!(error, SessionCommandError::Host(_)));
    }

    #[test]
    fn releasing_inputs_on_an_empty_session_is_safe_and_idempotent() {
        let mut app = controller();
        let id = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.release_inputs(id).unwrap();
        app.release_inputs(id).unwrap();
    }

    #[test]
    fn failed_path_launch_does_not_create_a_phantom_session() {
        let mut app = controller();
        let missing = std::env::temp_dir().join(format!(
            "host-app-missing-launch-{}-never-created.gb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&missing);

        let result = app.launch_content_path(
            &missing,
            ContentLimits::new(1024),
            &[],
            &CoreRegistry::new(),
        );

        assert!(result.is_err());
        assert!(app.host().is_empty());
        assert_eq!(app.active_session(), None);
        let _ = std::fs::remove_file(missing);
    }

    #[test]
    fn failed_launch_preserves_existing_session_selection_and_library() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "host-app-failed-launch-preserve-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let library_file = root.join("library-item.gb");
        std::fs::write(&library_file, b"library-content").unwrap();
        let missing_file = root.join("missing-item.gb");

        let mut app = controller();
        app.import_content(&library_file, 1024).unwrap();
        let existing_session = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(existing_session).unwrap();

        let result = app.launch_content_path(
            &missing_file,
            ContentLimits::new(1024),
            &[],
            &CoreRegistry::new(),
        );

        assert!(result.is_err());
        assert_eq!(app.session_count(), 1);
        assert_eq!(app.active_session(), Some(existing_session));
        assert_eq!(app.library().len(), 1);
        assert_eq!(
            app.library().entries().next().unwrap().path(),
            std::fs::canonicalize(&library_file).unwrap()
        );

        app.close_all_sessions();
        std::fs::remove_file(library_file).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn empty_and_oversized_content_are_rejected_without_changing_app_state() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "host-app-invalid-content-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let empty_file = root.join("empty.gb");
        let oversized_file = root.join("oversized.gb");
        std::fs::write(&empty_file, b"").unwrap();
        std::fs::write(&oversized_file, b"12345").unwrap();

        let mut app = controller();
        let existing_session = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(existing_session).unwrap();

        for path in [&empty_file, &oversized_file] {
            let result =
                app.launch_content_path(path, ContentLimits::new(4), &[], &CoreRegistry::new());

            assert!(
                result.is_err(),
                "invalid content must be rejected: {}",
                path.display()
            );
            assert_eq!(app.session_count(), 1);
            assert_eq!(app.active_session(), Some(existing_session));
            assert_eq!(
                app.session_summary(existing_session).unwrap().state,
                SessionState::Empty
            );
        }

        app.close_all_sessions();
        std::fs::remove_file(empty_file).unwrap();
        std::fs::remove_file(oversized_file).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn ambiguous_format_rejection_does_not_create_a_session() {
        use emulation_core::content::FormatProbeResult;
        use std::time::{SystemTime, UNIX_EPOCH};

        struct AlwaysMatch(&'static str);

        impl ContentFormatProbe for AlwaysMatch {
            fn format_id(&self) -> &'static str {
                self.0
            }

            fn probe(&self, _prefix: &[u8]) -> FormatProbeResult {
                FormatProbeResult::Match
            }
        }

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "host-app-ambiguous-format-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let ambiguous_file = root.join("ambiguous.bin");
        std::fs::write(&ambiguous_file, b"content matched by two probes").unwrap();

        let first_probe = AlwaysMatch("format-one");
        let second_probe = AlwaysMatch("format-two");
        let probes: [&dyn ContentFormatProbe; 2] = [&first_probe, &second_probe];
        let mut app = controller();

        let result = app.launch_content_path(
            &ambiguous_file,
            ContentLimits::new(1024),
            &probes,
            &CoreRegistry::new(),
        );

        assert!(result.is_err(), "ambiguous content must be rejected");
        assert_eq!(app.session_count(), 0);
        assert_eq!(app.active_session(), None);
        assert!(app.session_summaries().is_empty());

        std::fs::remove_file(ambiguous_file).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn content_read_failure_does_not_create_a_session_or_change_selection() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "host-app-content-read-failure-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        // A directory with a content-like extension reliably exercises the host read failure
        // across Windows, macOS, and Linux without relying on platform-specific permissions.
        let unreadable_path = root.join("unreadable.gb");
        std::fs::create_dir(&unreadable_path).unwrap();

        let mut app = controller();
        let existing_session = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(existing_session).unwrap();

        let result = app.launch_content_path(
            &unreadable_path,
            ContentLimits::new(1024),
            &[],
            &CoreRegistry::new(),
        );

        assert!(result.is_err(), "a directory must not be loaded as content");
        assert_eq!(app.session_count(), 1);
        assert_eq!(app.active_session(), Some(existing_session));
        assert_eq!(
            app.session_summary(existing_session).unwrap().state,
            SessionState::Empty
        );

        app.close_all_sessions();
        std::fs::remove_dir(&unreadable_path).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn unknown_format_rejection_preserves_existing_session_selection() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "host-app-unknown-format-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let unknown_file = root.join("unknown.bin");
        std::fs::write(&unknown_file, b"not a recognized content format").unwrap();

        let mut app = controller();
        let existing_session = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(existing_session).unwrap();

        let result = app.launch_content_path(
            &unknown_file,
            ContentLimits::new(1024),
            &[],
            &CoreRegistry::new(),
        );

        assert!(result.is_err());
        assert_eq!(app.session_count(), 1);
        assert_eq!(app.active_session(), Some(existing_session));
        assert_eq!(
            app.session_summary(existing_session).unwrap().state,
            SessionState::Empty
        );

        app.close_all_sessions();
        std::fs::remove_file(unknown_file).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn core_rejection_preserves_existing_session_selection() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "emulation-core-rejection-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let malformed_rom = root.join("malformed.gb");
        let mut bytes = vec![0u8; 0x8000];
        bytes[0x0104..0x0134].copy_from_slice(&[
            0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C,
            0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6,
            0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC,
            0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
        ]);
        bytes[0x148] = 0;
        bytes[0x149] = 0;
        // Keep the default checksum byte invalid so the probe matches but the core rejects it.
        bytes[0x14D] = 0;
        std::fs::write(&malformed_rom, bytes).unwrap();

        let mut registry = CoreRegistry::new();
        registry
            .register(
                emulation_core::SystemId::new(game_boy::SYSTEM_ID).unwrap(),
                || Box::new(GameBoyColorCore::new()),
            )
            .unwrap();
        let probe = game_boy::GameBoyColorCartridgeProbe;

        let mut app = controller();
        let canonical_rom = app
            .import_content(&malformed_rom, 0x10000)
            .unwrap()
            .path()
            .to_path_buf();
        let existing_session = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(existing_session).unwrap();

        let result = app.launch_content_path(
            &malformed_rom,
            ContentLimits::new(0x10000),
            &[&probe],
            &registry,
        );

        assert!(result.is_err(), "the core must reject the malformed header");
        assert_eq!(app.session_count(), 1);
        assert_eq!(app.active_session(), Some(existing_session));
        assert_eq!(
            app.session_summary(existing_session).unwrap().state,
            SessionState::Empty
        );
        assert!(matches!(
            app.library().get(&canonical_rom).unwrap().validation(),
            ContentValidation::Rejected(_)
        ));
        assert!(matches!(
            app.library()
                .get(&canonical_rom)
                .unwrap()
                .format_detection(),
            ContentFormatDetection::Detected(_)
        ));

        app.close_all_sessions();
        std::fs::remove_file(malformed_rom).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn successful_library_launch_loads_content_and_supports_session_lifecycle() {
        use emulation_core::content::ContentFormatProbe;
        use game_boy::{register_game_boy_color, GameBoyColorCartridgeProbe};
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "host-app-successful-launch-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let rom_path = root.join("valid-test.gb");

        // Build a minimal 32 KiB ROM-only image with a valid logo and header checksum.
        let mut rom = vec![0u8; 0x8000];
        rom[0x0104..0x0134].copy_from_slice(&[
            0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C,
            0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6,
            0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC,
            0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
        ]);
        rom[0x0134..0x013C].copy_from_slice(b"CI TEST ");
        rom[0x0147] = 0x00;
        rom[0x0148] = 0x00;
        rom[0x0149] = 0x00;
        let mut checksum = 0u8;
        for byte in &rom[0x0134..=0x014C] {
            checksum = checksum.wrapping_sub(*byte).wrapping_sub(1);
        }
        rom[0x014D] = checksum;
        std::fs::write(&rom_path, rom).unwrap();

        let mut app = controller();
        app.import_content(&rom_path, 0x10000).unwrap();
        let mut registry = CoreRegistry::new();
        register_game_boy_color(&mut registry).unwrap();
        let probe = GameBoyColorCartridgeProbe;
        let probes: [&dyn ContentFormatProbe; 1] = [&probe];

        let id = app
            .launch_library_entry(1, ContentLimits::new(0x10000), &probes, &registry)
            .expect("a valid ROM-only test image should launch through the library");

        assert_eq!(app.active_session(), Some(id));
        assert_eq!(app.session_count(), 1);
        assert!(app.session_summary(id).unwrap().content_loaded);
        let library_entry = app.library().entries().next().unwrap();
        assert_eq!(
            library_entry.format_detection(),
            &ContentFormatDetection::Detected("game-boy-color".to_owned())
        );
        assert_eq!(library_entry.validation(), &ContentValidation::Accepted);

        app.command_session(id, SessionCommand::Pause).unwrap();
        assert_eq!(app.session_summary(id).unwrap().state, SessionState::Paused);
        app.command_session(id, SessionCommand::Resume).unwrap();
        assert_eq!(
            app.session_summary(id).unwrap().state,
            SessionState::Running
        );
        app.command_session(id, SessionCommand::Stop).unwrap();
        app.close_session(id).unwrap();
        assert_eq!(app.session_count(), 0);
        assert_eq!(app.active_session(), None);

        std::fs::remove_file(rom_path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn forgetting_a_missing_library_file_preserves_open_sessions() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "host-app-forget-controller-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let file = root.join("game.gb");
        std::fs::write(&file, b"test-content").unwrap();

        let mut app = controller();
        let canonical = app
            .import_content(&file, 1024)
            .unwrap()
            .path()
            .to_path_buf();
        let session = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(session).unwrap();

        std::fs::remove_file(&canonical).unwrap();
        let removed = app.forget_content(&canonical).unwrap();

        assert_eq!(removed.path(), canonical);
        assert!(app.library().is_empty());
        assert_eq!(app.session_count(), 1);
        assert_eq!(app.active_session(), Some(session));
        assert!(app.host().session(session).is_ok());

        app.close_all_sessions();
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn closing_sessions_preserves_or_repairs_active_selection() {
        let mut app = controller();
        let first = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        let second = app
            .host_mut()
            .create_session(Box::new(GameBoyColorCore::new()))
            .unwrap();
        app.select_session(second).unwrap();

        app.close_session(first).unwrap();
        assert_eq!(app.active_session(), Some(second));
        assert_eq!(app.session_count(), 1);

        app.close_session(second).unwrap();
        assert_eq!(app.active_session(), None);
        assert_eq!(app.session_count(), 0);
    }

    #[test]
    fn library_index_errors_are_explicit_and_do_not_create_sessions() {
        let mut app = controller();
        let error = app
            .launch_library_entry(1, ContentLimits::new(1024), &[], &CoreRegistry::new())
            .unwrap_err();

        assert!(matches!(error, ApplicationError::UnknownLibraryEntry(1)));
        assert!(app.host().is_empty());
        assert_eq!(app.active_session(), None);
    }
}
