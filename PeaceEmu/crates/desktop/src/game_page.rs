//! game page for the PeaceEmu desktop frontend.

use super::*;

impl DesktopApp {
    pub(super) fn show_game(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.heading(self.language_pack.text("ui.game_page.001"));
                ui.label(self.language_pack.text("ui.game_page.002"));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(self.language_pack.text("ui.game_page.003")).clicked() {
                    self.navigate_to(DesktopPage::Sessions);
                }
                if ui.button(self.language_pack.text("ui.game_page.004")).clicked() {
                    self.navigate_to(DesktopPage::Library);
                }
            });
        });
        ui.add_space(8.0);

        let active = self.controller.active_session();
        let session_summary = active.and_then(|id| self.controller.session_summary(id).ok());
        if let Some(summary) = session_summary {
            let id = summary.id;
            let state = summary.state;
            let content_loaded = summary.content_loaded;
            let save_states = summary.save_states;
            let persistent_saves = summary.persistent_saves;
            let system = summary.system_name;
            egui::Frame::group(ui.style())
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(egui::Margin::same(14.0))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.vertical(|ui| {
                            ui.heading(format!("{system} · Session {}", id.get()));
                            ui.label(if self.language == "fr" {
                                format!("État : {state:?}")
                            } else {
                                format!("State: {state:?}")
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                    });
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(if summary.video_available {
                            self.language_pack.text("ui.game_page.012")
                        } else {
                            self.language_pack.text("ui.game_page.013")
                        });
                        ui.separator();
                        ui.label(if summary.audio_available {
                            self.language_pack.text("ui.game_page.014")
                        } else {
                            self.language_pack.text("ui.game_page.015")
                        });
                        ui.separator();
                        ui.label(if summary.persistent_saves {
                            self.language_pack.text("ui.game_page.016")
                        } else { self.language_pack.text("ui.game_page.017") });
                        ui.separator();
                        ui.label(self.language_pack.text("ui.game_page.018"));
                    });
                });
        } else {
            egui::Frame::group(ui.style())
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(egui::Margin::same(14.0))
                .show(ui, |ui| {
                ui.heading(self.language_pack.text("ui.game_page.019"));
                ui.label(self.language_pack.text("ui.game_page.020"));
                if ui.button(self.language_pack.text("ui.game_page.021")).clicked() {
                    self.navigate_to(DesktopPage::Library);
                }
            });
        }

        ui.add_space(8.0);
        let available = ui.available_size();
        let display_height = (available.y - 100.0).max(180.0);
        egui::Frame::group(ui.style())
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(egui::Margin::same(14.0))
                .show(ui, |ui| {
            ui.set_min_height(display_height);
            ui.set_min_width(ui.available_width());
            ui.vertical_centered(|ui| {
                if let (Some(texture), Some([width, height])) = (&self.frame_texture, self.video_size) {
                    let available = ui.available_size();
                    let max_width = available.x.max(1.0);
                    let max_height = available.y.max(1.0);
                    let scale = (max_width / width as f32).min(max_height / height as f32);
                    let size = egui::vec2(width as f32 * scale, height as f32 * scale);
                    ui.image((texture.id(), size));
                    ui.small(format!("{width} x {height} - nearest-neighbour scaling"));
                } else {
                    ui.add_space(14.0);
                    ui.columns(2, |columns| {
                        columns[0].vertical_centered(|ui| {
                            ui.add_space(10.0);
                            let width = ui.available_width().min(270.0);
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(width, 208.0),
                                egui::Sense::hover(),
                            );
                            let accent = self.accent_color;
                            let shell = egui::Rect::from_min_size(
                                rect.min + egui::vec2(width * 0.12, 4.0),
                                egui::vec2(width * 0.76, 194.0),
                            );
                            ui.painter().rect_filled(
                                shell,
                                egui::Rounding::same(24.0),
                                if self.dark_mode {
                                    egui::Color32::from_rgb(56, 65, 86)
                                } else {
                                    egui::Color32::from_rgb(207, 216, 237)
                                },
                            );
                            let screen = egui::Rect::from_min_size(
                                shell.min + egui::vec2(18.0, 18.0),
                                egui::vec2(shell.width() - 36.0, 108.0),
                            );
                            ui.painter().rect_filled(
                                screen,
                                egui::Rounding::same(9.0),
                                if self.dark_mode {
                                    egui::Color32::from_rgb(15, 21, 33)
                                } else {
                                    egui::Color32::from_rgb(237, 244, 255)
                                },
                            );
                            ui.painter().text(
                                screen.center(),
                                egui::Align2::CENTER_CENTER,
                                "ARCHIA",
                                egui::FontId::proportional(18.0),
                                accent,
                            );
                            ui.painter().circle_filled(
                                egui::pos2(shell.min.x + 32.0, shell.min.y + 158.0),
                                8.0,
                                accent,
                            );
                            ui.painter().circle_filled(
                                egui::pos2(shell.min.x + 52.0, shell.min.y + 158.0),
                                8.0,
                                accent.gamma_multiply(0.65),
                            );
                            ui.painter().circle_filled(
                                egui::pos2(shell.max.x - 38.0, shell.min.y + 156.0),
                                10.0,
                                egui::Color32::from_rgb(218, 91, 113),
                            );
                            ui.painter().circle_filled(
                                egui::pos2(shell.max.x - 62.0, shell.min.y + 170.0),
                                10.0,
                                egui::Color32::from_rgb(83, 171, 137),
                            );
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new(self.language_pack.text("ui.game_page.022")).size(10.0).strong().color(ui.visuals().weak_text_color()));
                        });
                        columns[1].vertical(|ui| {
                            ui.add_space(14.0);
                            ui.label(egui::RichText::new(self.language_pack.text("ui.game_page.023")).size(10.0).strong().color(self.accent_color));
                            ui.add_space(6.0);
                            ui.heading(self.language_pack.text("ui.game_page.024"));
                            ui.add_space(6.0);
                            ui.label(self.language_pack.text("ui.game_page.025"));
                            ui.add_space(14.0);
                            egui::Frame::group(ui.style())
                                .rounding(egui::Rounding::same(9.0))
                                .inner_margin(egui::Margin::same(12.0))
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new(self.language_pack.text("ui.game_page.026")).size(10.0).strong());
                                    ui.add_space(8.0);
                                    ui.horizontal_wrapped(|ui| {
                                        for (key, action) in [
                                            (self.language_pack.text("ui.game_page.027"), self.language_pack.text("ui.game_page.028")),
                                            ("Z / X", "Action"),
                                            ("Entrée", "Start"),
                                            ("Retour", "Select"),
                                        ] {
                                            ui.label(egui::RichText::new(key).strong());
                                            ui.label(action);
                                            ui.separator();
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
            ui.label(self.language_pack.text("ui.game_page.029"));
            ui.label(&self.media_status);
        });
        ui.small(self.language_pack.text("ui.game_page.030"));
    }
}
