//! sessions page for the PeaceEmu desktop frontend.

use super::*;

impl DesktopApp {
    pub(super) fn show_sessions(&mut self, ui: &mut egui::Ui) {
        let french = self.language == "fr";
        let session_count = self.controller.session_count();
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.sessions_page.001"))
                        .size(10.0)
                        .strong()
                        .color(self.accent_color),
                );
                ui.add_space(3.0);
                ui.heading(self.language_pack.text("ui.sessions_page.002"));
                ui.label(self.language_pack.text("ui.sessions_page.003"));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::Frame::group(ui.style())
                    .rounding(egui::Rounding::same(9.0))
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let session_label = if french {
                                if session_count == 1 {
                                    "session"
                                } else {
                                    "sessions"
                                }
                            } else if session_count == 1 {
                                "session"
                            } else {
                                "sessions"
                            };
                            ui.label(format!("{session_count} {session_label}"));
                        });
                    });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        session_count > 0,
                        egui::Button::new(self.language_pack.text("ui.sessions_page.004")),
                    )
                    .clicked()
                {
                    let save_errors = self.flush_dirty_game_saves();
                    if !save_errors.is_empty() {
                        self.message = format!(
                            "Close all cancelled; game saves could not be flushed: {}",
                            save_errors.join("; ")
                        );
                    } else {
                        // If held inputs cannot be released, keep every session open so the
                        // lifecycle transition cannot leave a game with stuck controls.
                        if let Err(error) = self.release_game_inputs() {
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
            });
        });
        ui.add_space(8.0);
        let sessions = self.controller.session_summaries();
        if sessions.is_empty() {
            egui::Frame::group(ui.style())
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(egui::Margin::same(14.0))
                .show(ui, |ui| {
                ui.heading(self.language_pack.text("ui.sessions_page.005"));
                ui.label(self.language_pack.text("ui.sessions_page.006"));
                if ui.button(self.language_pack.text("ui.sessions_page.007")).clicked() {
                    self.navigate_to(DesktopPage::Library);
                }
            });
        }
        for summary in sessions {
            let id = summary.id;
            let state = summary.state;
            let content_loaded = summary.content_loaded;
            let save_states = summary.save_states;
            let persistent_saves = summary.persistent_saves;
            let system_name = summary.system_name;
            let video_available = summary.video_available;
            let audio_available = summary.audio_available;
            let active_error = summary.last_error.clone();
            let core_diagnostic = summary.last_error.clone();
            let tick_error = self.last_tick_errors.iter().find_map(|(error_id, error)| {
                if *error_id == id {
                    Some(error.clone())
                } else {
                    None
                }
            });
            egui::Frame::group(ui.style())
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(egui::Margin::same(14.0))
                .show(ui, |ui| {
                if state == SessionState::Paused {
                    ui.set_min_width(170.0);
                    ui.set_max_width(220.0);
                    ui.vertical(|ui| {
                        ui.strong(format!("Session {}", id.get()));
                        ui.label(&system_name);
                        ui.small(self.language_pack.text("ui.sessions_page.008"));
                        if ui.button(self.language_pack.text("ui.sessions_page.009")).clicked() {
                            self.session_action(id, SessionAction::Resume);
                        }
                        if content_loaded && save_states {
                            if ui.button(self.language_pack.text("ui.sessions_page.010")).clicked() {
                                self.save_state_action(id);
                            }
                            if ui.button(self.language_pack.text("ui.sessions_page.011")).clicked() {
                                self.load_state_action(id);
                            }
                            if ui.button(self.language_pack.text("ui.sessions_page.012")).clicked() {
                                self.session_action(id, SessionAction::StopAndSave);
                            }
                        }
                        if content_loaded && persistent_saves && ui.button(self.language_pack.text("ui.sessions_page.013")).clicked() {
                            self.save_game_data_action(id);
                        }
                        if ui.button(self.language_pack.text("ui.sessions_page.014")).clicked() {
                            if let Err(error) = self.save_dirty_game_data_for_session(id) {
                                self.message = if french {
                                    format!("Fermeture annulée ; sauvegarde en jeu impossible : {error}")
                                } else {
                                    format!("Close cancelled; game save could not be flushed: {error}")
                                };
                                return;
                            }
                            let was_active = self.controller.active_session() == Some(id);
                            if was_active {
                                if let Err(error) = self.release_game_inputs() {
                                    self.message = if french {
                                        format!("Fermeture annulée ; impossible de libérer les commandes : {error}")
                                    } else {
                                        format!("Close cancelled; could not release held inputs: {error}")
                                    };
                                    return;
                                }
                            }
                            match self.controller.close_session(id) {
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
                                    }
                                    self.last_tick_errors.retain(|(error_id, _)| *error_id != id);
                                    self.session_content_paths.retain(|(session_id, _)| *session_id != id);
                                }
                                Err(error) => self.message = error.to_string(),
                            }
                        }
                    });
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    let selected = self.controller.active_session() == Some(id);
                    if ui
                        .selectable_label(selected, format!("Session {}", id.get()))
                        .clicked()
                        && self.controller.active_session() != Some(id)
                    {
                        // Do not switch away if held controls could not be released:
                        // the old session must not keep receiving stuck input.
                        if let Err(error) = self.release_game_inputs() {
                            self.message = error;
                            return;
                        }
                        match self.controller.select_session(id) {
                            Ok(()) => {
                                self.audio_output.clear();
                                self.frame_cadence.reset();
                                self.frame_texture = None;
                                self.video_size = None;
                                self.last_tick = Instant::now();
                                self.active_page = DesktopPage::Game;
                            }
                            Err(error) => self.message = error.to_string(),
                        }
                    }
                    ui.label(&system_name);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(self.language_pack.text("ui.sessions_page.015")).clicked() {
                            if let Err(error) = self.save_dirty_game_data_for_session(id) {
                                self.message = if french {
                                    format!("Fermeture annulée ; sauvegarde en jeu impossible : {error}")
                                } else {
                                    format!("Close cancelled; game save could not be flushed: {error}")
                                };
                                return;
                            }
                            let was_active = self.controller.active_session() == Some(id);
                            if was_active {
                                if let Err(error) = self.release_game_inputs() {
                                    self.message = if french {
                                        format!("Fermeture annulée ; impossible de libérer les commandes : {error}")
                                    } else {
                                        format!("Close cancelled; could not release held inputs: {error}")
                                    };
                                    return;
                                }
                            }
                            match self.controller.close_session(id) {
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
                            }
                        }
                    });
                });
                if let Some((_, content_path)) = self
                    .session_content_paths
                    .iter()
                    .find(|(session_id, _)| *session_id == id)
                {
                    ui.label(if french { format!("Source : {}", content_path.display()) } else { format!("Source: {}", content_path.display()) });
                } else if content_loaded {
                    ui.label(self.language_pack.text("ui.sessions_page.016"));
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{}: {state:?}",
                        self.language_pack.text("ui.sessions_page.017")
                    ));
                    ui.separator();
                    ui.label(if french {
                        if content_loaded {
                            "Contenu : chargé"
                        } else {
                            "Contenu : non chargé"
                        }
                    } else if content_loaded {
                        "Content: loaded"
                    } else {
                        "Content: not loaded"
                    });
                    if state == SessionState::Running && ui.button(self.language_pack.text("ui.sessions_page.018")).clicked() {
                        self.session_action(id, SessionAction::Pause);
                    }
                    if state == SessionState::Paused && ui.button(self.language_pack.text("ui.sessions_page.019")).clicked() {
                        self.session_action(id, SessionAction::Resume);
                    }
                    if content_loaded && save_states {
                        if ui.button(self.language_pack.text("ui.sessions_page.020")).clicked() {
                            self.save_state_action(id);
                        }
                        if ui.button(self.language_pack.text("ui.sessions_page.021")).clicked() {
                            self.load_state_action(id);
                        }
                        if ui.button(self.language_pack.text("ui.sessions_page.022")).clicked() {
                            self.session_action(id, SessionAction::StopAndSave);
                        }
                    }
                    if content_loaded && persistent_saves && ui.button(self.language_pack.text("ui.sessions_page.023")).clicked() {
                        self.save_game_data_action(id);
                    }
                    if state != SessionState::Empty && ui.button(self.language_pack.text("ui.sessions_page.024")).clicked() {
                        self.session_action(id, SessionAction::Reset);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(if french {
                        if video_available {
                            "Vidéo : disponible"
                        } else {
                            "Vidéo : indisponible"
                        }
                    } else if video_available {
                        "Video: available"
                    } else {
                        "Video: unavailable"
                    });
                    ui.separator();
                    ui.label(if french {
                        if audio_available {
                            "Sortie audio du cœur : déclarée"
                        } else {
                            "Sortie audio du cœur : indisponible"
                        }
                    } else if audio_available {
                        "Audio core output: declared"
                    } else {
                        "Audio core output: unavailable"
                    });
                });
                if let Some(error) = &tick_error {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        format!(
                            "{}: {error}",
                            self.language_pack.text("ui.sessions_page.025")
                        ),
                    );
                }
                if let Some(error) = active_error {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        format!(
                            "{}: {error}",
                            self.language_pack.text("ui.sessions_page.026")
                        ),
                    );
                }
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
                            "{}: {}",
                            self.language_pack.text("ui.sessions_page.029"),
                            self.controller.active_session() == Some(id)
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
                                egui::Color32::LIGHT_RED,
                                if french { format!("Dernière erreur d’exécution/média : {error}") } else { format!("Most recent tick/media failure: {error}") },
                            );
                        } else {
                            ui.label(self.language_pack.text("ui.sessions_page.036"));
                        }
                        if let Some(error) = &core_diagnostic {
                            ui.colored_label(
                                egui::Color32::LIGHT_RED,
                                format!("{}: {error}", self.language_pack.text("ui.sessions_page.037")),
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
                        let core_diagnostic_text = core_diagnostic
                            .as_deref()
                            .unwrap_or("none recorded");
                        let report = format!(
                            "PeaceEmu session diagnostics\nSession ID: {}\nActive: {}\nState: {:?}\nSystem: {}\nContent loaded: {}\nSource path: {}\nVideo capability: {}\nAudio capability: {}\nLast tick/media error: {}\nLast core error: {}",
                            id.get(),
                            self.controller.active_session() == Some(id),
                            state,
                            system_name,
                            content_loaded,
                            source_path,
                            if video_available { "available" } else { "unavailable" },
                            if audio_available { "available" } else { "unavailable" },
                            tick_diagnostic,
                            core_diagnostic_text,
                        );
                        if ui.button(self.language_pack.text("ui.sessions_page.040")).clicked() {
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
    }
}
