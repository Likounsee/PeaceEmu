//! Game page for the PeaceEmu desktop frontend.

use super::*;

impl DesktopApp {
    pub(super) fn show_game(&mut self, ui: &mut egui::Ui) {
        let palette = theme::Palette::new(self.dark_mode, self.accent_color);
        let french = self.language == "fr";
        let mut go: Option<DesktopPage> = None;

        theme::page_header(
            ui,
            &palette,
            self.language_pack.text("ui.game_page.001"),
            self.language_pack.text("ui.game_page.002"),
            |ui| {
                if ui
                    .button(self.language_pack.text("ui.game_page.003"))
                    .clicked()
                {
                    go = Some(DesktopPage::Sessions);
                }
                if ui
                    .button(self.language_pack.text("ui.game_page.004"))
                    .clicked()
                {
                    go = Some(DesktopPage::Library);
                }
            },
        );
        if let Some(page) = go {
            self.navigate_to(page);
        }

        let active = self.controller.active_session();
        let session_summary = active.and_then(|id| self.controller.session_summary(id).ok());
        if let Some(summary) = session_summary {
            let id = summary.id;
            let state = summary.state;
            let content_loaded = summary.content_loaded;
            let save_states = summary.save_states;
            let persistent_saves = summary.persistent_saves;
            let video_available = summary.video_available;
            let audio_available = summary.audio_available;
            let system = summary.system_name;
            theme::card(ui, &palette, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{system} · Session {}", id.get()))
                            .size(18.0)
                            .strong(),
                    );
                    theme::badge(
                        ui,
                        &crate::sessions_page::state_label(&state, french),
                        crate::sessions_page::state_color(&state, &palette),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    if state == SessionState::Running
                        && ui
                            .button(self.language_pack.text("ui.game_page.005"))
                            .clicked()
                    {
                        self.session_action(id, SessionAction::Pause);
                    } else if state == SessionState::Paused
                        && ui
                            .button(self.language_pack.text("ui.game_page.006"))
                            .clicked()
                    {
                        self.session_action(id, SessionAction::Resume);
                    }
                    if content_loaded && save_states {
                        if ui
                            .button(self.language_pack.text("ui.game_page.007"))
                            .clicked()
                        {
                            self.save_state_action(id);
                        }
                        if ui
                            .button(self.language_pack.text("ui.game_page.008"))
                            .clicked()
                        {
                            self.load_state_action(id);
                        }
                        if ui
                            .button(self.language_pack.text("ui.game_page.009"))
                            .clicked()
                        {
                            self.session_action(id, SessionAction::StopAndSave);
                        }
                    }
                    if content_loaded
                        && persistent_saves
                        && ui
                            .button(self.language_pack.text("ui.game_page.010"))
                            .clicked()
                    {
                        self.save_game_data_action(id);
                    }
                    if state != SessionState::Empty
                        && ui
                            .button(self.language_pack.text("ui.game_page.011"))
                            .clicked()
                    {
                        self.session_action(id, SessionAction::Reset);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for (on, yes, no) in [
                        (video_available, "ui.game_page.012", "ui.game_page.013"),
                        (audio_available, "ui.game_page.014", "ui.game_page.015"),
                        (persistent_saves, "ui.game_page.016", "ui.game_page.017"),
                    ] {
                        theme::badge(
                            ui,
                            self.language_pack.text(if on { yes } else { no }),
                            if on {
                                palette.success
                            } else {
                                palette.text_dim
                            },
                        );
                    }
                    ui.label(
                        egui::RichText::new(self.language_pack.text("ui.game_page.018"))
                            .size(13.0)
                            .color(palette.text_dim),
                    );
                });
            });
        } else {
            theme::card(ui, &palette, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.game_page.019"))
                        .size(18.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.game_page.020"))
                        .color(palette.text_dim),
                );
                if ui
                    .add(theme::primary_button(
                        &palette,
                        self.language_pack.text("ui.game_page.021"),
                    ))
                    .clicked()
                {
                    self.navigate_to(DesktopPage::Library);
                }
            });
        }

        ui.add_space(8.0);
        theme::card(ui, &palette, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(if french {
                    "Fenêtre de jeu séparée"
                } else {
                    "Separate game window"
                }).size(18.0).strong());
                theme::badge(
                    ui,
                    if self.game_viewport_open {
                        if french { "Jeu ouvert" } else { "Game open" }
                    } else {
                        if french { "Fenêtre fermée" } else { "Window closed" }
                    },
                    if self.game_viewport_open { palette.success } else { palette.text_dim },
                );
            });
            ui.label(if french {
                "L’image du jeu est affichée dans sa propre fenêtre, sans interface ajoutée par l’émulateur. Utilisez cette page pour gérer la session ; relancez le jeu depuis la bibliothèque si vous avez fermé sa fenêtre."
            } else {
                "The game image is shown in its own window without emulator-added UI. Use this page to manage the session; launch the game again from the library if you closed its window."
            });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(&self.media_status)
                    .size(13.0)
                    .color(palette.text_dim),
            );
        });
    }
}
