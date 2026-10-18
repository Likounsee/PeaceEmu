#![forbid(unsafe_code)]
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
//! Experimental desktop frontend for the shared PeaceEmu application controller.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use audio_output::AudioOutput;
#[cfg(feature = "game-boy-color-module")]
use emulation_core::content::{ContentFormatProbe, ContentLimits};
use emulation_core::core_io::{FrameCadence, InputControl, InputEvent};
use game_view::{map_key_to_control, video_frame_to_image};
use host_app::controller::{ApplicationController, SessionCommand};
use preferences::{config_file_path, load_preferences, write_preferences_with_language};

mod appearance_page;
mod audio_output;
mod game_page;
mod game_saves;
mod game_view;
mod home_page;
mod language_pack;
mod library_page;
pub mod manifest_ed25519;
mod preferences;
mod save_states;
mod sessions_page;
pub mod worker_messages;
pub mod worker_process;

mod settings_page;
mod shell;
mod systems_page;
mod theme;
mod update;
use eframe::egui;
use emulation_core::host_runtime::{HostRuntime, HostRuntimeConfig, SessionId};
use emulation_core::runtime::SessionState;
use emulation_core::CoreRegistry;
#[cfg(feature = "game-boy-color-module")]
use game_boy::{register_game_boy_color, GameBoyColorCartridgeProbe};

const MAX_CONTENT_BYTES: usize = 32 * 1024 * 1024;

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Some(message) = parse_cli_arguments(&arguments)? {
        println!("{message}");
        return Ok(());
    }

    let app = DesktopApp::new()?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_fullscreen(true)
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PeaceEmu",
        options,
        Box::new(move |_creation_context| Box::new(app)),
    )?;
    Ok(())
}

fn parse_cli_arguments(arguments: &[String]) -> Result<Option<&'static str>, String> {
    if arguments.len() > 1 {
        return Err("expected at most one command-line argument; try --help".to_owned());
    }
    arguments
        .first()
        .map(|argument| cli_message(argument))
        .transpose()
}

fn cli_message(argument: &str) -> Result<&'static str, String> {
    match argument {
        "--version" | "-V" => Ok(concat!("PeaceEmu ", env!("CARGO_PKG_VERSION"))),
        "--help" | "-h" => Ok(concat!(
            "PeaceEmu ",
            env!("CARGO_PKG_VERSION"),
            "\n\nUsage: desktop-app [OPTIONS]\n\nOptions:\n  -h, --help     Print this help message\n  -V, --version  Print version information\n"
        )),
        other => Err(format!("unknown argument: {other}; try --help")),
    }
}

fn clear_tick_error_for_session(errors: &mut Vec<(SessionId, String)>, id: SessionId) {
    errors.retain(|(error_id, _)| *error_id != id);
}

#[cfg(test)]
fn message_indicates_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "import failed:",
        "launch failed:",
        "launch cancelled:",
        "save state failed:",
        "load state failed:",
        "game save failed:",
        "close cancelled;",
        "close all cancelled;",
        "could not ",
        "preferences unavailable:",
        "input error:",
    ]
    .iter()
    .any(|prefix| message.starts_with(prefix))
        || (message.starts_with("session ") && message.contains(" failed:"))
        || message.contains("in-game save could not be loaded")
        || message.contains("mais la sauvegarde en jeu n’a pas pu être chargée")
        || message.contains("sauvegarde en jeu impossible")
}

/// Polls a background task and drops its receiver on success or disconnection.
/// A disconnected worker must not leave the UI repainting forever as if it were still running.
fn poll_background_result<T>(receiver: &mut Option<Receiver<T>>) -> Result<Option<T>, ()> {
    match receiver.as_ref().map(Receiver::try_recv) {
        None | Some(Err(TryRecvError::Empty)) => Ok(None),
        Some(Ok(result)) => {
            *receiver = None;
            Ok(Some(result))
        }
        Some(Err(TryRecvError::Disconnected)) => {
            *receiver = None;
            Err(())
        }
    }
}

struct DesktopApp {
    controller: ApplicationController,
    registry: CoreRegistry,
    #[cfg(feature = "game-boy-color-module")]
    probe: GameBoyColorCartridgeProbe,
    content_path: String,
    library_filter: String,
    library_format: LibraryFormat,
    library_sort: LibrarySort,
    selected_content: Option<PathBuf>,
    message: String,
    dark_mode: bool,
    language: String,
    language_pack: language_pack::LanguagePack,
    accent_color: egui::Color32,
    config_path: Option<PathBuf>,
    active_page: DesktopPage,
    last_tick: Instant,
    last_game_save: Instant,
    frame_cadence: FrameCadence,
    frame_texture: Option<egui::TextureHandle>,
    video_size: Option<[u32; 2]>,
    media_status: String,
    audio_output: AudioOutput,
    last_tick_errors: Vec<(SessionId, String)>,
    session_content_paths: Vec<(SessionId, PathBuf)>,
    update_check_receiver: Option<Receiver<Result<update::UpdateManifest, String>>>,
    update_download_receiver: Option<Receiver<Result<PathBuf, String>>>,
    update_manifest: Option<update::UpdateManifest>,
    staged_update_path: Option<PathBuf>,
    update_status: String,
    fullscreen: bool,
    sidebar_open: bool,
    settings_section: SettingsSection,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibraryFormat {
    All,
    GameBoy,
    GameBoyColor,
}

impl LibraryFormat {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All formats",
            Self::GameBoy => "Game Boy (.gb)",
            Self::GameBoyColor => "Game Boy Color (.gbc)",
        }
    }

    fn matches(self, path: &Path) -> bool {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        match self {
            Self::All => true,
            Self::GameBoy => extension.eq_ignore_ascii_case("gb"),
            Self::GameBoyColor => extension.eq_ignore_ascii_case("gbc"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibrarySort {
    Name,
    Largest,
    Path,
}

impl LibrarySort {
    fn label(self) -> &'static str {
        match self {
            Self::Name => "Title (A–Z)",
            Self::Largest => "Largest first",
            Self::Path => "File path",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DesktopPage {
    Home,
    Library,
    Sessions,
    Game,
    Systems,
    Settings,
}

/// Sub-sections of the settings page, selected from the contextual sidebar.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    Appearance,
    Application,
}

impl DesktopPage {
    fn title(self, language_pack: &language_pack::LanguagePack) -> &str {
        match self {
            Self::Home => language_pack.text("ui.main.001"),
            Self::Library => language_pack.text("ui.main.002"),
            Self::Sessions => language_pack.text("ui.main.006"),
            Self::Game => language_pack.text("ui.main.003"),
            Self::Systems => language_pack.text("ui.main.004"),
            Self::Settings => language_pack.text("ui.main.005"),
        }
    }
}

impl DesktopApp {
    fn new() -> Result<Self, Box<dyn Error>> {
        let registry = CoreRegistry::new();
        #[cfg(feature = "game-boy-color-module")]
        let registry = {
            let mut registry = registry;
            register_game_boy_color(&mut registry)?;
            registry
        };
        let host = HostRuntime::new(HostRuntimeConfig::default())?;
        let mut controller = ApplicationController::new(host);
        let config_path = config_file_path();
        let mut dark_mode = true;
        let mut language = preferences::system_language();
        let mut accent_color = theme::DEFAULT_ACCENT;
        let mut message = String::from("Ready. Select a content path to import or launch.");
        if let Some(path) = config_path.as_deref() {
            match load_preferences(path) {
                Ok(Some(saved)) => {
                    dark_mode = saved.dark_mode;
                    language = saved.language;
                    accent_color = egui::Color32::from_rgb(
                        saved.accent_rgb[0],
                        saved.accent_rgb[1],
                        saved.accent_rgb[2],
                    );
                    let mut restored = 0usize;
                    let mut skipped = 0usize;
                    let mut skipped_details = Vec::new();
                    for content_path in saved.paths {
                        match controller.import_content(&content_path, MAX_CONTENT_BYTES as u64) {
                            Ok(_) => restored += 1,
                            Err(error) => {
                                skipped += 1;
                                if skipped_details.len() < 3 {
                                    skipped_details
                                        .push(format!("{}: {error}", content_path.display()));
                                }
                            }
                        }
                    }
                    message = if skipped == 0 {
                        format!("Restored preferences and {restored} library item(s).")
                    } else {
                        let details = if skipped_details.is_empty() {
                            String::new()
                        } else {
                            format!(" Details: {}", skipped_details.join("; "))
                        };
                        format!(
                            "Restored {restored} library item(s); skipped {skipped} unavailable or invalid path(s). Re-import those files if needed.{details}"
                        )
                    };
                }
                Ok(None) => {}
                Err(error) => {
                    message =
                        if sys_locale::get_locale().is_some_and(|locale| locale.starts_with("fr")) {
                            format!("Impossible de charger les préférences : {error}")
                        } else {
                            format!("Could not load preferences: {error}")
                        }
                }
            }
        } else {
            message = "Preferences unavailable: no user config directory was found.".to_owned();
        }
        let language_pack = language_pack::LanguagePack::load(&language);
        let mut app = Self {
            controller,
            registry,
            #[cfg(feature = "game-boy-color-module")]
            probe: GameBoyColorCartridgeProbe,
            content_path: String::new(),
            library_filter: String::new(),
            library_format: LibraryFormat::All,
            library_sort: LibrarySort::Name,
            selected_content: None,
            message,
            dark_mode,
            language,
            language_pack,
            accent_color,
            config_path,
            active_page: DesktopPage::Home,
            last_tick: Instant::now(),
            last_game_save: Instant::now(),
            frame_cadence: FrameCadence::new(60, 3)?,
            frame_texture: None,
            video_size: None,
            media_status: "No active session.".to_owned(),
            audio_output: AudioOutput::new(),
            last_tick_errors: Vec::new(),
            session_content_paths: Vec::new(),
            update_check_receiver: None,
            update_download_receiver: None,
            update_manifest: None,
            staged_update_path: None,
            update_status: "Not checked yet.".to_owned(),
            fullscreen: true,
            sidebar_open: true,
            settings_section: SettingsSection::Appearance,
        };
        // Check the official stable manifest without blocking the first window render.
        // The user can still trigger another check from Settings after this completes.
        app.start_update_check();
        Ok(app)
    }

    fn start_update_check(&mut self) {
        if self.update_check_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.update_check_receiver = Some(receiver);
        self.update_status = if self.language == "fr" {
            "Vérification de la version stable officielle en arrière-plan…".to_owned()
        } else {
            "Checking the official stable version in the background…".to_owned()
        };
        std::thread::spawn(move || {
            let _ = sender.send(update::fetch_manifest());
        });
    }

    fn navigate_to(&mut self, page: DesktopPage) {
        if self.active_page == DesktopPage::Game && page != DesktopPage::Game {
            if let Err(error) = self.release_game_inputs() {
                self.message = format!(
                    "{error}. Navigation was cancelled so the game can safely release its controls."
                );
                return;
            }
        }
        self.active_page = page;
    }

    fn choose_content_file(&mut self) {
        let selected = rfd::FileDialog::new()
            .add_filter("Game Boy ROMs", &["gb", "gbc"])
            .add_filter("Game Boy ROM", &["gb"])
            .add_filter("Game Boy Color ROM", &["gbc"])
            .add_filter("All files", &["*"])
            .pick_files();
        if let Some(paths) = selected {
            if paths.is_empty() {
                return;
            }
            let mut imported = 0usize;
            let mut rejected = 0usize;
            let mut first_imported_path = None;
            let mut failures = Vec::new();
            for path in paths {
                match self
                    .controller
                    .import_content(&path, MAX_CONTENT_BYTES as u64)
                {
                    Ok(entry) => {
                        imported += 1;
                        if first_imported_path.is_none() {
                            first_imported_path = Some(entry.path().to_path_buf());
                        }
                    }
                    Err(error) => {
                        rejected += 1;
                        if failures.len() < 3 {
                            failures.push(format!("{}: {error}", path.display()));
                        }
                    }
                }
            }
            if let Some(path) = first_imported_path {
                self.content_path = path.display().to_string();
                self.selected_content = Some(path);
            }
            let import_message = if self.language == "fr" {
                if rejected == 0 {
                    format!("{imported} ROM(s) importée(s) dans la bibliothèque.")
                } else {
                    let detail = if failures.is_empty() {
                        String::new()
                    } else {
                        format!(" Détails : {}", failures.join("; "))
                    };
                    format!("{imported} ROM(s) importée(s) ; {rejected} fichier(s) ignoré(s) ou refusé(s).{detail}")
                }
            } else if rejected == 0 {
                format!("Imported {imported} ROM(s) into your library.")
            } else {
                let detail = if failures.is_empty() {
                    String::new()
                } else {
                    format!(" Details: {}", failures.join("; "))
                };
                format!(
                    "Imported {imported} ROM(s); {rejected} file(s) skipped or rejected.{detail}"
                )
            };
            self.message = import_message.clone();
            if imported > 0 {
                self.navigate_to(DesktopPage::Library);
                if !self.save_preferences() {
                    let save_error = std::mem::take(&mut self.message);
                    self.message = format!(
                        "{import_message} Library changes are active for this session, but preferences were not saved. {save_error}"
                    );
                }
            }
        }
    }

    #[cfg(feature = "game-boy-color-module")]
    fn launch_content(&mut self) {
        let path = self.content_path.trim();
        if path.is_empty() {
            self.message = if self.language == "fr" {
                "Saisissez d’abord le chemin du contenu.".to_owned()
            } else {
                "Enter a content path first.".to_owned()
            };
            return;
        }

        // Release held controls in every session before creating/switching sessions.
        // Input may have been sent to a non-active session earlier; failing to release
        // that session before a launch could leave a control logically held.
        if let Err(error) = self.release_game_inputs() {
            self.message = if self.language == "fr" {
                format!("Lancement annulé : impossible de libérer les entrées : {error}")
            } else {
                format!("Launch cancelled: could not release held inputs: {error}")
            };
            return;
        }

        #[cfg(feature = "game-boy-color-module")]
        let probes: [&dyn ContentFormatProbe; 1] = [&self.probe];
        #[cfg(not(feature = "game-boy-color-module"))]
        let probes: [&dyn ContentFormatProbe; 0] = [];
        match self.controller.launch_content_path(
            Path::new(path),
            ContentLimits::new(MAX_CONTENT_BYTES),
            &probes,
            &self.registry,
        ) {
            Ok(id) => {
                self.audio_output.clear();
                let launched_path =
                    std::fs::canonicalize(Path::new(path)).unwrap_or_else(|_| PathBuf::from(path));
                self.session_content_paths
                    .retain(|(session_id, _)| *session_id != id);
                self.session_content_paths.push((id, launched_path));
                self.message = match self.load_game_data_for_session(id) {
                    Ok(true) => {
                        if self.language == "fr" {
                            format!("Session {} lancée ; sauvegarde en jeu restaurée.", id.get())
                        } else {
                            format!("Launched session {}; restored its in-game save.", id.get())
                        }
                    }
                    Ok(false) => {
                        if self.language == "fr" {
                            format!("Session {} lancée.", id.get())
                        } else {
                            format!("Launched session {}.", id.get())
                        }
                    }
                    Err(error) => {
                        if self.language == "fr" {
                            format!("Session {} lancée, mais la sauvegarde en jeu n’a pas pu être chargée : {error}", id.get())
                        } else {
                            format!("Launched session {}, but its in-game save could not be loaded: {error}", id.get())
                        }
                    }
                };
                self.frame_cadence.reset();
                self.frame_texture = None;
                self.video_size = None;
                self.last_tick = Instant::now();
                self.active_page = DesktopPage::Game;
            }
            Err(error) => {
                self.message = if self.language == "fr" {
                    format!("Échec du lancement : {error}")
                } else {
                    format!("Launch failed: {error}")
                }
            }
        }
    }

    #[cfg(not(feature = "game-boy-color-module"))]
    fn launch_content(&mut self) {
        self.message = if self.language == "fr" {
            "Le module Game Boy Color n’est pas inclus dans cette compilation. Recompilez avec --features game-boy-color-module pour l’activer.".to_owned()
        } else {
            "The Game Boy Color module is not included in this build. Rebuild with --features game-boy-color-module to enable it.".to_owned()
        };
    }

    fn save_preferences(&mut self) -> bool {
        let Some(path) = self.config_path.as_deref() else {
            self.message =
                "Could not save preferences: no writable user configuration directory is available."
                    .to_owned();
            return false;
        };
        let paths: Vec<PathBuf> = self
            .controller
            .library()
            .entries()
            .map(|entry| entry.path().to_path_buf())
            .collect();
        if let Err(error) = write_preferences_with_language(
            path,
            self.dark_mode,
            1.0,
            [
                self.accent_color.r(),
                self.accent_color.g(),
                self.accent_color.b(),
            ],
            &self.language,
            &paths,
        ) {
            self.message = if self.language == "fr" {
                format!("Impossible d’enregistrer les préférences : {error}")
            } else {
                format!("Could not save preferences: {error}")
            };
            return false;
        }
        true
    }

    fn game_save_path(&self, id: SessionId) -> Result<PathBuf, String> {
        let config_path = self.config_path.as_deref().ok_or_else(|| {
            "Game saves are unavailable because the configuration directory is missing.".to_owned()
        })?;
        let content_path = self
            .session_content_paths
            .iter()
            .find(|(session_id, _)| *session_id == id)
            .map(|(_, path)| path.as_path())
            .ok_or_else(|| {
                format!(
                    "Session {} has no known ROM path for its game-save slot.",
                    id.get()
                )
            })?;
        game_saves::save_path(config_path, content_path)
            .map_err(|error| format!("Could not resolve game-save path: {error}"))
    }

    fn save_game_data_for_session(&mut self, id: SessionId) -> Result<PathBuf, String> {
        let path = self.game_save_path(id)?;
        let (system_id, core_version, data) = {
            let session = self
                .controller
                .host_mut()
                .session_mut(id)
                .map_err(|error| error.to_string())?;
            let descriptor = session.descriptor();
            let data = session
                .save_game_data()
                .map_err(|error| error.to_string())?;
            (
                descriptor.system_id.as_str().to_owned(),
                descriptor.core_version.to_owned(),
                data,
            )
        };
        let envelope =
            emulation_core::save_state::SaveStateEnvelope::new(system_id, core_version, data)
                .map_err(|error| format!("Could not encode game save: {error}"))?;
        emulation_core::save_state_store::SaveStateStore::new(path.clone())
            .save(&envelope)
            .map_err(|error| format!("Could not write game save: {error}"))?;
        self.controller
            .host_mut()
            .session_mut(id)
            .map_err(|error| error.to_string())?
            .clear_persistent_save_dirty();
        Ok(path)
    }

    fn save_game_data_action(&mut self, id: SessionId) {
        match self.save_game_data_for_session(id) {
            Ok(_) => {
                self.message = if self.language == "fr" {
                    format!("Sauvegarde en jeu de la session {} enregistrée.", id.get())
                } else {
                    format!("Session {} in-game save written.", id.get())
                };
            }
            Err(error) => {
                self.message = if self.language == "fr" {
                    format!("Échec de la sauvegarde du jeu : {error}")
                } else {
                    format!("Game save failed: {error}")
                }
            }
        }
    }

    fn save_dirty_game_data_for_session(&mut self, id: SessionId) -> Result<(), String> {
        let dirty = self
            .controller
            .host()
            .session(id)
            .map_err(|error| error.to_string())?
            .persistent_save_dirty();
        if dirty {
            self.save_game_data_for_session(id)?;
        }
        Ok(())
    }

    fn flush_dirty_game_saves(&mut self) -> Vec<String> {
        let session_ids: Vec<_> = self
            .controller
            .session_summaries()
            .into_iter()
            .filter(|session| session.persistent_saves)
            .map(|session| session.id)
            .collect();
        let mut failures = Vec::new();
        for id in session_ids {
            if let Err(error) = self.save_dirty_game_data_for_session(id) {
                failures.push(format!("session {}: {error}", id.get()));
            }
        }
        failures
    }

    fn autosave_dirty_game_saves(&mut self) {
        if self.last_game_save.elapsed() < Duration::from_secs(2) {
            return;
        }
        let failures = self.flush_dirty_game_saves();
        self.last_game_save = Instant::now();
        if !failures.is_empty() {
            self.message = if self.language == "fr" {
                format!("Échec de la sauvegarde du jeu : {}", failures.join("; "))
            } else {
                format!("Game save failed: {}", failures.join("; "))
            };
        }
    }

    #[cfg(feature = "game-boy-color-module")]
    fn load_game_data_for_session(&mut self, id: SessionId) -> Result<bool, String> {
        let summary = self
            .controller
            .session_summary(id)
            .map_err(|error| error.to_string())?;
        if !summary.persistent_saves {
            return Ok(false);
        }
        let path = self.game_save_path(id)?;
        let envelope = match emulation_core::save_state_store::SaveStateStore::new(path).load() {
            Ok(envelope) => envelope,
            Err(emulation_core::save_state_store::SaveStateStoreError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(false);
            }
            Err(error) => return Err(format!("Could not read game save: {error}")),
        };
        let result = self
            .controller
            .host_mut()
            .session_mut(id)
            .map_err(|error| error.to_string())
            .and_then(|session| {
                let descriptor = session.descriptor();
                envelope
                    .validate_for(descriptor.system_id.as_str(), descriptor.core_version)
                    .map_err(|error| error.to_string())?;
                session
                    .load_game_data(envelope.payload())
                    .map_err(|error| error.to_string())
            });
        result.map(|()| true)
    }

    fn save_state_path(&self, id: SessionId) -> Result<PathBuf, String> {
        let config_path = self.config_path.as_deref().ok_or_else(|| {
            "Save states are unavailable because the configuration directory is missing.".to_owned()
        })?;
        let content_path = self
            .session_content_paths
            .iter()
            .find(|(session_id, _)| *session_id == id)
            .map(|(_, path)| path.as_path())
            .ok_or_else(|| {
                format!(
                    "Session {} has no known ROM path for its save slot.",
                    id.get()
                )
            })?;
        save_states::state_path(config_path, content_path)
            .map_err(|error| format!("Could not resolve save-state path: {error}"))
    }

    fn save_state_for_session(&mut self, id: SessionId) -> Result<PathBuf, String> {
        let path = self.save_state_path(id)?;
        let envelope = self
            .controller
            .host_mut()
            .session_mut(id)
            .map_err(|error| error.to_string())?
            .save_state_envelope()
            .map_err(|error| error.to_string())?;
        emulation_core::save_state_store::SaveStateStore::new(path.clone())
            .save(&envelope)
            .map_err(|error| format!("Could not write save state: {error}"))?;
        Ok(path)
    }

    fn save_state_action(&mut self, id: SessionId) {
        match self.save_state_for_session(id) {
            Ok(_) => {
                self.message = if self.language == "fr" {
                    format!("État de la session {} sauvegardé.", id.get())
                } else {
                    format!("Session {} state saved.", id.get())
                };
            }
            Err(error) => {
                self.message = if self.language == "fr" {
                    format!("Échec de la sauvegarde de l’état : {error}")
                } else {
                    format!("Save state failed: {error}")
                }
            }
        }
    }

    fn load_state_action(&mut self, id: SessionId) {
        let path = match self.save_state_path(id) {
            Ok(path) => path,
            Err(error) => {
                self.message = if self.language == "fr" {
                    format!("Échec du chargement de l’état : {error}")
                } else {
                    format!("Load state failed: {error}")
                };
                return;
            }
        };
        let envelope = match emulation_core::save_state_store::SaveStateStore::new(path).load() {
            Ok(envelope) => envelope,
            Err(emulation_core::save_state_store::SaveStateStoreError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                self.message = if self.language == "fr" {
                    "Aucune sauvegarde d’état n’existe pour cette ROM.".to_owned()
                } else {
                    "No save state exists for this ROM yet.".to_owned()
                };
                return;
            }
            Err(error) => {
                self.message = format!("Load state failed: {error}");
                return;
            }
        };
        let result = self
            .controller
            .host_mut()
            .session_mut(id)
            .map_err(|error| error.to_string())
            .and_then(|session| {
                session
                    .load_state_envelope(&envelope)
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => {
                if self.controller.active_session() == Some(id) {
                    self.audio_output.clear();
                    self.frame_cadence.reset();
                    self.frame_texture = None;
                    self.video_size = None;
                    self.last_tick = Instant::now();
                    self.media_status = if self.language == "fr" {
                        "État restauré ; attente de la prochaine trame.".to_owned()
                    } else {
                        "State restored; waiting for the next frame.".to_owned()
                    };
                }
                self.message = if self.language == "fr" {
                    format!("État de la session {} restauré.", id.get())
                } else {
                    format!("Session {} state restored.", id.get())
                };
            }
            Err(error) => {
                self.message = if self.language == "fr" {
                    format!("Échec du chargement de l’état : {error}")
                } else {
                    format!("Load state failed: {error}")
                }
            }
        }
    }

    fn session_action(&mut self, id: SessionId, action: SessionAction) {
        let is_active = self.controller.active_session() == Some(id);
        let release_error =
            if is_active && matches!(action, SessionAction::StopAndSave | SessionAction::Reset) {
                self.release_game_inputs().err()
            } else {
                None
            };

        // A reset requires released inputs. Stop-and-save remains a recovery action if
        // input release fails, but it must not stop if the snapshot itself cannot be saved.

        if matches!(action, SessionAction::Reset) {
            if let Some(error) = release_error.as_ref() {
                self.message = format!(
                    "Session {}: reset aborted because input release failed: {error}",
                    id.get()
                );
                return;
            }
        }

        if matches!(action, SessionAction::StopAndSave) {
            if let Err(error) = self.save_state_for_session(id) {
                self.message = format!(
                    "Session {}: stop aborted because saving the state failed: {error}",
                    id.get()
                );
                return;
            }
        }
        if matches!(action, SessionAction::StopAndSave)
            && self
                .controller
                .session_summary(id)
                .is_ok_and(|summary| summary.persistent_saves)
        {
            if let Err(error) = self.save_game_data_for_session(id) {
                self.message = format!(
                    "Session {}: stop aborted because saving in-game data failed: {error}",
                    id.get()
                );
                return;
            }
        }

        let command = match action {
            SessionAction::Pause => SessionCommand::Pause,
            SessionAction::Resume => SessionCommand::Resume,
            SessionAction::StopAndSave => SessionCommand::Stop,
            SessionAction::Reset => SessionCommand::Reset,
        };
        let result = self
            .controller
            .command_session(id, command)
            .map_err(|error| error.to_string());
        if result.is_ok() && is_active {
            match action {
                SessionAction::Pause => {
                    self.audio_output.pause();
                    self.frame_cadence.reset();
                }
                SessionAction::Resume => {
                    self.audio_output.resume();
                    self.last_tick = Instant::now();
                }
                SessionAction::StopAndSave => {
                    self.audio_output.clear();
                    self.frame_cadence.reset();
                    self.frame_texture = None;
                    self.video_size = None;
                    self.media_status = "Active session stopped after saving its state.".to_owned();
                }
                SessionAction::Reset => {
                    self.audio_output.clear();
                    self.frame_cadence.reset();
                    self.frame_texture = None;
                    self.video_size = None;
                    self.last_tick = Instant::now();
                    self.media_status = "Active session reset; waiting for a new frame.".to_owned();
                }
            }
        }
        if result.is_ok() && matches!(action, SessionAction::StopAndSave | SessionAction::Reset) {
            // A deliberate lifecycle transition supersedes an earlier execution/media tick error.
            clear_tick_error_for_session(&mut self.last_tick_errors, id);
        }
        self.message = match (result, release_error) {
            (Ok(()), Some(error)) => {
                if self.language == "fr" {
                    format!("Session {} : opération « {} » réussie, mais la libération des entrées a échoué : {error}", id.get(), action.label())
                } else {
                    format!(
                        "Session {}: {} succeeded, but input release failed: {error}",
                        id.get(),
                        action.label()
                    )
                }
            }
            (Ok(()), None) => {
                if self.language == "fr" {
                    format!(
                        "Session {} : opération « {} » réussie.",
                        id.get(),
                        action.label()
                    )
                } else {
                    format!("Session {}: {} succeeded.", id.get(), action.label())
                }
            }
            (Err(error), _) => {
                if self.language == "fr" {
                    format!(
                        "Session {} : échec de l’opération « {} » : {error}",
                        id.get(),
                        action.label()
                    )
                } else {
                    format!("Session {}: {} failed: {error}", id.get(), action.label())
                }
            }
        };
    }
}

#[derive(Clone, Copy)]
enum SessionAction {
    Pause,
    Resume,
    StopAndSave,
    Reset,
}

impl SessionAction {
    fn label(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::StopAndSave => "stop and save",
            Self::Reset => "reset",
        }
    }
}

impl eframe::App for DesktopApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Flush dirty battery-backed RAM before tearing down sessions.
        for error in self.flush_dirty_game_saves() {
            eprintln!("PeaceEmu: could not flush persistent game save on exit: {error}");
        }
        // Release held controls before tearing down sessions so a core never exits
        // with inputs logically stuck in the pressed state.
        let _ = self.release_game_inputs();
        self.controller.close_all_sessions();
    }

    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let update_result = match poll_background_result(&mut self.update_check_receiver) {
            Ok(result) => result,
            Err(()) => {
                self.update_status = if self.language == "fr" {
                    "Échec de la vérification : le processus en arrière-plan s’est interrompu."
                        .to_owned()
                } else {
                    "Update check failed: background worker disconnected before returning a result."
                        .to_owned()
                };
                self.update_manifest = None;
                None
            }
        };
        if let Some(result) = update_result {
            match result {
                Ok(manifest) => {
                    let current = option_env!("CARGO_PKG_VERSION").unwrap_or("0.0.0");
                    self.update_status = if update::is_newer_version(&manifest.version, current) {
                        if manifest.download_for_current_platform().is_some() {
                            if self.language == "fr" {
                                format!("La version {} est disponible au téléchargement et à la vérification; l’installation n’est pas automatique.", manifest.version)
                            } else {
                                format!("Version {} is available for download and integrity verification; installation is not automatic yet.", manifest.version)
                            }
                        } else {
                            if self.language == "fr" {
                                format!("La version {} est publiée, mais aucun téléchargement n’est disponible pour cette plateforme.", manifest.version)
                            } else {
                                format!("Version {} is published, but no download is available for this platform yet.", manifest.version)
                            }
                        }
                    } else {
                        if self.language == "fr" {
                            format!("Vous utilisez la dernière version stable ({}).", current)
                        } else {
                            format!("You are using the latest stable version ({}).", current)
                        }
                    };
                    self.update_manifest = Some(manifest);
                }
                Err(error) => {
                    self.update_status = if self.language == "fr" {
                        format!("Échec de la vérification des mises à jour : {error}")
                    } else {
                        format!("Update check failed: {error}")
                    };
                    self.update_manifest = None;
                }
            }
        }
        let download_result = match poll_background_result(&mut self.update_download_receiver) {
            Ok(result) => result,
            Err(()) => {
                self.update_status = if self.language == "fr" {
                    "Échec du téléchargement : le processus en arrière-plan s’est interrompu."
                        .to_owned()
                } else {
                    "Update download failed: background worker disconnected before returning a result.".to_owned()
                };
                self.staged_update_path = None;
                None
            }
        };
        if let Some(result) = download_result {
            match result {
                Ok(path) => {
                    self.update_status = if self.language == "fr" {
                        format!("Mise à jour téléchargée et SHA-256 vérifié : {}. L’installation n’est pas automatique.", path.display())
                    } else {
                        format!("Update downloaded and SHA-256 verified at {}. Installation is not automatic.", path.display())
                    };
                    self.staged_update_path = Some(path);
                }
                Err(error) => {
                    self.update_status = if self.language == "fr" {
                        format!("Échec du téléchargement de la mise à jour : {error}")
                    } else {
                        format!("Update download failed: {error}")
                    };
                    self.staged_update_path = None;
                }
            }
        }
        if self.update_check_receiver.is_some() || self.update_download_receiver.is_some() {
            context.request_repaint_after(Duration::from_millis(250));
        }
        self.advance_emulation(context);
        let previous_dark_mode = self.dark_mode;
        let previous_accent_color = self.accent_color;
        let previous_language = self.language.clone();
        let palette = theme::Palette::new(self.dark_mode, self.accent_color);
        palette.apply(context);

        // Keep the toggle synchronized with native window-manager changes and support F11.
        let native_fullscreen =
            context.input(|input| input.viewport().fullscreen.unwrap_or(self.fullscreen));
        if native_fullscreen != self.fullscreen {
            self.fullscreen = native_fullscreen;
        }
        let toggle_fullscreen = context.input(|input| {
            input.key_pressed(egui::Key::F11)
                || (input.modifiers.alt && input.key_pressed(egui::Key::Enter))
        });
        if toggle_fullscreen {
            self.fullscreen = !self.fullscreen;
            context.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }

        // Icon rail, contextual sidebar and status bar (see shell.rs).
        self.show_shell(context);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(palette.panel)
                    .inner_margin(egui::Margin {
                        left: 32.0,
                        right: 32.0,
                        top: 28.0,
                        bottom: 12.0,
                    }),
            )
            .show(context, |ui| {
                // Keep long pages usable on short displays and when the window is resized.
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.active_page {
                        DesktopPage::Home => self.show_home(ui),
                        DesktopPage::Library => self.show_library(ui),
                        DesktopPage::Sessions => self.show_sessions(ui),
                        DesktopPage::Game => self.show_game(ui),
                        DesktopPage::Systems => self.show_systems(ui),
                        DesktopPage::Settings => self.show_settings(ui, context),
                    });
            });

        if self.language != previous_language {
            self.language_pack = language_pack::LanguagePack::load(&self.language);
        }
        if self.dark_mode != previous_dark_mode
            || self.accent_color != previous_accent_color
            || self.language != previous_language
        {
            self.save_preferences();
        }
    }
}

impl DesktopApp {
    fn advance_emulation(&mut self, context: &egui::Context) {
        // Flush a previous tick's dirty battery RAM even if the next tick errors or
        // the active session has since been deselected.
        self.autosave_dirty_game_saves();
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;

        if self.active_page == DesktopPage::Game {
            let events = context.input(|input| input.events.clone());
            for event in events {
                match event {
                    egui::Event::Key {
                        key,
                        pressed,
                        repeat,
                        ..
                    } => {
                        if repeat && pressed {
                            continue;
                        }
                        if let Some(control) = map_key_to_control(key) {
                            self.send_game_input(control, pressed);
                        }
                    }
                    egui::Event::WindowFocused(false) => {
                        if let Err(error) = self.release_game_inputs() {
                            self.message = error;
                        }
                    }
                    _ => {}
                }
            }
        }

        let Some(id) = self.controller.active_session() else {
            self.frame_cadence.reset();
            self.frame_texture = None;
            self.video_size = None;
            self.media_status =
                "No active session. Launch a ROM to receive core-provided frames.".to_owned();
            return;
        };

        let update = match self
            .controller
            .advance_session(id, elapsed, &mut self.frame_cadence)
        {
            Ok(update) => update,
            Err(error) => {
                let release_status = if self.controller.active_session() == Some(id) {
                    self.controller
                        .release_inputs(id)
                        .err()
                        .map(|release| format!(" Input release also failed: {release}"))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                self.media_status =
                    format!("Could not advance selected session: {error}.{release_status}");
                self.last_tick_errors
                    .retain(|(error_id, _)| *error_id != id);
                self.last_tick_errors.push((id, self.media_status.clone()));
                self.frame_texture = None;
                self.video_size = None;
                return;
            }
        };
        let state = update.state;
        let video_available = update.video_available;
        let audio_available = update.audio_available;
        let mut error = update.error;
        let newest_frame = update.newest_frame;
        if state == SessionState::Running {
            for buffer in &update.audio_buffers {
                self.audio_output.submit(buffer);
            }
        }

        if let Some(frame) = newest_frame {
            match video_frame_to_image(&frame) {
                Ok(image) => {
                    let [width, height] = image.size;
                    self.video_size = Some([width as u32, height as u32]);
                    if let Some(texture) = self.frame_texture.as_mut() {
                        texture.set(image, egui::TextureOptions::NEAREST);
                    } else {
                        self.frame_texture = Some(context.load_texture(
                            "desktop-game-frame",
                            image,
                            egui::TextureOptions::NEAREST,
                        ));
                    }
                }
                Err(frame_error) => error = Some(frame_error),
            }
        }

        if let Some(error_message) = error.as_ref() {
            let mut diagnostic = error_message.clone();
            if self.controller.active_session() == Some(id) {
                if let Err(release_error) = self.controller.release_inputs(id) {
                    diagnostic.push_str(&format!(" Input release also failed: {release_error}"));
                }
            }
            self.last_tick_errors
                .retain(|(error_id, _)| *error_id != id);
            self.last_tick_errors.push((id, diagnostic));
            self.frame_texture = None;
            self.video_size = None;
        } else {
            self.last_tick_errors
                .retain(|(error_id, _)| *error_id != id);
        }

        let pipeline_status = if let Some(error) = error {
            error
        } else if state == SessionState::Paused {
            "Paused — video output is frozen until you resume.".to_owned()
        } else if state != SessionState::Running {
            format!("Session state: {state:?}.")
        } else if !video_available {
            "This core does not currently advertise video output; the display area is a placeholder.".to_owned()
        } else if audio_available {
            "Video pipeline connected; core audio buffers are forwarded to the host output."
                .to_owned()
        } else {
            "Video pipeline connected. This core does not currently advertise audio output."
                .to_owned()
        };
        self.media_status = if audio_available {
            format!(
                "{pipeline_status} Audio device: {}.",
                self.audio_output.status()
            )
        } else {
            pipeline_status
        };
        if state == SessionState::Running {
            context.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn send_game_input(&mut self, control: InputControl, pressed: bool) {
        let Some(id) = self.controller.active_session() else {
            return;
        };
        let result = self
            .controller
            .send_input(id, InputEvent { control, pressed })
            .map_err(|error| error.to_string());
        if let Err(error) = result {
            if !error.contains("unsupported") && !error.contains("input") {
                self.message = if self.language == "fr" {
                    format!("Erreur de saisie : {error}")
                } else {
                    format!("Input error: {error}")
                };
            }
        }
    }

    fn release_game_inputs(&mut self) -> Result<(), String> {
        // Multiple sessions may have received input before another session was
        // selected. Release every session, not only the currently selected one,
        // before closing the runtime or switching away from emulation.
        let session_ids: Vec<_> = self
            .controller
            .session_summaries()
            .into_iter()
            .map(|session| session.id)
            .collect();
        let mut failures = Vec::new();
        for id in session_ids {
            if let Err(error) = self.controller.release_inputs(id) {
                failures.push(format!("session {}: {error}", id.get()));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Could not release held inputs for {}",
                failures.join("; ")
            ))
        }
    }
}

fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    if bytes >= MIB {
        format!("{:.2} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod preference_tests {
    use super::*;
    use crate::preferences::{
        escape_config_value, load_preferences, unescape_config_value, write_preferences,
    };
    use std::fs;

    #[test]
    fn launch_cancellation_is_classified_as_an_error() {
        assert!(message_indicates_error(
            "Launch cancelled: could not release inputs from session 3"
        ));
        assert!(!message_indicates_error("Launched session 3."));
    }

    #[test]
    fn command_line_help_and_version_are_available_without_starting_the_gui() {
        assert_eq!(
            cli_message("--version").unwrap(),
            concat!("PeaceEmu ", env!("CARGO_PKG_VERSION"))
        );
        assert!(cli_message("--help").unwrap().contains("--version"));
        assert!(cli_message("-h").unwrap().contains("Usage: desktop-app"));
        assert_eq!(
            cli_message("--unknown").unwrap_err(),
            "unknown argument: --unknown; try --help"
        );
    }

    #[test]
    fn cli_argument_parser_rejects_multiple_arguments_before_gui_startup() {
        let arguments = vec!["--help".to_owned(), "--version".to_owned()];
        assert_eq!(
            parse_cli_arguments(&arguments).unwrap_err(),
            "expected at most one command-line argument; try --help"
        );
    }

    #[test]
    fn cli_argument_parser_leaves_gui_startup_to_the_caller_without_options() {
        assert_eq!(parse_cli_arguments(&[]).unwrap(), None);
        assert!(parse_cli_arguments(&["--version".to_owned()])
            .unwrap()
            .unwrap()
            .starts_with("PeaceEmu "));
    }

    #[test]
    fn user_status_classifies_each_error_family_without_false_positives() {
        for message in [
            "Import failed: file is unreadable",
            "Launch failed: format was rejected",
            "Launch cancelled: inputs could not be released",
            "Could not load preferences: invalid data",
            "Preferences unavailable: no config directory",
            "Input error: device disconnected",
            "Game save failed: invalid RAM payload",
            "Launched session 7, but its in-game save could not be loaded: checksum mismatch",
            "Fermeture annulée ; sauvegarde en jeu impossible : disk full",
            "Close all cancelled; game saves could not be flushed: disk full",
            "Session 7 failed: core stopped unexpectedly",
        ] {
            assert!(
                message_indicates_error(message),
                "not classified as error: {message}"
            );
        }
        for message in [
            "Ready. Select a content path to import or launch.",
            "Restored preferences and 2 library item(s).",
            "Launched session 7.",
            "No active session.",
        ] {
            assert!(
                !message_indicates_error(message),
                "false error classification: {message}"
            );
        }
    }

    #[test]
    fn file_size_formatting_changes_units_at_exact_boundaries() {
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1024 * 1024 - 1), "1024.0 KiB");
        assert_eq!(format_size(1024 * 1024), "1.00 MiB");
    }

    #[test]
    fn background_task_receiver_is_cleared_after_worker_disconnects() {
        let (sender, receiver) = mpsc::channel::<u8>();
        drop(sender);
        let mut receiver = Some(receiver);

        assert!(poll_background_result(&mut receiver).is_err());
        assert!(receiver.is_none());
    }

    #[test]
    fn background_task_receiver_returns_results_and_waits_while_empty() {
        let (sender, receiver) = mpsc::channel::<u8>();
        let mut receiver = Some(receiver);

        assert_eq!(poll_background_result(&mut receiver), Ok(None));
        sender.send(42).unwrap();
        assert_eq!(poll_background_result(&mut receiver), Ok(Some(42)));
        assert!(receiver.is_none());
    }

    #[cfg(feature = "game-boy-color-module")]
    #[test]
    fn clearing_one_session_tick_error_preserves_other_sessions() {
        let mut controller =
            ApplicationController::new(HostRuntime::new(HostRuntimeConfig::default()).unwrap());
        let first = controller
            .host_mut()
            .create_session(Box::new(game_boy::GameBoyColorCore::new()))
            .unwrap();
        let second = controller
            .host_mut()
            .create_session(Box::new(game_boy::GameBoyColorCore::new()))
            .unwrap();
        let mut errors = vec![
            (first, "first session failed".to_owned()),
            (second, "second session failed".to_owned()),
        ];

        clear_tick_error_for_session(&mut errors, first);

        assert_eq!(errors, vec![(second, "second session failed".to_owned())]);
    }

    #[test]
    fn library_format_filter_is_case_insensitive() {
        assert!(LibraryFormat::GameBoy.matches(Path::new("Pokemon.GB")));
        assert!(LibraryFormat::GameBoyColor.matches(Path::new("Pokemon.GbC")));
        assert!(!LibraryFormat::GameBoy.matches(Path::new("Pokemon.gbc")));
        assert!(LibraryFormat::All.matches(Path::new("archive.zip")));
    }

    #[test]
    fn displayed_file_sizes_use_readable_units() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(2 * 1024 * 1024), "2.00 MiB");
    }

    #[test]
    fn config_path_escaping_round_trips_special_characters() {
        let value = "C:\\Games\\my\nrom\r.gb";
        assert_eq!(
            unescape_config_value(&escape_config_value(value)),
            Some(value.to_owned())
        );
    }

    #[test]
    fn malformed_escape_sequences_are_rejected() {
        assert_eq!(unescape_config_value("bad\\x"), None);
        assert_eq!(unescape_config_value("trailing\\"), None);
    }

    #[test]
    fn preference_scale_is_clamped_and_invalid_values_are_ignored() {
        let root = std::env::temp_dir().join(format!("desktop-prefs-{}", std::process::id()));
        let file = root.join("desktop.conf");
        write_preferences(
            &file,
            false,
            2.0,
            [75, 115, 210],
            &[PathBuf::from("sample.gb")],
        )
        .unwrap();
        let loaded = load_preferences(&file).unwrap().unwrap();
        assert!(!loaded.dark_mode);
        assert_eq!(loaded.ui_scale, 1.4);
        assert_eq!(loaded.paths, vec![PathBuf::from("sample.gb")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replacing_preferences_keeps_only_the_new_complete_config() {
        let root =
            std::env::temp_dir().join(format!("desktop-prefs-replace-{}", std::process::id()));
        let file = root.join("desktop.conf");
        write_preferences(&file, true, 1.0, [75, 115, 210], &[PathBuf::from("old.gb")]).unwrap();
        write_preferences(
            &file,
            false,
            1.2,
            [75, 115, 210],
            &[PathBuf::from("new.gb")],
        )
        .unwrap();

        let loaded = load_preferences(&file).unwrap().unwrap();
        assert!(!loaded.dark_mode);
        assert_eq!(loaded.ui_scale, 1.2);
        assert_eq!(loaded.paths, vec![PathBuf::from("new.gb")]);
        assert!(!file.with_extension("conf.tmp").exists());
        assert!(!file.with_extension("conf.bak").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_scale_lines_do_not_override_a_valid_scale() {
        let root = std::env::temp_dir().join(format!(
            "desktop-prefs-invalid-scale-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("desktop.conf");
        fs::write(&file, "version=1\ndark=false\nscale=NaN\nscale=oops\n").unwrap();

        let loaded = load_preferences(&file).unwrap().unwrap();
        assert!(!loaded.dark_mode);
        assert_eq!(loaded.ui_scale, 1.0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsupported_preferences_version_is_rejected() {
        let root =
            std::env::temp_dir().join(format!("desktop-prefs-version-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("desktop.conf");
        fs::write(&file, "version=99\ndark=false\n").unwrap();

        let error = match load_preferences(&file) {
            Err(error) => error,
            Ok(_) => panic!("unsupported preferences version should be rejected"),
        };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        fs::remove_dir_all(root).unwrap();
    }
}
