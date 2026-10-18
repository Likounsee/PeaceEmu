//! Application settings page: about, updates, core settings and diagnostics.
//!
//! Every block is a `theme::card`, statuses are `theme::badge` pills, and all
//! colours come from the shared [`theme::Palette`] so the page follows the
//! light/dark mode and the accent colour chosen in the Appearance page.

use super::*;

/// State of the update section, derived once per frame.
#[derive(Clone, Copy, PartialEq, Eq)]
enum UpdateState {
    Unknown,
    Checking,
    Available,
    UpToDate,
    InstalledIsNewer,
    Failed,
}

/// A small section title with a one-line description, drawn above a card.
fn section_heading(ui: &mut egui::Ui, palette: &theme::Palette, title: &str, description: &str) {
    ui.add_space(18.0);
    ui.label(egui::RichText::new(title).size(17.0).strong());
    ui.label(
        egui::RichText::new(description)
            .size(13.0)
            .color(palette.text_dim),
    );
    ui.add_space(4.0);
}

/// One "label: value" line. The value wraps instead of overflowing the card,
/// which matters for long file paths.
fn key_value(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    label: &str,
    value: &str,
    monospace: bool,
) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(190.0);
            ui.label(
                egui::RichText::new(label)
                    .size(13.0)
                    .color(palette.text_dim),
            );
        });
        ui.vertical(|ui| {
            let text = egui::RichText::new(value).size(13.0);
            ui.label(if monospace { text.monospace() } else { text });
        });
    });
}

impl DesktopApp {
    pub(super) fn show_settings(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        match self.settings_section {
            SettingsSection::Appearance => self.show_appearance(ui),
            SettingsSection::Application => self.show_application_settings(ui, context),
        }
    }

    fn show_application_settings(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        let palette = theme::Palette::new(self.dark_mode, self.accent_color);
        let french = self.language == "fr";
        let current = option_env!("CARGO_PKG_VERSION").unwrap_or("0.0.0");

        ui.label(
            egui::RichText::new(self.language_pack.text("settings.application.title"))
                .size(30.0)
                .strong(),
        );
        ui.label(
            egui::RichText::new(self.language_pack.text("settings.application.description"))
                .color(palette.text_dim),
        );
        ui.add_space(8.0);
        ui.separator();

        // ---------------------------------------------------------------- About
        section_heading(
            ui,
            &palette,
            self.language_pack.text("settings.about.title"),
            self.language_pack.text("settings.about.description"),
        );
        theme::card(ui, &palette, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("PeaceEmu").size(20.0).strong());
                        theme::badge(ui, &format!("v{current}"), palette.text_dim);
                        theme::badge(
                            ui,
                            self.language_pack.text("status.experimental"),
                            palette.warning,
                        );
                    });
                    ui.label(
                        egui::RichText::new(self.language_pack.text("about.development"))
                            .color(palette.text_dim),
                    );
                });
            });
            ui.add_space(6.0);
            ui.label(self.language_pack.text("about.core_notice"));
            ui.label(
                egui::RichText::new(self.language_pack.text("about.paths_saved"))
                    .size(13.0)
                    .color(palette.text_dim),
            );
            ui.add_space(4.0);
            if ui
                .button(self.language_pack.text("settings.save"))
                .clicked()
                && self.save_preferences()
            {
                self.message = self.language_pack.text("settings.saved").to_owned();
            }
        });

        // -------------------------------------------------------------- Updates
        section_heading(
            ui,
            &palette,
            self.language_pack.text("ui.settings_page.001"),
            self.language_pack.text("ui.settings_page.002"),
        );
        let checking = self.update_check_receiver.is_some();
        let downloading = self.update_download_receiver.is_some();
        let update_info = self.update_manifest.as_ref().map(|manifest| {
            (
                manifest.version.clone(),
                manifest.release_notes_url.clone(),
                manifest.download_for_current_platform().cloned(),
            )
        });
        let update_state = if checking {
            UpdateState::Checking
        } else if self.update_manifest.is_none()
            && (self.update_status.starts_with("Échec")
                || self.update_status.starts_with("Update check failed")
                || self.update_status.starts_with("Update verification failed"))
        {
            UpdateState::Failed
        } else if let Some((manifest_version, _, _)) = update_info.as_ref() {
            if update::is_newer_version(manifest_version, current) {
                UpdateState::Available
            } else if update::is_newer_version(current, manifest_version) {
                UpdateState::InstalledIsNewer
            } else {
                UpdateState::UpToDate
            }
        } else {
            UpdateState::Unknown
        };
        let (manifest_version, release_notes_url, download) = match update_info {
            Some((version, url, download)) => (Some(version), Some(url), download),
            None => (None, None, None),
        };
        let has_download = download.is_some();
        theme::card(ui, &palette, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                match update_state {
                    UpdateState::Checking => theme::badge(
                        ui,
                        self.language_pack.text("ui.settings_page.003"),
                        palette.accent,
                    ),
                    UpdateState::Available => theme::badge(
                        ui,
                        self.language_pack.text("ui.settings_page.004"),
                        palette.accent,
                    ),
                    UpdateState::UpToDate => theme::badge(
                        ui,
                        self.language_pack.text("ui.settings_page.005"),
                        palette.success,
                    ),
                    UpdateState::InstalledIsNewer => theme::badge(
                        ui,
                        self.language_pack.text("ui.settings_page.006"),
                        palette.warning,
                    ),
                    UpdateState::Unknown => theme::badge(
                        ui,
                        self.language_pack.text("ui.settings_page.007"),
                        palette.text_dim,
                    ),
                    UpdateState::Failed => {
                        theme::badge(ui, self.language_pack.text("ui.settings_page.038"), palette.danger)
                    }
                }
                ui.label(
                    egui::RichText::new(&self.update_status)
                        .size(13.0)
                        .color(palette.text_dim),
                );
            });

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !checking,
                        egui::Button::new(self.language_pack.text("ui.settings_page.008")),
                    )
                    .clicked()
                {
                    self.start_update_check();
                    context.request_repaint_after(Duration::from_millis(250));
                }
                if update_state == UpdateState::Available {
                    if let (Some(download), Some(version)) = (download, manifest_version.clone()) {
                        if ui
                            .add_enabled(
                                !downloading,
                                theme::primary_button(
                                    &palette,
                                    self.language_pack.text("ui.settings_page.009"),
                                ),
                            )
                            .clicked()
                        {
                            let (sender, receiver) = mpsc::channel();
                            self.update_download_receiver = Some(receiver);
                            self.update_status = if french {
                                format!("Téléchargement de la version {version} dans la zone temporaire…")
                            } else {
                                format!("Downloading version {version} to a temporary staging area…")
                            };
                            std::thread::spawn(move || {
                                let result = update::download_and_verify(&download, &version);
                                let _ = sender.send(result);
                            });
                            context.request_repaint_after(Duration::from_millis(250));
                        }
                    }
                }
            });
            if update_state == UpdateState::Available && !has_download {
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.settings_page.010"))
                        .size(13.0)
                        .color(palette.text_dim),
                );
            }

            if let Some(version) = manifest_version.as_ref() {
                ui.separator();
                key_value(
                    ui,
                    &palette,
                    self.language_pack.text("ui.settings_page.011"),
                    current,
                    true,
                );
                key_value(
                    ui,
                    &palette,
                    self.language_pack.text("ui.settings_page.012"),
                    &version.to_string(),
                    true,
                );
                if let Some(url) = release_notes_url.as_ref() {
                    ui.hyperlink_to(self.language_pack.text("ui.settings_page.013"), url);
                }
            }
            if let Some(path) = self.staged_update_path.as_ref() {
                ui.separator();
                key_value(
                    ui,
                    &palette,
                    self.language_pack.text("ui.settings_page.014"),
                    &path.display().to_string(),
                    true,
                );
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.settings_page.015"))
                        .size(13.0)
                        .color(palette.text_dim),
                );
            }
            ui.label(
                egui::RichText::new(self.language_pack.text("ui.settings_page.016"))
                    .size(12.0)
                    .color(palette.text_dim),
            );
        });

        // ---------------------------------------------------------- Core settings
        section_heading(
            ui,
            &palette,
            self.language_pack.text("ui.settings_page.017"),
            self.language_pack.text("ui.settings_page.018"),
        );
        theme::card(ui, &palette, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                theme::badge(
                    ui,
                    self.language_pack.text("ui.settings_page.019"),
                    palette.text_dim,
                );
                ui.label(self.language_pack.text("ui.settings_page.020"));
            });
            ui.label(
                egui::RichText::new(self.language_pack.text("ui.settings_page.021"))
                    .size(13.0)
                    .color(palette.text_dim),
            );
        });

        // ----------------------------------------------------------- Diagnostics
        section_heading(
            ui,
            &palette,
            "Diagnostic",
            self.language_pack.text("ui.settings_page.022"),
        );
        let none_text = self.language_pack.text("ui.settings_page.023");
        let unavailable_text = self.language_pack.text("ui.settings_page.024");
        let platform = format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH);
        let library_items = self.controller.library().len().to_string();
        let open_sessions = self.controller.session_summaries().len().to_string();
        let error_count = self.last_tick_errors.len().to_string();
        let active_session = self
            .controller
            .active_session()
            .map(|id| id.get().to_string())
            .unwrap_or_else(|| none_text.to_owned());
        let config_path = self
            .config_path
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| unavailable_text.to_owned());
        let staged_path = self
            .staged_update_path
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| none_text.to_owned());
        theme::card(ui, &palette, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            key_value(
                ui,
                &palette,
                self.language_pack.text("ui.settings_page.025"),
                current,
                true,
            );
            key_value(
                ui,
                &palette,
                self.language_pack.text("ui.settings_page.026"),
                &platform,
                true,
            );
            key_value(
                ui,
                &palette,
                self.language_pack.text("ui.settings_page.027"),
                &library_items,
                false,
            );
            key_value(
                ui,
                &palette,
                self.language_pack.text("ui.settings_page.028"),
                &open_sessions,
                false,
            );
            key_value(
                ui,
                &palette,
                self.language_pack.text("ui.settings_page.029"),
                &active_session,
                true,
            );
            key_value(
                ui,
                &palette,
                self.language_pack.text("ui.settings_page.030"),
                &error_count,
                false,
            );
            ui.separator();
            key_value(
                ui,
                &palette,
                self.language_pack.text("ui.settings_page.031"),
                &config_path,
                true,
            );
            key_value(
                ui,
                &palette,
                self.language_pack.text("ui.settings_page.032"),
                &self.media_status,
                false,
            );
            key_value(
                ui,
                &palette,
                self.language_pack.text("ui.settings_page.033"),
                &self.update_status,
                false,
            );
            key_value(
                ui,
                &palette,
                self.language_pack.text("ui.settings_page.034"),
                &staged_path,
                true,
            );
            key_value(
                ui,
                &palette,
                self.language_pack.text("ui.settings_page.035"),
                &self.message,
                false,
            );
            if !self.last_tick_errors.is_empty() {
                ui.separator();
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.settings_page.036"))
                        .strong()
                        .color(palette.danger),
                );
                for (session_id, error) in self.last_tick_errors.iter().take(5) {
                    ui.label(if french {
                        format!("Session {} : {error}", session_id.get())
                    } else {
                        format!("Session {}: {error}", session_id.get())
                    });
                }
            }
            ui.add_space(4.0);
            if ui
                .button(self.language_pack.text("ui.settings_page.037"))
                .clicked()
            {
                let report = self.diagnostics_report(french);
                ui.output_mut(|output| output.copied_text = report);
                self.message = if french {
                    "Diagnostic copié dans le presse-papiers.".to_owned()
                } else {
                    "Diagnostics copied to clipboard.".to_owned()
                };
            }
        });
        ui.add_space(16.0);
    }

    /// Plain-text report copied by the "Copy diagnostics" button.
    fn diagnostics_report(&self, french: bool) -> String {
        let recent_errors = self
            .last_tick_errors
            .iter()
            .take(5)
            .map(|(session_id, error)| {
                if french {
                    format!("Session {} : {error}", session_id.get())
                } else {
                    format!("Session {}: {error}", session_id.get())
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let active_session = self
            .controller
            .active_session()
            .map(|id| id.get().to_string())
            .unwrap_or_else(|| "none".to_owned());
        let config = self
            .config_path
            .as_deref()
            .map(|path| path.display().to_string());
        let staged = self
            .staged_update_path
            .as_deref()
            .map(|path| path.display().to_string());
        let version = option_env!("CARGO_PKG_VERSION");
        if french {
            format!(
                "PeaceEmu {}\nPlateforme : {} / {}\nÉléments de la bibliothèque : {}\nSessions ouvertes : {}\nSession active : {}\nErreurs d’exécution : {}\nConfiguration : {}\nÉtat des médias : {}\nÉtat des mises à jour : {}\nMise à jour préparée : {}\nDernier message : {}\nErreurs récentes :\n{}",
                version.unwrap_or("inconnue"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                self.controller.library().len(),
                self.controller.session_summaries().len(),
                active_session,
                self.last_tick_errors.len(),
                config.unwrap_or_else(|| "indisponible".to_owned()),
                self.media_status,
                self.update_status,
                staged.unwrap_or_else(|| "aucune".to_owned()),
                self.message,
                recent_errors
            )
        } else {
            format!(
                "PeaceEmu {}\nPlatform: {} / {}\nLibrary items: {}\nOpen sessions: {}\nActive session: {}\nExecution errors: {}\nConfiguration: {}\nMedia status: {}\nUpdate status: {}\nStaged update: {}\nLatest message: {}\nRecent errors:\n{}",
                version.unwrap_or("unknown"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                self.controller.library().len(),
                self.controller.session_summaries().len(),
                active_session,
                self.last_tick_errors.len(),
                config.unwrap_or_else(|| "unavailable".to_owned()),
                self.media_status,
                self.update_status,
                staged.unwrap_or_else(|| "none".to_owned()),
                self.message,
                recent_errors
            )
        }
    }
}
