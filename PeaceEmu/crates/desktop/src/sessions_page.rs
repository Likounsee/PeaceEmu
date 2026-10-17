//! sessions page for the PeaceEmu desktop frontend.

use super::*;

impl DesktopApp {
    pub(super) fn show_sessions(&mut self, ui: &mut egui::Ui) {
        let french = self.language == "fr";
        let session_count = self.controller.session_count();
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(if french { "ACTIVITÉ" } else { "ACTIVITY" })
                        .size(10.0)
                        .strong()
                        .color(self.accent_color),
                );
                ui.add_space(3.0);
                ui.heading(if french {
                    "Gestionnaire de sessions"
                } else {
                    "Session manager"
                });
                ui.label(if french {
                    "Suivez l’état des sessions et contrôlez leur cycle de vie."
                } else {
                    "Inspect session state and control each session lifecycle."
                });
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
                        egui::Button::new(if self.language == "fr" {
                            "Tout fermer"
                        } else {
                            "Close all"
                        }),
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
                        // Keyboard input is routed only to the active session. Release it before
                        // closing sessions so a held control cannot survive the lifecycle transition.
                        let release_error = self.release_game_inputs().err();
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
                        self.message = match (french, release_error) {
                            (true, Some(error)) => {
                                let noun = if count == 1 {
                                    "session fermée"
                                } else {
                                    "sessions fermées"
                                };
                                format!("{error} ; {count} {noun}.")
                            }
                            (true, None) => {
                                let noun = if count == 1 {
                                    "session fermée"
                                } else {
                                    "sessions fermées"
                                };
                                format!("{count} {noun}.")
                            }
                            (false, Some(error)) => {
                                let noun = if count == 1 { "session" } else { "sessions" };
                                format!("{error}; closed {count} {noun}.")
                            }
                            (false, None) => {
                                let noun = if count == 1 { "session" } else { "sessions" };
                                format!("Closed {count} {noun}.")
                            }
                        };
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
                ui.heading(if self.language == "fr" { "Aucune session ouverte" } else { "No open sessions" });
                ui.label(if self.language == "fr" { "Ouvrez une session à partir d’un fichier importé pour consulter son état et ses capacités." } else { "Open a session from an imported local file to inspect lifecycle and capability reporting." });
                if ui.button(if self.language == "fr" { "Ouvrir la bibliothèque" } else { "Browse library" }).clicked() {
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
                        ui.small(if self.language == "fr" { "En pause" } else { "Paused" });
                        if ui.button(if self.language == "fr" { "Reprendre" } else { "Resume" }).clicked() {
                            self.session_action(id, SessionAction::Resume);
                        }
                        if content_loaded && save_states {
                            if ui.button(if french { "Sauvegarder l’état" } else { "Save" }).clicked() {
                                self.save_state_action(id);
                            }
                            if ui.button(if french { "Charger l’état" } else { "Load state" }).clicked() {
                                self.load_state_action(id);
                            }
                            if ui.button(if french { "Arrêter et sauvegarder" } else { "Stop and Save" }).clicked() {
                                self.session_action(id, SessionAction::StopAndSave);
                            }
                        }
                        if content_loaded && persistent_saves && ui.button(if french { "Sauvegarder la partie" } else { "Save game" }).clicked() {
                            self.save_game_data_action(id);
                        }
                        if ui.button(if self.language == "fr" { "Fermer" } else { "Close" }).clicked() {
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
                        if ui.button(if self.language == "fr" { "Fermer" } else { "Close" }).clicked() {
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
                    ui.label(if french {
                        "Source : chemin indisponible pour cette session"
                    } else {
                        "Source: path unavailable for this session"
                    });
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{}: {state:?}",
                        if french { "État" } else { "State" }
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
                    if state == SessionState::Running && ui.button(if self.language == "fr" { "Mettre en pause" } else { "Pause" }).clicked() {
                        self.session_action(id, SessionAction::Pause);
                    }
                    if state == SessionState::Paused && ui.button(if self.language == "fr" { "Reprendre" } else { "Resume" }).clicked() {
                        self.session_action(id, SessionAction::Resume);
                    }
                    if content_loaded && save_states {
                        if ui.button(if french { "Sauvegarder" } else { "Save" }).clicked() {
                            self.save_state_action(id);
                        }
                        if ui.button(if french { "Charger l’état" } else { "Load state" }).clicked() {
                            self.load_state_action(id);
                        }
                        if ui.button(if french { "Arrêter et sauvegarder" } else { "Stop and Save" }).clicked() {
                            self.session_action(id, SessionAction::StopAndSave);
                        }
                    }
                    if content_loaded && persistent_saves && ui.button(if french { "Sauvegarder la partie" } else { "Save game" }).clicked() {
                        self.save_game_data_action(id);
                    }
                    if state != SessionState::Empty && ui.button(if self.language == "fr" { "Réinitialiser" } else { "Reset" }).clicked() {
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
                            if french {
                                "Dernière erreur d’exécution/média"
                            } else {
                                "Last execution/media tick error"
                            }
                        ),
                    );
                }
                if let Some(error) = active_error {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        format!(
                            "{}: {error}",
                            if french { "Dernière erreur du cœur" } else { "Last core error" }
                        ),
                    );
                }
                egui::CollapsingHeader::new(if french {
                    "Diagnostics de session"
                } else {
                    "Session diagnostics"
                })
                    .id_source(("session_diagnostics", id))
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label(format!(
                            "{}: {}",
                            if french {
                                "Identifiant de session"
                            } else {
                                "Session identifier"
                            },
                            id.get()
                        ));
                        ui.label(format!(
                            "{}: {}",
                            if french {
                                "Sélectionnée par l’hôte"
                            } else {
                                "Selected by host"
                            },
                            self.controller.active_session() == Some(id)
                        ));
                        ui.label(format!(
                            "{}: {state:?}",
                            if french {
                                "État du cycle de vie"
                            } else {
                                "Lifecycle state"
                            }
                        ));
                        ui.label(format!(
                            "{}: {content_loaded}",
                            if french {
                                "Contenu chargé"
                            } else {
                                "Content loaded"
                            }
                        ));
                        ui.label(format!(
                            "{}: {}",
                            if french {
                                "Capacité de sortie vidéo"
                            } else {
                                "Video output capability"
                            },
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
                            if french {
                                "Capacité de sortie audio"
                            } else {
                                "Audio output capability"
                            },
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
                            ui.label(if self.language == "fr" { "La sortie vidéo est indisponible : ce cœur ne déclare pas cette capacité." } else { "Video output is unavailable because this core does not declare the capability." });
                        }
                        if !audio_available {
                            ui.label(if self.language == "fr" { "La sortie audio est indisponible : ce cœur ne déclare pas cette capacité." } else { "Audio output is unavailable because this core does not declare the capability." });
                        }
                        if let Some(error) = &tick_error {
                            ui.colored_label(
                                egui::Color32::LIGHT_RED,
                                if french { format!("Dernière erreur d’exécution/média : {error}") } else { format!("Most recent tick/media failure: {error}") },
                            );
                        } else {
                            ui.label(if self.language == "fr" { "Dernière erreur d’exécution/média : aucune" } else { "Most recent tick/media failure: none recorded" });
                        }
                        if let Some(error) = &core_diagnostic {
                            ui.colored_label(
                                egui::Color32::LIGHT_RED,
                                format!("{}: {error}", if french {
                                "Diagnostic du cœur"
                            } else {
                                "Core diagnostic"
                            }),
                            );
                        } else {
                            ui.label(if self.language == "fr" { "Diagnostic du cœur : aucun" } else { "Core diagnostic: none recorded" });
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
                            .unwrap_or(if french {
                                "aucune enregistrée"
                            } else {
                                "none recorded"
                            });
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
                        if ui.button(if self.language == "fr" { "Copier le diagnostic de session" } else { "Copy session diagnostics" }).clicked() {
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
