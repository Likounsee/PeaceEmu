//! Overview dashboard for the PeaceEmu desktop frontend.

use super::*;

impl DesktopApp {
    pub(super) fn show_home(&mut self, ui: &mut egui::Ui) {
        let palette = theme::Palette::new(self.dark_mode, self.accent_color);
        let french = self.language == "fr";
        let mut import_requested = false;

        theme::page_header(
            ui,
            &palette,
            DesktopPage::Home.title(&self.language_pack),
            if french {
                "Vue d’ensemble de votre collection et de vos sessions."
            } else {
                "Overview of your collection and sessions."
            },
            |ui| {
                if ui
                    .add(theme::primary_button(
                        &palette,
                        self.language_pack.text("ui.home_page.004"),
                    ))
                    .clicked()
                {
                    import_requested = true;
                }
            },
        );
        if import_requested {
            self.choose_content_file();
        }

        // At-a-glance metrics
        let mut available_systems: std::collections::BTreeSet<String> = self
            .registry
            .system_ids()
            .map(|id| id.as_str().to_owned())
            .collect();
        available_systems.extend(nes_module_manager::available_module_ids());
        let metrics = [
            (
                self.language_pack.text("ui.home_page.009"),
                self.controller.library().len().to_string(),
                self.language_pack.text("ui.home_page.010"),
            ),
            (
                if french { "SESSIONS" } else { "SESSIONS" },
                self.controller.session_count().to_string(),
                self.language_pack.text("ui.home_page.011"),
            ),
            (
                self.language_pack.text("ui.home_page.012"),
                available_systems.len().to_string(),
                self.language_pack.text("ui.home_page.013"),
            ),
        ];
        let columns = if ui.available_width() < 560.0 {
            1
        } else if ui.available_width() < 900.0 {
            2
        } else {
            3
        };
        egui::Grid::new("home_overview_metrics")
            .num_columns(columns)
            .spacing(egui::vec2(10.0, 10.0))
            .show(ui, |ui| {
                for (index, (label, value, detail)) in metrics.into_iter().enumerate() {
                    theme::card(ui, &palette, |ui| {
                        ui.set_min_width((ui.available_width() - 12.0).max(120.0));
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new(label)
                                .size(11.0)
                                .strong()
                                .color(palette.text_dim),
                        );
                        ui.label(egui::RichText::new(value).size(30.0).strong());
                        ui.label(
                            egui::RichText::new(detail)
                                .size(12.0)
                                .color(palette.text_dim),
                        );
                    });
                    if (index + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
            });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(self.language_pack.text("ui.home_page.014"))
                    .size(theme::FONT_SECTION)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .link(self.language_pack.text("ui.home_page.015"))
                    .clicked()
                {
                    self.navigate_to(DesktopPage::Library);
                }
            });
        });

        let entries: Vec<_> = self
            .controller
            .library()
            .entries()
            .map(|entry| {
                (
                    entry.title().to_owned(),
                    entry.path().to_path_buf(),
                    super::library_page::live_path_size(entry.path()),
                )
            })
            .collect();

        theme::card(ui, &palette, |ui| {
            ui.set_width(ui.available_width());
            if entries.is_empty() {
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.home_page.016"))
                        .strong()
                        .size(16.0),
                );
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.home_page.017"))
                        .color(palette.text_dim),
                );
                if ui
                    .button(self.language_pack.text("ui.home_page.018"))
                    .clicked()
                {
                    self.choose_content_file();
                }
                return;
            }
            for (index, (title, path, size)) in entries.iter().take(5).enumerate() {
                if index > 0 {
                    ui.separator();
                }
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("ROM")
                    .to_ascii_uppercase();
                ui.horizontal_wrapped(|ui| {
                    egui::Frame::none()
                        .fill(palette.accent_subtle)
                        .rounding(egui::Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(extension)
                                    .strong()
                                    .color(palette.accent_text),
                            );
                        });
                    ui.vertical(|ui| {
                        ui.set_max_width((ui.available_width() - 90.0).max(100.0));
                        ui.label(egui::RichText::new(title).strong());
                        ui.label(
                            egui::RichText::new(format!(
                                "{}  ·  {}",
                                path.file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("ROM"),
                                super::library_page::display_path_size(*size, french)
                            ))
                            .size(12.0)
                            .color(palette.text_dim),
                        );
                    });
                    if ui
                        .button(if french {
                            "Ouvrir"
                        } else {
                            "Open"
                        })
                        .clicked()
                    {
                        self.content_path = path.display().to_string();
                        self.selected_content = Some(path.clone());
                        self.navigate_to(DesktopPage::Library);
                    }
                });
            }
        });

        ui.add_space(4.0);
        theme::card(ui, &palette, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(self.language_pack.text("ui.home_page.020"))
                            .size(11.0)
                            .strong()
                            .color(palette.text_dim),
                    );
                    ui.label(
                        egui::RichText::new(self.language_pack.text("ui.home_page.021"))
                            .strong()
                            .size(15.0),
                    );
                    ui.label(self.language_pack.text("ui.home_page.022"));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::badge(
                        ui,
                        self.language_pack.text("ui.home_page.023"),
                        palette.accent,
                    );
                });
            });
        });
        ui.add_space(16.0);
    }
}
