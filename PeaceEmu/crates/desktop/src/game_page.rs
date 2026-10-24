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

        ui.add_space(4.0);
        let display_height = (ui.available_height() - 100.0).max(180.0);
        theme::card(ui, &palette, |ui| {
            ui.set_min_height(display_height);
            ui.set_min_width(ui.available_width());
            ui.vertical_centered(|ui| {
                if self.game_window_open {
                    ui.label(if french {
                        "Le jeu est affiché dans sa fenêtre dédiée."
                    } else {
                        "Gameplay is displayed in its dedicated window."
                    });
                } else if let (Some(texture), Some([width, height])) =
                    (&self.frame_texture, self.video_size)
                {
                    let available = ui.available_size();
                    let scale = (available.x.max(1.0) / width as f32)
                        .min(available.y.max(1.0) / height as f32);
                    let size = egui::vec2(width as f32 * scale, height as f32 * scale);
                    ui.image((texture.id(), size));
                    ui.label(
                        egui::RichText::new(format!(
                            "{width} x {height}  ·  {}",
                            if french {
                                "mise à l’échelle au plus proche voisin"
                            } else {
                                "nearest-neighbour scaling"
                            }
                        ))
                        .size(11.0)
                        .color(palette.text_dim),
                    );
                } else {
                    ui.add_space(14.0);
                    ui.columns(2, |columns| {
                        columns[0].vertical_centered(|ui| {
                            let width = ui.available_width().min(260.0);
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(width, 170.0),
                                egui::Sense::hover(),
                            );
                            theme::paint_console(ui.painter(), rect, &palette, "PEACEEMU");
                            ui.label(
                                egui::RichText::new(self.language_pack.text("ui.game_page.022"))
                                    .size(11.0)
                                    .strong()
                                    .color(palette.text_dim),
                            );
                        });
                        columns[1].vertical(|ui| {
                            ui.label(
                                egui::RichText::new(self.language_pack.text("ui.game_page.023"))
                                    .size(11.0)
                                    .strong()
                                    .color(palette.accent_text),
                            );
                            ui.label(
                                egui::RichText::new(self.language_pack.text("ui.game_page.024"))
                                    .size(22.0)
                                    .strong(),
                            );
                            ui.label(self.language_pack.text("ui.game_page.025"));
                            ui.add_space(8.0);
                            egui::Frame::none()
                                .fill(palette.panel)
                                .stroke(egui::Stroke::new(1.0_f32, palette.border))
                                .rounding(egui::Rounding::same(10.0))
                                .inner_margin(egui::Margin::same(12.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(
                                            self.language_pack.text("ui.game_page.026"),
                                        )
                                        .size(11.0)
                                        .strong()
                                        .color(palette.text_dim),
                                    );
                                    ui.horizontal_wrapped(|ui| {
                                        for (key, action) in [
                                            (
                                                self.language_pack.text("ui.game_page.027"),
                                                self.language_pack.text("ui.game_page.028"),
                                            ),
                                            ("Z / X", "Action"),
                                            (
                                                if french { "Entrée" } else { "Enter" },
                                                if french { "Démarrer" } else { "Start" },
                                            ),
                                            (
                                                if french { "Retour" } else { "Backspace" },
                                                if french { "Sélection" } else { "Select" },
                                            ),
                                        ] {
                                            theme::badge(ui, key, palette.text_dim);
                                            ui.label(action);
                                            ui.add_space(6.0);
                                        }
                                    });
                                });
                        });
                    });
                    ui.add_space(14.0);
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(self.language_pack.text("ui.game_page.029"))
                    .size(13.0)
                    .color(palette.text_dim),
            );
            ui.label(egui::RichText::new(&self.media_status).size(13.0));
        });
        ui.label(
            egui::RichText::new(self.language_pack.text("ui.game_page.030"))
                .size(12.0)
                .color(palette.text_dim),
        );
    }
}
