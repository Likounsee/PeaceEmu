//! Sessions page for the PeaceEmu desktop frontend.
//!
//! Running sessions are full-width rows in an "Active" category; every other
//! session (paused or otherwise stopped) is a small tile with the game icon.

use super::*;

/// Outer width of a tile in the "Stopped" category.
const TILE_WIDTH: f32 = 170.0;

pub(crate) fn state_label(state: &SessionState, french: bool) -> String {
    match (state, french) {
        (SessionState::Empty, true) => "Vide".to_owned(),
        (SessionState::Empty, false) => "Empty".to_owned(),
        (SessionState::Running, true) => "En cours".to_owned(),
        (SessionState::Running, false) => "Running".to_owned(),
        (SessionState::Paused, true) => "En pause".to_owned(),
        (SessionState::Paused, false) => "Paused".to_owned(),
        (SessionState::Halted, true) => "Arrêtée par le cœur".to_owned(),
        (SessionState::Halted, false) => "Halted by core".to_owned(),
        (SessionState::Stopped, true) => "Arrêtée".to_owned(),
        (SessionState::Stopped, false) => "Stopped".to_owned(),
    }
}

pub(crate) fn state_color(state: &SessionState, palette: &theme::Palette) -> egui::Color32 {
    if *state == SessionState::Running {
        palette.success
    } else if *state == SessionState::Paused {
        palette.warning
    } else {
        palette.text_dim
    }
}

/// Draws the flat game icon (coloured tile with the title's initials).
fn paint_cover(ui: &mut egui::Ui, title: &str, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, egui::Rounding::same(10.0), theme::cover_color(title));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        theme::initials(title),
        egui::FontId::proportional(size.y * 0.4),
        egui::Color32::WHITE,
    );
}

fn section_title(ui: &mut egui::Ui, palette: &theme::Palette, title: &str, count: usize) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .size(theme::FONT_SECTION)
                .strong(),
        );
        theme::badge(ui, &count.to_string(), palette.text_dim);
    });
}

fn shorten(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut shortened: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

impl DesktopApp {
    pub(super) fn show_sessions(&mut self, ui: &mut egui::Ui) {
        let palette = theme::Palette::new(self.dark_mode, self.accent_color);
        let french = self.language == "fr";
        let session_count = self.controller.session_count();
        let mut close_all = false;
        let mut closing: Option<SessionId> = None;
        let mut opening: Option<SessionId> = None;

        theme::page_header(
            ui,
            &palette,
            self.language_pack.text("ui.sessions_page.002"),
            self.language_pack.text("ui.sessions_page.003"),
            |ui| {
                if ui
                    .add_enabled(
                        session_count > 0,
                        egui::Button::new(self.language_pack.text("ui.sessions_page.004")),
                    )
                    .clicked()
                {
                    close_all = true;
                }
                theme::badge(ui, &format!("{session_count} session(s)"), palette.text_dim);
            },
        );

        let sessions = self.controller.session_summaries();
        if sessions.is_empty() {
            theme::card(ui, &palette, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.sessions_page.005"))
                        .size(18.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.sessions_page.006"))
                        .color(palette.text_dim),
                );
                if ui
                    .add(theme::primary_button(
                        &palette,
                        self.language_pack.text("ui.sessions_page.007"),
                    ))
                    .clicked()
                {
                    self.navigate_to(DesktopPage::Library);
                }
            });
        }
        let (active, stopped): (Vec<_>, Vec<_>) = sessions
            .into_iter()
            .partition(|summary| summary.state == SessionState::Running);

        // ---------------------------------------------------------- Active (long rows)
        if !active.is_empty() {
            section_title(
                ui,
                &palette,
                if french { "Actives" } else { "Active" },
                active.len(),
            );
        }
        for summary in active {
            let id = summary.id;
            let state = summary.state;
            let content_loaded = summary.content_loaded;
            let save_states = summary.save_states;
            let persistent_saves = summary.persistent_saves;
            let system_name = summary.system_name;
            let video_available = summary.video_available;
            let audio_available = summary.audio_available;
            let core_error = summary.last_error;
            let tick_error = self
                .last_tick_errors
                .iter()
                .find(|(error_id, _)| *error_id == id)
                .map(|(_, error)| error.clone());
            let is_active = self.controller.active_session() == Some(id);
            let title = self
                .session_content_paths
                .iter()
                .find(|(session_id, _)| *session_id == id)
                .and_then(|(_, path)| path.file_stem())
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| system_name.clone());

            theme::card(ui, &palette, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    paint_cover(ui, &title, egui::vec2(52.0, 52.0));
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(format!("Session {} · {title}", id.get()))
                                    .size(16.0)
                                    .strong(),
                            );
                            theme::badge(
                                ui,
                                &state_label(&state, french),
                                state_color(&state, &palette),
                            );
                            if is_active {
                                theme::badge(ui, "Active", palette.accent);
                            }
                        });
                        ui.label(egui::RichText::new(&system_name).color(palette.text_dim));
                    });
                });
                if let Some((_, content_path)) = self
                    .session_content_paths
                    .iter()
                    .find(|(session_id, _)| *session_id == id)
                {
                    ui.label(
                        egui::RichText::new(if french {
                            format!("Source : {}", content_path.display())
                        } else {
                            format!("Source: {}", content_path.display())
                        })
                        .size(13.0)
                        .color(palette.text_dim),
                    );
                } else if content_loaded {
                    ui.label(
                        egui::RichText::new(self.language_pack.text("ui.sessions_page.016"))
                            .size(13.0)
                            .color(palette.text_dim),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    for (label, on) in [
                        (if french { "Contenu" } else { "Content" }, content_loaded),
                        (if french { "Vidéo" } else { "Video" }, video_available),
                        (
                            if french {
                                "Audio du cœur"
                            } else {
                                "Core audio"
                            },
                            audio_available,
                        ),
                    ] {
                        theme::badge(
                            ui,
                            &format!("{} {label}", if on { "●" } else { "○" }),
                            if on {
                                palette.success
                            } else {
                                palette.text_dim
                            },
                        );
                    }
                });
                if let Some(error) = &tick_error {
                    ui.colored_label(
                        palette.danger,
                        format!(
                            "{}: {error}",
                            self.language_pack.text("ui.sessions_page.025")
                        ),
                    );
                }
                if let Some(error) = &core_error {
                    ui.colored_label(
                        palette.danger,
                        format!(
                            "{}: {error}",
                            self.language_pack.text("ui.sessions_page.026")
                        ),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add(theme::primary_button(
                            &palette,
                            if french { "Ouvrir" } else { "Open" },
                        ))
                        .clicked()
                    {
                        opening = Some(id);
                    }
                    if ui
                        .button(self.language_pack.text("ui.sessions_page.018"))
                        .clicked()
                    {
                        self.session_action(id, SessionAction::Pause);
                    }
                    if content_loaded && save_states {
                        if ui
                            .button(self.language_pack.text("ui.sessions_page.020"))
                            .clicked()
                        {
                            self.save_state_action(id);
                        }
                        if ui
                            .button(self.language_pack.text("ui.sessions_page.021"))
                            .clicked()
                        {
                            self.load_state_action(id);
                        }
                        if ui
                            .button(self.language_pack.text("ui.sessions_page.022"))
                            .clicked()
                        {
                            self.session_action(id, SessionAction::StopAndSave);
                        }
                    }
                    if content_loaded
                        && persistent_saves
                        && ui
                            .button(self.language_pack.text("ui.sessions_page.023"))
                            .clicked()
                    {
                        self.save_game_data_action(id);
                    }
                    if ui
                        .button(self.language_pack.text("ui.sessions_page.024"))
                        .clicked()
                    {
                        self.session_action(id, SessionAction::Reset);
                    }
                    if ui
                        .button(self.language_pack.text("ui.sessions_page.015"))
                        .clicked()
                    {
                        closing = Some(id);
                    }
                });
                egui::CollapsingHeader::new(self.language_pack.text("ui.sessions_page.027"))
                    .id_source(("session_diagnostics", id))
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label(format!(
                            "{}: {}",
                            self.language_pack.text("ui.sessions_page.028"),
                            id.get()
                        ));
                        ui.label(format!(
                            "{}: {is_active}",
                            self.language_pack.text("ui.sessions_page.029")
                        ));
                        ui.label(format!(
                            "{}: {state:?}",
                            self.language_pack.text("ui.sessions_page.030")
                        ));
                        ui.label(format!(
                            "{}: {content_loaded}",
                            self.language_pack.text("ui.sessions_page.031")
                        ));
                        ui.label(format!(
                            "{}: {}",
                            self.language_pack.text("ui.sessions_page.032"),
                            if french {
                                if video_available {
                                    "déclarée disponible"
                                } else {
                                    "indisponible dans ce cœur"
                                }
                            } else if video_available {
                                "declared available"
                            } else {
                                "unavailable in this core"
                            }
                        ));
                        ui.label(format!(
                            "{}: {}",
                            self.language_pack.text("ui.sessions_page.033"),
                            if french {
                                if audio_available {
                                    "déclarée disponible"
                                } else {
                                    "indisponible dans ce cœur"
                                }
                            } else if audio_available {
                                "declared available"
                            } else {
                                "unavailable in this core"
                            }
                        ));
                        if !video_available {
                            ui.label(self.language_pack.text("ui.sessions_page.034"));
                        }
                        if !audio_available {
                            ui.label(self.language_pack.text("ui.sessions_page.035"));
                        }
                        if let Some(error) = &tick_error {
                            ui.colored_label(
                                palette.danger,
                                if french {
                                    format!("Dernière erreur d’exécution/média : {error}")
                                } else {
                                    format!("Most recent tick/media failure: {error}")
                                },
                            );
                        } else {
                            ui.label(self.language_pack.text("ui.sessions_page.036"));
                        }
                        if let Some(error) = &core_error {
                            ui.colored_label(
                                palette.danger,
                                format!(
                                    "{}: {error}",
                                    self.language_pack.text("ui.sessions_page.037")
                                ),
                            );
                        } else {
                            ui.label(self.language_pack.text("ui.sessions_page.038"));
                        }
                        let source_path = self
                            .session_content_paths
                            .iter()
                            .find(|(session_id, _)| *session_id == id)
                            .map(|(_, path)| path.display().to_string())
                            .unwrap_or_else(|| {
                                if french {
                                    "indisponible".to_owned()
                                } else {
                                    "unavailable".to_owned()
                                }
                            });
                        let tick_diagnostic = tick_error
                            .as_deref()
                            .unwrap_or(self.language_pack.text("ui.sessions_page.039"));
                        let core_diagnostic_text = core_error.as_deref().unwrap_or(
                            if french {
                                "aucune enregistrée"
                            } else {
                                "none recorded"
                            },
                        );
                        let report = if french {
                            format!(
                                "Diagnostic de session PeaceEmu\nID de session : {}\nActive : {}\nÉtat : {}\nSystème : {}\nContenu chargé : {}\nChemin source : {}\nCapacité vidéo : {}\nCapacité audio : {}\nDernière erreur d’exécution/média : {}\nDernière erreur du cœur : {}",
                                id.get(),
                                if is_active { "oui" } else { "non" },
                                state_label(&state, true),
                                system_name,
                                if content_loaded { "oui" } else { "non" },
                                source_path,
                                if video_available { "disponible" } else { "indisponible" },
                                if audio_available { "disponible" } else { "indisponible" },
                                tick_diagnostic,
                                core_diagnostic_text,
                            )
                        } else {
                            format!(
                                "PeaceEmu session diagnostics\nSession ID: {}\nActive: {}\nState: {:?}\nSystem: {}\nContent loaded: {}\nSource path: {}\nVideo capability: {}\nAudio capability: {}\nLast tick/media error: {}\nLast core error: {}",
                                id.get(),
                                is_active,
                                state,
                                system_name,
                                content_loaded,
                                source_path,
                                if video_available { "available" } else { "unavailable" },
                                if audio_available { "available" } else { "unavailable" },
                                tick_diagnostic,
                                core_diagnostic_text,
                            )
                        };
                        if ui
                            .button(self.language_pack.text("ui.sessions_page.040"))
                            .clicked()
                        {
                            ui.output_mut(|output| output.copied_text = report);
                            self.message = if french {
                                format!(
                                    "Diagnostic de la session {} copié dans le presse-papiers.",
                                    id.get()
                                )
                            } else {
                                format!(
                                    "Copied diagnostics for session {} to the clipboard.",
                                    id.get()
                                )
                            };
                        }
                    });
            });
        }

        // ------------------------------------------------- Stopped (small tiles)
        if !stopped.is_empty() {
            section_title(
                ui,
                &palette,
                if french { "Arrêtées" } else { "Stopped" },
                stopped.len(),
            );
            ui.horizontal_wrapped(|ui| {
                for summary in stopped {
                    let id = summary.id;
                    let state = summary.state;
                    let system_name = summary.system_name;
                    let title = self
                        .session_content_paths
                        .iter()
                        .find(|(session_id, _)| *session_id == id)
                        .and_then(|(_, path)| path.file_stem())
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_else(|| system_name.clone());
                    theme::card(ui, &palette, |ui| {
                        let inner = TILE_WIDTH - 32.0;
                        ui.set_width(inner);
                        paint_cover(ui, &title, egui::vec2(inner, 64.0));
                        ui.label(egui::RichText::new(shorten(&title, 20)).strong())
                            .on_hover_text(&title);
                        ui.label(
                            egui::RichText::new(format!("Session {} · {system_name}", id.get()))
                                .size(12.0)
                                .color(palette.text_dim),
                        );
                        theme::badge(
                            ui,
                            &state_label(&state, french),
                            state_color(&state, &palette),
                        );
                        if state == SessionState::Paused {
                            if ui
                                .add_sized(
                                    [inner, 30.0],
                                    egui::Button::new(
                                        self.language_pack.text("ui.sessions_page.019"),
                                    ),
                                )
                                .clicked()
                            {
                                self.session_action(id, SessionAction::Resume);
                            }
                        } else if ui
                            .add_sized(
                                [inner, 30.0],
                                egui::Button::new(if french { "Ouvrir" } else { "Open" }),
                            )
                            .clicked()
                        {
                            opening = Some(id);
                        }
                        if ui
                            .add_sized(
                                [inner, 30.0],
                                egui::Button::new(self.language_pack.text("ui.sessions_page.015")),
                            )
                            .clicked()
                        {
                            closing = Some(id);
                        }
                    });
                }
            });
        }

        // ---------------------------------------------------- Deferred actions
        if let Some(id) = opening {
            if self.controller.active_session() == Some(id) {
                self.active_page = DesktopPage::Game;
            } else {
                match self.release_game_inputs() {
                    // Do not switch away if held controls could not be released:
                    // the old session must not keep receiving stuck input.
                    Err(error) => self.message = error,
                    Ok(()) => match self.controller.select_session(id) {
                        Ok(()) => {
                            self.audio_output.clear();
                            self.frame_cadence.reset();
                            self.frame_texture = None;
                            self.video_size = None;
                            self.last_tick = Instant::now();
                            self.active_page = DesktopPage::Game;
                        }
                        Err(error) => self.message = error.to_string(),
                    },
                }
            }
        }

        if let Some(id) = closing {
            if let Err(error) = self.save_dirty_game_data_for_session(id) {
                self.message = if french {
                    format!("Fermeture annulée ; sauvegarde en jeu impossible : {error}")
                } else {
                    format!("Close cancelled; game save could not be flushed: {error}")
                };
            } else {
                let was_active = self.controller.active_session() == Some(id);
                let released = if was_active {
                    self.release_game_inputs()
                } else {
                    // A non-active session can still retain controls from an earlier
                    // selection. Release that session before removing it as well.
                    self.controller
                        .release_inputs(id)
                        .map_err(|error| error.to_string())
                };
                match released {
                    Err(error) => {
                        self.message = if french {
                            format!(
                                "Fermeture annulée ; impossible de libérer les commandes : {error}"
                            )
                        } else {
                            format!("Close cancelled; could not release held inputs: {error}")
                        };
                    }
                    Ok(()) => match self.controller.close_session(id) {
                        Ok(()) => {
                            self.message = if french {
                                format!("Session {} fermée.", id.get())
                            } else {
                                format!("Closed session {}.", id.get())
                            };
                            if was_active {
                                self.audio_output.clear();
                                self.frame_cadence.reset();
                                self.frame_texture = None;
                                self.video_size = None;
                                self.last_tick = Instant::now();
                                self.media_status = if french {
                                    "Session active fermée.".to_owned()
                                } else {
                                    "Active session closed.".to_owned()
                                };
                            }
                            self.last_tick_errors
                                .retain(|(error_id, _)| *error_id != id);
                            self.session_content_paths
                                .retain(|(session_id, _)| *session_id != id);
                        }
                        Err(error) => self.message = error.to_string(),
                    },
                }
            }
        }

        if close_all {
            let save_errors = self.flush_dirty_game_saves();
            if !save_errors.is_empty() {
                self.message = if french {
                    format!(
                        "Fermeture annulée ; sauvegardes en jeu impossibles : {}",
                        save_errors.join("; ")
                    )
                } else {
                    format!(
                        "Close all cancelled; game saves could not be flushed: {}",
                        save_errors.join("; ")
                    )
                };
            } else if let Err(error) = self.release_game_inputs() {
                // Keep every session open if held controls cannot be released.
                self.message = if french {
                    format!("Fermeture annulée ; impossible de libérer les commandes : {error}")
                } else {
                    format!("Close all cancelled; could not release held inputs: {error}")
                };
            } else {
                let count = self.controller.close_all_sessions();
                self.audio_output.clear();
                self.frame_cadence.reset();
                self.frame_texture = None;
                self.video_size = None;
                self.last_tick = Instant::now();
                self.media_status = if french {
                    "Aucune session active.".to_owned()
                } else {
                    "No active session.".to_owned()
                };
                self.last_tick_errors.clear();
                self.session_content_paths.clear();
                self.message = if french {
                    let noun = if count == 1 {
                        "session fermée"
                    } else {
                        "sessions fermées"
                    };
                    format!("{count} {noun}.")
                } else {
                    let noun = if count == 1 { "session" } else { "sessions" };
                    format!("Closed {count} {noun}.")
                };
            }
        }
    }
}

#[cfg(test)]
mod state_label_tests {
    use super::*;

    #[test]
    fn labels_every_session_state_in_french_and_english() {
        let cases = [
            (SessionState::Empty, "Vide", "Empty"),
            (SessionState::Running, "En cours", "Running"),
            (SessionState::Paused, "En pause", "Paused"),
            (
                SessionState::Halted,
                "Arrêtée par le cœur",
                "Halted by core",
            ),
            (SessionState::Stopped, "Arrêtée", "Stopped"),
        ];
        for (state, french, english) in cases {
            assert_eq!(state_label(&state, true), french);
            assert_eq!(state_label(&state, false), english);
        }
    }
}
