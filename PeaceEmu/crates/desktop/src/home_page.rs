//! Polished overview dashboard for the PeaceEmu desktop frontend.

use super::*;

impl DesktopApp {
    pub(super) fn show_home(&mut self, ui: &mut egui::Ui) {
        let french = self.language == "fr";
        let accent = self.accent_color;
        let panel = ui.visuals().faint_bg_color;
        let border = ui.visuals().widgets.noninteractive.bg_stroke;
        let text = ui.visuals().text_color();
        let muted = ui.visuals().weak_text_color();

        // Hero
        egui::Frame::none()
            .fill(if self.dark_mode {
                egui::Color32::from_rgb(24, 31, 49)
            } else {
                egui::Color32::from_rgb(235, 240, 252)
            })
            .rounding(egui::Rounding::same(16.0))
            .inner_margin(egui::Margin::same(24.0))
            .stroke(egui::Stroke::new(1.0_f32, accent.gamma_multiply(0.45)))
            .show(ui, |ui| {
                ui.set_min_height(172.0);
                ui.columns(2, |columns| {
                    columns[0].vertical(|ui| {
                        ui.label(
                            egui::RichText::new(self.language_pack.text("ui.home_page.001"))
                            .size(11.0)
                            .strong()
                            .color(accent),
                        );
                        ui.add_space(8.0);
                        ui.heading(
                            egui::RichText::new(self.language_pack.text("ui.home_page.002"))
                            .size(28.0)
                            .strong(),
                        );
                        ui.add_space(4.0);
                        ui.label(self.language_pack.text("ui.home_page.003"));
                        ui.add_space(16.0);
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(self.language_pack.text("ui.home_page.004"))
                                        .strong(),
                                    )
                                    .fill(accent)
                                    .min_size(egui::vec2(150.0, 38.0)),
                                )
                                .clicked()
                            {
                                self.choose_content_file();
                            }
                            if ui
                                .add(
                                    egui::Button::new(self.language_pack.text("ui.home_page.005"))
                                    .min_size(egui::vec2(150.0, 38.0)),
                                )
                                .clicked()
                            {
                                self.navigate_to(DesktopPage::Library);
                            }
                        });
                    });
                    columns[1].vertical(|ui| {
                        ui.add_space(5.0);
                        egui::Frame::none()
                            .fill(if self.dark_mode {
                                egui::Color32::from_rgb(31, 39, 60)
                            } else {
                                egui::Color32::from_rgb(224, 232, 250)
                            })
                            .rounding(egui::Rounding::same(13.0))
                            .inner_margin(egui::Margin::same(14.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(self.language_pack.text("ui.home_page.006"))
                                            .size(10.0)
                                            .strong()
                                            .color(accent),
                                        );
                                        ui.label(
                                            egui::RichText::new(self.language_pack.text("ui.home_page.007"))
                                            .size(15.0)
                                            .strong(),
                                        );
                                    });
                                });
                                ui.add_space(12.0);
                                let console_width = ui.available_width().min(220.0);
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(console_width, 66.0),
                                    egui::Sense::hover(),
                                );
                                let body = egui::Rect::from_min_size(
                                    rect.min + egui::vec2(console_width * 0.17, 2.0),
                                    egui::vec2(console_width * 0.66, 62.0),
                                );
                                ui.painter().rect_filled(
                                    body,
                                    egui::Rounding::same(12.0),
                                    if self.dark_mode {
                                        egui::Color32::from_rgb(66, 79, 108)
                                    } else {
                                        egui::Color32::from_rgb(197, 208, 235)
                                    },
                                );
                                let screen = egui::Rect::from_min_size(
                                    body.min + egui::vec2(10.0, 9.0),
                                    egui::vec2(body.width() - 20.0, 29.0),
                                );
                                ui.painter().rect_filled(
                                    screen,
                                    egui::Rounding::same(5.0),
                                    if self.dark_mode {
                                        egui::Color32::from_rgb(17, 24, 38)
                                    } else {
                                        egui::Color32::from_rgb(240, 245, 255)
                                    },
                                );
                                ui.painter().circle_filled(
                                    egui::pos2(body.center().x - 13.0, body.max.y - 8.0),
                                    3.0,
                                    accent,
                                );
                                ui.painter().circle_filled(
                                    egui::pos2(body.center().x + 13.0, body.max.y - 8.0),
                                    3.0,
                                    accent,
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new(if self.controller.session_count() > 0 {
                                        "Session active"
                                    } else { self.language_pack.text("ui.home_page.008") })
                                    .size(11.0)
                                    .color(muted),
                                );
                            });
                    });
                });
            });

        ui.add_space(16.0);

        // At-a-glance metrics
        let metrics = [
            (
                self.language_pack.text("ui.home_page.009"),
                self.controller.library().len().to_string(),
                self.language_pack.text("ui.home_page.010"),
            ),
            (
                "SESSIONS",
                self.controller.session_count().to_string(),
                self.language_pack.text("ui.home_page.011"),
            ),
            (
                self.language_pack.text("ui.home_page.012"),
                self.registry.system_ids().count().to_string(),
                self.language_pack.text("ui.home_page.013"),
            ),
        ];
        let columns = if ui.available_width() < 620.0 { 1 } else { 3 };
        ui.columns(columns, |cols| {
            for (index, (label, value, detail)) in metrics.into_iter().enumerate() {
                let card_ui = &mut cols[index % columns];
                egui::Frame::none()
                    .fill(panel)
                    .rounding(egui::Rounding::same(12.0))
                    .inner_margin(egui::Margin::same(16.0))
                    .stroke(border)
                    .show(card_ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(label).size(10.0).strong().color(muted));
                        });
                        ui.add_space(6.0);
                        ui.heading(egui::RichText::new(value).size(30.0).strong().color(text));
                        ui.label(egui::RichText::new(detail).size(12.0).color(muted));
                    });
            }
        });

        ui.add_space(18.0);
        ui.horizontal_wrapped(|ui| {
            ui.heading(self.language_pack.text("ui.home_page.014"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .link(self.language_pack.text("ui.home_page.015"))
                    .clicked()
                {
                    self.navigate_to(DesktopPage::Library);
                }
            });
        });
        ui.add_space(8.0);

        let entries: Vec<_> = self
            .controller
            .library()
            .entries()
            .map(|entry| {
                (
                    entry.title().to_owned(),
                    entry.path().to_path_buf(),
                    entry.size_bytes(),
                )
            })
            .collect();

        if entries.is_empty() {
            egui::Frame::none()
                .fill(panel)
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(egui::Margin::same(20.0))
                .stroke(border)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(self.language_pack.text("ui.home_page.016"))
                                .strong()
                                .size(16.0),
                            );
                            ui.label(self.language_pack.text("ui.home_page.017"));
                            ui.add_space(6.0);
                            if ui
                                .button(self.language_pack.text("ui.home_page.018"))
                                .clicked()
                            {
                                self.choose_content_file();
                            }
                        });
                    });
                });
        } else {
            egui::Frame::none()
                .fill(panel)
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(egui::Margin::same(8.0))
                .stroke(border)
                .show(ui, |ui| {
                    for (title, path, size) in entries.iter().take(5) {
                        ui.horizontal(|ui| {
                            egui::Frame::none()
                                .fill(accent.gamma_multiply(0.18))
                                .rounding(egui::Rounding::same(8.0))
                                .inner_margin(egui::Margin::same(10.0))
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new("GB").strong().color(accent));
                                });
                            ui.add_space(4.0);
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(title).strong());
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{}  -  {}",
                                        path.file_name()
                                            .and_then(|name| name.to_str())
                                            .unwrap_or("ROM"),
                                        format_size(*size)
                                    ))
                                    .size(12.0)
                                    .color(muted),
                                );
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .button(self.language_pack.text("ui.home_page.019"))
                                        .clicked()
                                    {
                                        self.content_path = path.display().to_string();
                                        self.selected_content = Some(path.clone());
                                        self.launch_content();
                                    }
                                },
                            );
                        });
                        ui.add_space(4.0);
                        ui.separator();
                    }
                });
        }

        ui.add_space(16.0);
        egui::Frame::none()
            .fill(panel)
            .rounding(egui::Rounding::same(12.0))
            .inner_margin(egui::Margin::same(16.0))
            .stroke(border)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(self.language_pack.text("ui.home_page.020"))
                            .size(10.0).strong().color(muted));
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(self.language_pack.text("ui.home_page.021")).strong().size(15.0));
                        ui.label(self.language_pack.text("ui.home_page.022"));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::Frame::none()
                            .fill(accent.gamma_multiply(0.16))
                            .rounding(egui::Rounding::same(20.0))
                            .inner_margin(egui::Margin { left: 10.0, right: 10.0, top: 6.0, bottom: 6.0 })
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new(self.language_pack.text("ui.home_page.023"))
                                    .size(10.0).strong().color(accent));
                            });
                    });
                });
            });
    }
}
