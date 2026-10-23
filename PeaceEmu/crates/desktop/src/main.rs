#![forbid(unsafe_code)]
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
//! Experimental desktop frontend for the shared PeaceEmu application controller.

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use audio_output::AudioOutput;
use emulation_core::content::{ContentFormatProbe, ContentLimits};
use emulation_core::core_io::{FrameCadence, InputControl, InputEvent};
use game_view::{map_key_to_control, video_frame_to_image};
use host_app::controller::{ApplicationController, SessionCommand};
use preferences::{config_file_path, load_preferences, write_preferences_with_language};

mod appearance_page;
mod audio_output;
mod game_covers;
mod game_names;
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
mod nes_adapter;
use nes_adapter::{NesCartridgeProbe, NesCoreAdapter};

const MAX_CONTENT_BYTES: usize = 32 * 1024 * 1024;

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Some(message) = parse_cli_arguments(&arguments)? {
        println!("{message}");
        return Ok(());
    }

    let app = DesktopApp::new()?;
    let options = eframe::NativeOptions {
        // An explicit icon replaces egui's default white "e" window/taskbar icon.
        // Keep the native window's identity aligned with the installed PeaceEmu app.
        viewport: egui::ViewportBuilder::default()
            .with_fullscreen(true)
            .with_icon(std::sync::Arc::new(application_icon()?)),
        ..Default::default()
    };
    eframe::run_native(
        "PeaceEmu",
        options,
        Box::new(move |_creation_context| Box::new(app)),
    )?;
    Ok(())
}

/// Loads the bundled PeaceEmu bird mark for the native window and taskbar icon.
fn application_icon() -> Result<egui::IconData, image::ImageError> {
    // Keep the window icon small enough for X11 window-property limits while
    // retaining sufficient resolution for Windows taskbar and Task Manager.
    let rgba = image::load_from_memory(include_bytes!("../assets/branding/bird.png"))?
        .resize(256, 256, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
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
            "\n\nUsage: PeaceEmu [OPTIONS]\n\nOptions:\n  -h, --help     Print this help message\n  -V, --version  Print version information\n"
        )),
        other => Err(format!("unknown argument: {other}; try --help")),
    }
}

fn clear_tick_error_for_session<Id: Copy + PartialEq>(errors: &mut Vec<(Id, String)>, id: Id) {
    errors.retain(|(error_id, _)| *error_id != id);
}

fn preferences_unavailable_message(language: &str) -> &'static str {
    if language == "fr" {
        "Impossible d’enregistrer les préférences : aucun dossier de configuration utilisateur accessible en écriture."
    } else {
        "Could not save preferences: no writable user configuration directory is available."
    }
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
    probe: NesCartridgeProbe,
    game_viewport_open: bool,
    game_viewport_closed: Arc<AtomicBool>,
    game_viewport_inputs: Arc<Mutex<Vec<InputEvent>>>,
    content_path: String,
    library_filter: String,
    library_format: LibraryFormat,
    selected_content: Option<PathBuf>,
    message: String,
    import_warning: Option<String>,
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
    update_prompt_open: bool,
    update_prompt_dismissed: bool,
    install_after_update_download: bool,
    fullscreen: bool,
    sidebar_open: bool,
    settings_section: SettingsSection,
    cover_paths: HashMap<PathBuf, PathBuf>,
    cover_textures: HashMap<PathBuf, egui::TextureHandle>,
    game_names: HashMap<PathBuf, String>,
    display_name_edit: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibraryFormat {
    All,
    GameBoy,
}

impl LibraryFormat {
    fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::All, true) => "Tous les jeux",
            (Self::All, false) => "All games",
            (Self::GameBoy, _) => "Game Boy (GB / GBC / GBA)",
        }
    }

    fn matches(self, path: &Path) -> bool {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        match self {
            Self::All => true,
            Self::GameBoy => ["gb", "gbc", "gba"]
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported)),
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
        let mut registry = CoreRegistry::new();
        registry.register(emulation_core::SystemId::new("nes")?, || {
            Box::new(NesCoreAdapter::new())
        })?;
        let host = HostRuntime::new(HostRuntimeConfig::default())?;
        let mut controller = ApplicationController::new(host);
        let config_path = config_file_path();
        let mut dark_mode = true;
        let mut language = preferences::system_language();
        let mut accent_color = theme::DEFAULT_ACCENT;
        let mut message = String::from("Ready. Select a content path to import or launch.");
        let mut selected_content = None;
        if let Some(path) = config_path.as_deref() {
            match load_preferences(path) {
                Ok(Some(saved)) => {
                    dark_mode = saved.dark_mode;
                    // Keep reading the legacy persisted scale for compatibility; no scale control is exposed.
                    let _legacy_ui_scale = saved.ui_scale;
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
                            Ok(entry) => {
                                // Restore a usable library selection from the first path that
                                // successfully imports. Do not select stale paths that failed.
                                if selected_content.is_none() {
                                    selected_content = Some(entry.path().to_path_buf());
                                }
                                restored += 1;
                            }
                            Err(error) => {
                                skipped += 1;
                                if skipped_details.len() < 3 {
                                    skipped_details
                                        .push(format!("{}: {error}", content_path.display()));
                                }
                            }
                        }
                    }
                    message = if language == "fr" {
                        if skipped == 0 {
                            format!("Préférences restaurées et {restored} élément(s) de bibliothèque chargé(s).")
                        } else {
                            let details = if skipped_details.is_empty() {
                                String::new()
                            } else {
                                format!(" Détails : {}", skipped_details.join("; "))
                            };
                            format!(
                                "{restored} élément(s) de bibliothèque chargé(s) ; {skipped} chemin(s) indisponible(s) ou invalide(s) ignoré(s). Réimportez les fichiers concernés si nécessaire.{details}"
                            )
                        }
                    } else if skipped == 0 {
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
            message = if language == "fr" {
                "Préférences indisponibles : aucun dossier de configuration utilisateur n’a été trouvé.".to_owned()
            } else {
                "Preferences unavailable: no user config directory was found.".to_owned()
            };
        }
        let cover_paths = config_path
            .as_deref()
            .map(game_covers::load)
            .unwrap_or_default();
        let game_names = config_path
            .as_deref()
            .map(game_names::load)
            .unwrap_or_default();
        let language_pack = language_pack::LanguagePack::load(&language);
        let mut app = Self {
            controller,
            registry,
            probe: NesCartridgeProbe,
            game_viewport_open: false,
            game_viewport_closed: Arc::new(AtomicBool::new(false)),
            game_viewport_inputs: Arc::new(Mutex::new(Vec::new())),
            content_path: selected_content
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            library_filter: String::new(),
            library_format: LibraryFormat::All,
            selected_content,
            message,
            import_warning: None,
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
            update_prompt_open: false,
            update_prompt_dismissed: false,
            install_after_update_download: false,
            fullscreen: true,
            sidebar_open: true,
            settings_section: SettingsSection::Appearance,
            cover_paths,
            cover_textures: HashMap::new(),
            game_names,
            display_name_edit: String::new(),
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

    /// Starts one background download. The updater verifies size and SHA-256 before staging.
    fn start_update_download(&mut self, download: update::UpdateDownload, version: String) {
        if self.update_download_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.update_download_receiver = Some(receiver);
        self.update_status = if self.language == "fr" {
            format!("Téléchargement de la version {version} dans la zone temporaire…")
        } else {
            format!("Downloading version {version} to a temporary staging area…")
        };
        std::thread::spawn(move || {
            let result = update::download_and_verify(&download, &version);
            let _ = sender.send(result);
        });
    }

    /// Small, non-blocking startup notification, shown once per launch on Windows.
    fn show_update_prompt(&mut self, context: &egui::Context) {
        if !cfg!(target_os = "windows") || !self.update_prompt_open {
            return;
        }
        let Some(version) = self
            .update_manifest
            .as_ref()
            .map(|manifest| manifest.version.clone())
        else {
            self.update_prompt_open = false;
            return;
        };
        let french = self.language == "fr";
        let downloading = self.update_download_receiver.is_some();
        let status = self.update_status.clone();
        let show_status = downloading
            || status.contains("Échec")
            || status.contains("failed")
            || status.contains("Impossible")
            || status.contains("Could not");
        let mut install_clicked = false;
        let mut later_clicked = false;
        let _ = egui::Window::new(if french {
            "Mise à jour disponible"
        } else {
            "Update available"
        })
        .id(egui::Id::new("peaceemu_update_available"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(context, |ui| {
            ui.set_min_width(340.0);
            ui.label(if french {
                format!("PeaceEmu {version} est disponible.")
            } else {
                format!("PeaceEmu {version} is available.")
            });
            ui.add_space(4.0);
            ui.label(if french {
                "Téléchargez et vérifiez la mise à jour avant son installation."
            } else {
                "The update will be downloaded and verified before installation."
            });
            if show_status {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(&status).size(12.0));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let label = if downloading {
                    if french {
                        "Téléchargement…"
                    } else {
                        "Downloading…"
                    }
                } else if french {
                    "Installer maintenant"
                } else {
                    "Install now"
                };
                if ui
                    .add_enabled(!downloading, egui::Button::new(label))
                    .clicked()
                {
                    install_clicked = true;
                }
                if ui
                    .button(if french { "Plus tard" } else { "Later" })
                    .clicked()
                {
                    later_clicked = true;
                }
            });
        });
        if later_clicked {
            self.install_after_update_download = false;
            self.update_prompt_open = false;
            self.update_prompt_dismissed = true;
            return;
        }
        if install_clicked {
            if self.staged_update_path.is_some() {
                if self.start_update_install(context) {
                    self.update_prompt_open = false;
                    self.update_prompt_dismissed = true;
                }
            } else if let Some(download) = self
                .update_manifest
                .as_ref()
                .and_then(|manifest| manifest.download_for_current_platform())
                .cloned()
            {
                self.install_after_update_download = true;
                self.start_update_download(download, version);
                context.request_repaint_after(Duration::from_millis(250));
            }
        }
    }

    fn start_update_install(&mut self, _context: &egui::Context) -> bool {
        let Some(path) = self.staged_update_path.clone() else {
            self.update_status = if self.language == "fr" {
                "Téléchargez et vérifiez la mise à jour avant de l’installer.".to_owned()
            } else {
                "Download and verify the update before installing it.".to_owned()
            };
            return false;
        };
        let download = self
            .update_manifest
            .as_ref()
            .and_then(|manifest| manifest.download_for_current_platform())
            .map(|download| (download.size_bytes, download.sha256.clone()));
        let Some((size, sha256)) = download else {
            self.update_status = if self.language == "fr" {
                "Impossible de retrouver les informations officielles pour vérifier ce fichier."
                    .to_owned()
            } else {
                "Could not find official metadata to verify this installer.".to_owned()
            };
            return false;
        };
        #[cfg(target_os = "windows")]
        {
            match update::launch_verified_windows_installer(&path, size, &sha256) {
                Ok(()) => {
                    self.update_status = if self.language == "fr" {
                        "Fermeture de PeaceEmu puis lancement de l’installation…".to_owned()
                    } else {
                        "Closing PeaceEmu and starting the installer…".to_owned()
                    };
                    _context.send_viewport_cmd(egui::ViewportCommand::Close);
                    true
                }
                Err(error) => {
                    self.update_status = if self.language == "fr" {
                        format!("Impossible de lancer l’installation : {error}")
                    } else {
                        format!("Could not start the installer: {error}")
                    };
                    false
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (path, size, sha256);
            self.update_status = if self.language == "fr" {
                "L’installation automatique est actuellement disponible uniquement sous Windows."
                    .to_owned()
            } else {
                "Automatic installation is currently available on Windows only.".to_owned()
            };
            false
        }
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
            .add_filter("Game Boy ROMs", &["gb", "gbc", "gba"])
            .add_filter("Game Boy ROM", &["gb"])
            .add_filter("Game Boy Color ROM", &["gbc"])
            .add_filter("Game Boy Advance ROM", &["gba"])
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
                let result = validate_import_file(&path).and_then(|()| {
                    self.controller
                        .import_content(&path, MAX_CONTENT_BYTES as u64)
                        .map(|entry| entry.path().to_path_buf())
                        .map_err(|error| error.to_string())
                });
                match result {
                    Ok(imported_path) => {
                        imported += 1;
                        if first_imported_path.is_none() {
                            first_imported_path = Some(imported_path);
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
            let import_message =
                format_import_summary(&self.language, imported, rejected, &failures);
            self.message = import_message.clone();
            self.import_warning = (rejected > 0).then(|| import_message.clone());
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

    fn launch_content(&mut self) {
        let path = self.content_path.trim();
        if path.is_empty() {
            self.message = if self.language == "fr" {
                "Sélectionnez d’abord un jeu dans la bibliothèque.".to_owned()
            } else {
                "Select a game from the library first.".to_owned()
            };
            return;
        }

        if let Err(error) = self.release_game_inputs() {
            self.message = if self.language == "fr" {
                format!("Lancement annulé : impossible de libérer les entrées : {error}")
            } else {
                format!("Launch cancelled: could not release held inputs: {error}")
            };
            return;
        }

        let probes: [&dyn ContentFormatProbe; 1] = [&self.probe];
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
                            format!("Jeu lancé dans la session {}.", id.get())
                        } else {
                            format!("Game launched in session {}.", id.get())
                        }
                    }
                    Err(error) => {
                        if self.language == "fr" {
                            format!("Session {} lancée, mais la sauvegarde en jeu n’a pas pu être chargée : {error}", id.get())
                        } else {
                            format!("Session {} launched, but its in-game save could not be loaded: {error}", id.get())
                        }
                    }
                };
                self.frame_cadence.reset();
                self.frame_texture = None;
                self.video_size = None;
                self.last_tick = Instant::now();
                self.active_page = DesktopPage::Game;
                self.game_viewport_open = true;
                self.game_viewport_closed.store(false, Ordering::Release);
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

    fn save_preferences(&mut self) -> bool {
        let Some(path) = self.config_path.as_deref() else {
            self.message = preferences_unavailable_message(&self.language).to_owned();
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
        let envelope = self
            .controller
            .save_game_data_envelope(id)
            .map_err(|error| format!("Could not encode game save: {error}"))?;
        emulation_core::save_state_store::SaveStateStore::new(path.clone())
            .save(&envelope)
            .map_err(|error| format!("Could not write game save: {error}"))?;
        self.controller.clear_persistent_save_dirty(id)?;
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
        let dirty = self.controller.persistent_save_dirty(id)?;
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
        self.controller
            .load_game_data_envelope(id, &envelope)
            .map(|()| true)
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
        let envelope = self.controller.save_state_envelope(id)?;
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
                self.message = if self.language == "fr" {
                    format!("Échec du chargement de l’état : {error}")
                } else {
                    format!("Load state failed: {error}")
                };
                return;
            }
        };
        let result = self.controller.load_state_envelope(id, &envelope);
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
                self.message = if self.language == "fr" {
                    format!(
                        "Session {} : réinitialisation annulée car la libération des entrées a échoué : {error}",
                        id.get()
                    )
                } else {
                    format!(
                        "Session {}: reset aborted because input release failed: {error}",
                        id.get()
                    )
                };
                return;
            }
        }

        if matches!(action, SessionAction::StopAndSave) {
            if let Err(error) = self.save_state_for_session(id) {
                self.message = if self.language == "fr" {
                    format!(
                        "Session {} : arrêt annulé car la sauvegarde de l’état a échoué : {error}",
                        id.get()
                    )
                } else {
                    format!(
                        "Session {}: stop aborted because saving the state failed: {error}",
                        id.get()
                    )
                };
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
                self.message = if self.language == "fr" {
                    format!(
                        "Session {} : arrêt annulé car la sauvegarde en jeu a échoué : {error}",
                        id.get()
                    )
                } else {
                    format!(
                        "Session {}: stop aborted because saving in-game data failed: {error}",
                        id.get()
                    )
                };
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
                    self.media_status = if self.language == "fr" {
                        "Session active arrêtée après la sauvegarde de son état.".to_owned()
                    } else {
                        "Active session stopped after saving its state.".to_owned()
                    };
                }
                SessionAction::Reset => {
                    self.audio_output.clear();
                    self.frame_cadence.reset();
                    self.frame_texture = None;
                    self.video_size = None;
                    self.last_tick = Instant::now();
                    self.media_status = if self.language == "fr" {
                        "Session active réinitialisée ; en attente d’une nouvelle image.".to_owned()
                    } else {
                        "Active session reset; waiting for a new frame.".to_owned()
                    };
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
        if let Err(error) = self.release_game_inputs() {
            eprintln!("PeaceEmu: could not release held inputs on exit: {error}");
        }
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
                    let newer_version = update::is_newer_version(&manifest.version, current);
                    let has_platform_download = manifest.download_for_current_platform().is_some();
                    if cfg!(target_os = "windows")
                        && newer_version
                        && has_platform_download
                        && !self.update_prompt_dismissed
                    {
                        self.update_prompt_open = true;
                    }
                    self.update_status = if newer_version {
                        if manifest.download_for_current_platform().is_some() {
                            if self.language == "fr" {
                                format!("La version {} est disponible. Téléchargez-la puis cliquez sur Installer la mise à jour.", manifest.version)
                            } else {
                                format!("Version {} is available. Download it, then click Install update.", manifest.version)
                            }
                        } else {
                            if self.language == "fr" {
                                format!("La version {} est publiée, mais aucun téléchargement n’est disponible pour cette plateforme.", manifest.version)
                            } else {
                                format!("Version {} is published, but no download is available for this platform yet.", manifest.version)
                            }
                        }
                    } else if update::is_newer_version(current, &manifest.version) {
                        if self.language == "fr" {
                            format!(
                                "La version installée ({current}) est plus récente que la version stable annoncée ({}). Vérifiez le manifeste de mise à jour.",
                                manifest.version
                            )
                        } else {
                            format!(
                                "Installed version ({current}) is newer than the published stable version ({}). Check the update manifest.",
                                manifest.version
                            )
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
                self.install_after_update_download = false;
                // Keep any previously verified artifact available if this download
                // worker fails; the failed attempt must not orphan the staged file.
                None
            }
        };
        if let Some(result) = download_result {
            match result {
                Ok(path) => {
                    self.update_status = if self.language == "fr" {
                        format!("Mise à jour téléchargée et vérifiée : {}. Cliquez sur Installer la mise à jour.", path.display())
                    } else {
                        format!("Update downloaded and verified at {}. Click Install update to continue.", path.display())
                    };
                    self.staged_update_path = Some(path);
                    if self.install_after_update_download {
                        self.install_after_update_download = false;
                        if self.start_update_install(context) {
                            self.update_prompt_open = false;
                            self.update_prompt_dismissed = true;
                        }
                    }
                }
                Err(error) => {
                    self.install_after_update_download = false;
                    self.update_status = if self.language == "fr" {
                        format!("Échec du téléchargement de la mise à jour : {error}")
                    } else {
                        format!("Update download failed: {error}")
                    };
                    // A failed replacement download does not invalidate a previously
                    // verified artifact, so preserve its path for diagnostics/recovery.
                }
            }
        }
        if self.update_check_receiver.is_some() || self.update_download_receiver.is_some() {
            context.request_repaint_after(Duration::from_millis(250));
        }
        self.advance_emulation(context);
        self.show_game_viewport(context);
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

        // Keep the update notification non-blocking; the import-warning dialog stays on top.
        self.show_update_prompt(context);
        // Keep import failures modal and on top of every page until explicitly dismissed.
        self.show_import_warning(context, &palette);

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
    /// Opens a detached, image-only viewport for the active game.
    fn show_game_viewport(&mut self, context: &egui::Context) {
        if self.game_viewport_closed.swap(false, Ordering::AcqRel) {
            self.game_viewport_open = false;
        }
        if !self.game_viewport_open {
            return;
        }

        let texture = self.frame_texture.clone();
        let inputs = Arc::clone(&self.game_viewport_inputs);
        let closed = Arc::clone(&self.game_viewport_closed);
        let builder = egui::ViewportBuilder::default()
            .with_title("PeaceEmu — NES")
            .with_inner_size([768.0, 720.0])
            .with_min_inner_size([256.0, 240.0])
            .with_resizable(true);

        context.show_viewport_deferred(
            egui::ViewportId::from_hash_of("peaceemu-nes-game"),
            builder,
            move |viewport_context, _class| {
                if viewport_context.input(|input| input.viewport().close_requested()) {
                    if let Ok(mut queue) = inputs.lock() {
                        for control in [
                            InputControl::Action1,
                            InputControl::Action2,
                            InputControl::Select,
                            InputControl::Start,
                            InputControl::Up,
                            InputControl::Down,
                            InputControl::Left,
                            InputControl::Right,
                        ] {
                            queue.push(InputEvent { control, pressed: false });
                        }
                    }
                    closed.store(true, Ordering::Release);
                    return;
                }

                let events = viewport_context.input(|input| input.events.clone());
                if let Ok(mut queue) = inputs.lock() {
                    for event in events {
                        match event {
                            egui::Event::Key { key, pressed, repeat, .. } if !(repeat && pressed) => {
                                if let Some(control) = map_key_to_control(key) {
                                    queue.push(InputEvent { control, pressed });
                                }
                            }
                            egui::Event::WindowFocused(false) => {
                                for control in [
                                    InputControl::Action1,
                                    InputControl::Action2,
                                    InputControl::Select,
                                    InputControl::Start,
                                    InputControl::Up,
                                    InputControl::Down,
                                    InputControl::Left,
                                    InputControl::Right,
                                ] {
                                    queue.push(InputEvent { control, pressed: false });
                                }
                            }
                            _ => {}
                        }
                    }
                }

                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(egui::Color32::BLACK))
                    .show(viewport_context, |ui| {
                        if let Some(texture) = texture.as_ref() {
                            let available = ui.available_size();
                            let source = egui::vec2(256.0, 240.0);
                            let scale = (available.x / source.x)
                                .min(available.y / source.y)
                                .max(0.01);
                            let size = source * scale;
                            ui.vertical_centered(|ui| {
                                ui.add_space(((available.y - size.y) * 0.5).max(0.0));
                                ui.image((texture.id(), size));
                            });
                        }
                    });
                viewport_context.request_repaint_after(Duration::from_millis(16));
            },
        );
    }

    /// Shows the import failure dialog until the user dismisses it.
    ///
    /// The dialog is a compact box centered in the window, drawn over a dimmed
    /// backdrop that also swallows pointer input for the pages below it.
    fn show_import_warning(&mut self, context: &egui::Context, palette: &theme::Palette) {
        let Some(message) = self.import_warning.clone() else {
            return;
        };
        if context.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.import_warning = None;
            return;
        }
        let screen_rect = context.screen_rect();
        let layout = ImportDialogLayout::for_screen(screen_rect.size());
        let french = self.language == "fr";

        paint_import_warning_backdrop(context, palette, screen_rect);
        if show_import_warning_dialog(context, palette, &layout, &message, french) {
            self.import_warning = None;
            context.request_repaint();
        }
    }

    fn advance_emulation(&mut self, context: &egui::Context) {
        // Flush a previous tick's dirty battery RAM even if the next tick errors or
        // the active session has since been deselected.
        self.autosave_dirty_game_saves();
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;

        let viewport_events = self
            .game_viewport_inputs
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default();
        for event in viewport_events {
            self.send_game_input(event.control, event.pressed);
        }

        if self.active_page == DesktopPage::Game && !self.game_viewport_open {
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

/// Size limits of the import warning dialog for a given window size.
///
/// The dialog is never given a fixed height: it only receives upper bounds, so
/// it shrinks to its content and never stretches to the height of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ImportDialogLayout {
    /// Outer width of the dialog, border included.
    width: f32,
    /// Largest height the whole dialog may take before it scrolls as a whole.
    max_height: f32,
    /// Largest height of the scrollable error list.
    details_max_height: f32,
}

impl ImportDialogLayout {
    const MAX_WIDTH: f32 = 520.0;
    const MIN_WIDTH: f32 = 220.0;
    /// Free space kept between the dialog and the window edges.
    const SCREEN_MARGIN: f32 = 24.0;
    const INNER_MARGIN: f32 = 22.0;
    const BORDER: f32 = 1.5;
    /// Height taken by everything around the error list (title, message, buttons, margins).
    const CHROME_HEIGHT: f32 = 260.0;
    const MIN_DETAILS: f32 = 48.0;
    const MAX_DETAILS: f32 = 180.0;

    fn for_screen(screen: egui::Vec2) -> Self {
        let margin = 2.0 * Self::SCREEN_MARGIN;
        let width = (screen.x - margin).clamp(Self::MIN_WIDTH, Self::MAX_WIDTH);
        let max_height = (screen.y - margin).max(Self::MIN_DETAILS);
        let details = max_height - Self::CHROME_HEIGHT;
        Self {
            width,
            max_height,
            details_max_height: details.clamp(Self::MIN_DETAILS, Self::MAX_DETAILS),
        }
    }

    /// Width available to the content, once margins and border are removed.
    fn content_width(self) -> f32 {
        self.width - 2.0 * Self::INNER_MARGIN - 2.0 * Self::BORDER
    }

    /// Height limit of the dialog body, used as a fallback on very short windows.
    fn body_max_height(self) -> f32 {
        let chrome = 2.0 * Self::INNER_MARGIN + 2.0 * Self::BORDER;
        (self.max_height - chrome).max(Self::MIN_DETAILS)
    }
}

/// Texts of the import warning dialog in one language.
struct ImportDialogText {
    title: &'static str,
    summary: &'static str,
    details: &'static str,
    causes_title: &'static str,
    causes: [&'static str; 3],
    close: &'static str,
}

fn import_dialog_text(french: bool) -> ImportDialogText {
    if french {
        ImportDialogText {
            title: "⚠ Échec de l’importation",
            summary: "Un ou plusieurs fichiers n’ont pas pu être ajoutés à la bibliothèque.",
            details: "Détails de l’import",
            causes_title: "Causes possibles (non vérifiées)",
            causes: [
                "Le fichier n’est pas une ROM Game Boy ou Game Boy Color valide.",
                "Le format du fichier n’est pas pris en charge.",
                "Le fichier est illisible ou dépasse la taille maximale autorisée.",
            ],
            close: "Fermer",
        }
    } else {
        ImportDialogText {
            title: "⚠ Import warning",
            summary: "One or more files could not be added to the library.",
            details: "Import details",
            causes_title: "Possible causes (not verified)",
            causes: [
                "The file is not a valid Game Boy or Game Boy Color ROM.",
                "The file format is not supported.",
                "The file is unreadable or exceeds the maximum allowed size.",
            ],
            close: "Close",
        }
    }
}

/// Dims the whole window and swallows pointer input while the dialog is open.
///
/// egui 0.27 has no blur primitive, so a plain dark veil is used instead of a
/// fragile screenshot-based blur.
fn paint_import_warning_backdrop(
    context: &egui::Context,
    palette: &theme::Palette,
    screen_rect: egui::Rect,
) {
    let alpha = if palette.dark { 190 } else { 120 };
    let veil = egui::Color32::from_rgba_unmultiplied(8, 10, 16, alpha);
    egui::Area::new(egui::Id::new("import_warning_backdrop"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen_rect.min)
        .show(context, |ui| {
            let size = screen_rect.size();
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
            ui.painter().rect_filled(rect, 0.0, veil);
        });
}

/// Draws the dialog above the backdrop and reports whether it was dismissed.
fn show_import_warning_dialog(
    context: &egui::Context,
    palette: &theme::Palette,
    layout: &ImportDialogLayout,
    message: &str,
    french: bool,
) -> bool {
    let mut dismissed = false;
    // The dialog lives on a higher layer than the backdrop so that clicking the
    // backdrop can never raise it above the dialog.
    egui::Area::new(egui::Id::new("import_warning_dialog"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(context, |ui| {
            import_dialog_frame(ui, palette).show(ui, |ui| {
                ui.set_width(layout.content_width());
                egui::ScrollArea::vertical()
                    .max_height(layout.body_max_height())
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        dismissed = import_dialog_body(ui, palette, layout, message, french);
                    });
            });
        });
    dismissed
}

/// Window frame filled from the palette, outlined with the user's accent colour.
fn import_dialog_frame(ui: &egui::Ui, palette: &theme::Palette) -> egui::Frame {
    egui::Frame::window(ui.style())
        .fill(palette.panel)
        .stroke(egui::Stroke::new(
            ImportDialogLayout::BORDER,
            palette.accent,
        ))
        .rounding(egui::Rounding::same(14.0))
        .shadow(egui::epaint::Shadow {
            offset: egui::vec2(0.0, 12.0),
            blur: 28.0,
            spread: 0.0,
            color: egui::Color32::from_black_alpha(100),
        })
        .inner_margin(egui::Margin::same(ImportDialogLayout::INNER_MARGIN))
}

/// Title, summary, error list and close button. Returns true once dismissed.
fn import_dialog_body(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    layout: &ImportDialogLayout,
    message: &str,
    french: bool,
) -> bool {
    let text = import_dialog_text(french);

    ui.label(
        egui::RichText::new(text.title)
            .size(20.0)
            .strong()
            .color(palette.text),
    );
    let summary = egui::RichText::new(text.summary).size(14.0);
    ui.label(summary.color(palette.text));

    ui.label(
        egui::RichText::new(text.details)
            .strong()
            .color(palette.text),
    );
    // Only the error list scrolls; the title and the button stay in view.
    egui::Frame::none()
        .fill(palette.sidebar)
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(layout.details_max_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let details = egui::RichText::new(message).size(13.0);
                    ui.label(details.color(palette.text));
                });
        });

    // These causes are guesses, not findings: keep them folded and labelled.
    let causes_title = egui::RichText::new(text.causes_title)
        .strong()
        .color(palette.text_dim);
    egui::CollapsingHeader::new(causes_title)
        .id_source("import_warning_possible_causes")
        .default_open(false)
        .show(ui, |ui| {
            for cause in text.causes {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("•").color(palette.accent_text));
                    ui.label(egui::RichText::new(cause).color(palette.text_dim));
                });
            }
        });

    let mut dismissed = false;
    let button_row = egui::Layout::right_to_left(egui::Align::Center);
    ui.with_layout(button_row, |ui| {
        let close = ui.add(theme::primary_button(palette, text.close));
        if close.clicked() {
            dismissed = true;
        }
    });
    dismissed
}

/// Rejects unsupported extensions and non-ROM bytes before adding a file to the library.
fn validate_import_file(path: &Path) -> Result<(), String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("gb")
        && !extension.eq_ignore_ascii_case("gbc")
        && !extension.eq_ignore_ascii_case("gba")
    {
        return Err("unsupported file type: choose a .gb, .gbc or .gba ROM".to_owned());
    }

    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("could not read file metadata: {error}"))?;
    if !metadata.is_file() {
        return Err("the selected path is not a file".to_owned());
    }
    if metadata.len() == 0 {
        return Err("the file is empty".to_owned());
    }
    if metadata.len() > MAX_CONTENT_BYTES as u64 {
        return Err(format!(
            "file exceeds the {} MiB import limit",
            MAX_CONTENT_BYTES / (1024 * 1024)
        ));
    }

    // Importing a ROM into the library is supported before its emulator core exists.
    // Actual format/core validation belongs to launch time, not library import.
    Ok(())
}

fn format_import_summary(
    language: &str,
    imported: usize,
    rejected: usize,
    failures: &[String],
) -> String {
    let french = language == "fr";
    if rejected == 0 {
        return if french {
            format!("{imported} ROM(s) importée(s) dans la bibliothèque.")
        } else {
            format!("Imported {imported} ROM(s) into your library.")
        };
    }

    let details = if failures.is_empty() {
        String::new()
    } else {
        let omitted = rejected.saturating_sub(failures.len());
        let mut details = failures.join("; ");
        if omitted > 0 {
            if french {
                details.push_str(&format!(" ; et {omitted} autre(s) erreur(s)"));
            } else {
                details.push_str(&format!("; and {omitted} more error(s)"));
            }
        }
        if french {
            format!(" Détails : {details}")
        } else {
            format!(" Details: {details}")
        }
    };

    if french {
        format!("{imported} ROM(s) importée(s) ; {rejected} fichier(s) ignoré(s) ou refusé(s).{details}")
    } else {
        format!("Imported {imported} ROM(s); {rejected} file(s) skipped or rejected.{details}")
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
mod import_dialog_tests {
    use super::*;

    #[test]
    fn dialog_stays_compact_on_large_windows() {
        let layout = ImportDialogLayout::for_screen(egui::vec2(3840.0, 2160.0));
        assert_eq!(layout.width, ImportDialogLayout::MAX_WIDTH);
        assert_eq!(layout.details_max_height, ImportDialogLayout::MAX_DETAILS);
        assert!(layout.content_width() < ImportDialogLayout::MAX_WIDTH);
    }

    #[test]
    fn dialog_adapts_to_narrow_windows() {
        let layout = ImportDialogLayout::for_screen(egui::vec2(360.0, 800.0));
        assert!(layout.width <= 360.0 - 2.0 * ImportDialogLayout::SCREEN_MARGIN);
        assert!(layout.content_width() > 0.0);
    }

    #[test]
    fn dialog_scrolls_as_a_whole_on_very_short_windows() {
        let screen_height = 240.0;
        let layout = ImportDialogLayout::for_screen(egui::vec2(1280.0, screen_height));
        assert!(layout.max_height < screen_height);
        assert_eq!(layout.details_max_height, ImportDialogLayout::MIN_DETAILS);
        assert!(layout.body_max_height() > 0.0);
    }

    #[test]
    fn error_list_height_grows_with_the_window_but_is_capped() {
        let short = ImportDialogLayout::for_screen(egui::vec2(1280.0, 420.0));
        let tall = ImportDialogLayout::for_screen(egui::vec2(1280.0, 1440.0));
        assert!(short.details_max_height < tall.details_max_height);
        assert_eq!(tall.details_max_height, ImportDialogLayout::MAX_DETAILS);
    }

    #[test]
    fn dialog_texts_are_complete_in_both_languages() {
        for french in [true, false] {
            let text = import_dialog_text(french);
            assert!(!text.title.is_empty());
            assert!(!text.summary.is_empty());
            assert!(!text.details.is_empty());
            assert!(!text.close.is_empty());
            assert!(text.causes.iter().all(|cause| !cause.is_empty()));
        }
        assert_ne!(
            import_dialog_text(true).close,
            import_dialog_text(false).close
        );
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
    fn missing_configuration_directory_error_is_localized() {
        assert_eq!(
            preferences_unavailable_message("fr"),
            "Impossible d’enregistrer les préférences : aucun dossier de configuration utilisateur accessible en écriture."
        );
        assert_eq!(
            preferences_unavailable_message("en"),
            "Could not save preferences: no writable user configuration directory is available."
        );
        assert_eq!(
            preferences_unavailable_message("de"),
            "Could not save preferences: no writable user configuration directory is available."
        );
    }

    #[test]
    fn launch_cancellation_is_classified_as_an_error() {
        assert!(message_indicates_error(
            "Launch cancelled: could not release inputs from session 3"
        ));
        assert!(!message_indicates_error("Launched session 3."));
    }

    #[test]
    fn native_window_icon_uses_the_bundled_peaceemu_branding() {
        let icon = application_icon().expect("bundled PeaceEmu icon must decode");
        assert!(icon.width > 0, "icon width must be non-zero");
        assert!(icon.height > 0, "icon height must be non-zero");
        assert!(
            icon.width <= 256 && icon.height <= 256,
            "native window icon must stay within 256x256 to avoid oversized X11 requests"
        );
        assert_eq!(
            icon.rgba.len(),
            icon.width as usize * icon.height as usize * 4,
            "icon RGBA buffer must match the decoded image dimensions"
        );
    }

    #[test]
    fn command_line_help_and_version_are_available_without_starting_the_gui() {
        assert_eq!(
            cli_message("--version").unwrap(),
            concat!("PeaceEmu ", env!("CARGO_PKG_VERSION"))
        );
        assert!(cli_message("--help").unwrap().contains("--version"));
        assert!(cli_message("-h").unwrap().contains("Usage: PeaceEmu"));
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
    fn import_summary_reports_hidden_failures_in_both_languages() {
        let failures = vec![
            "a.gb: invalid header".to_owned(),
            "b.gb: unsupported format".to_owned(),
            "c.gb: file too large".to_owned(),
        ];

        let english = format_import_summary("en", 1, 5, &failures);
        assert!(english.contains("Imported 1 ROM(s); 5 file(s) skipped or rejected."));
        assert!(english.contains("a.gb: invalid header"));
        assert!(english.contains("and 2 more error(s)"));

        let french = format_import_summary("fr", 1, 5, &failures);
        assert!(french.contains("1 ROM(s) importée(s) ; 5 fichier(s) ignoré(s) ou refusé(s)."));
        assert!(french.contains("a.gb: invalid header"));
        assert!(french.contains("et 2 autre(s) erreur(s)"));
    }

    #[test]
    fn successful_import_summary_does_not_include_failure_details() {
        assert_eq!(
            format_import_summary("en", 2, 0, &[]),
            "Imported 2 ROM(s) into your library."
        );
        assert_eq!(
            format_import_summary("fr", 2, 0, &[]),
            "2 ROM(s) importée(s) dans la bibliothèque."
        );
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

    #[test]
    fn clearing_one_session_tick_error_preserves_other_sessions() {
        let first = 1_u64;
        let second = 2_u64;
        let mut errors = vec![
            (first, "first session failed".to_owned()),
            (second, "second session failed".to_owned()),
        ];

        clear_tick_error_for_session(&mut errors, first);

        assert_eq!(errors, vec![(second, "second session failed".to_owned())]);
    }

    #[test]
    fn desktop_page_modules_do_not_reach_into_runtime_internals() {
        let pages = [
            ("appearance_page.rs", include_str!("appearance_page.rs")),
            ("game_page.rs", include_str!("game_page.rs")),
            ("home_page.rs", include_str!("home_page.rs")),
            ("library_page.rs", include_str!("library_page.rs")),
            ("sessions_page.rs", include_str!("sessions_page.rs")),
            ("settings_page.rs", include_str!("settings_page.rs")),
            ("systems_page.rs", include_str!("systems_page.rs")),
            ("shell.rs", include_str!("shell.rs")),
            ("game_view.rs", include_str!("game_view.rs")),
        ];
        let forbidden = [
            "HostRuntime",
            "EmulationSession",
            "emulation_core::host_runtime",
            ".host(",
            ".host_mut(",
            ".session(",
            ".session_mut(",
        ];

        for (path, source) in pages {
            let compact: String = source.chars().filter(|ch| !ch.is_whitespace()).collect();
            for token in forbidden {
                assert!(
                    !compact.contains(token),
                    "{path} must use ApplicationController APIs; found forbidden token {token}"
                );
            }
        }
    }

    #[test]
    fn library_format_filter_is_case_insensitive() {
        assert!(LibraryFormat::GameBoy.matches(Path::new("Pokemon.GB")));
        assert!(LibraryFormat::GameBoy.matches(Path::new("Pokemon.GbC")));
        assert!(LibraryFormat::GameBoy.matches(Path::new("Pokemon.GBA")));
        assert!(!LibraryFormat::GameBoy.matches(Path::new("Pokemon.zip")));
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
