//! Overview dashboard for the PeaceEmu desktop frontend.

use super::*;

fn metric_columns(available_width: f32) -> usize {
    if available_width < 560.0 {
        1
    } else if available_width < 900.0 {
        2
    } else {
        3
    }
}

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
                "SESSIONS",
                self.controller.session_count().to_string(),
                self.language_pack.text("ui.home_page.011"),
            ),
            (
                self.language_pack.text("ui.home_page.012"),
                available_systems.len().to_string(),
                self.language_pack.text("ui.home_page.013"),
            ),
        ];
        let columns = metric_columns(ui.available_width());
        let metric_width =
            ((ui.available_width() - 10.0 * (columns - 1) as f32) / columns as f32).max(140.0);
        egui::Grid::new("home_overview_metrics")
            .num_columns(columns)
            .spacing(egui::vec2(10.0, 10.0))
            .show(ui, |ui| {
                for (index, (label, value, detail)) in metrics.into_iter().enumerate() {
                    theme::card(ui, &palette, |ui| {
                        ui.set_min_width(metric_width);
                        ui.set_max_width(metric_width);
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
                    if ui.button(if french { "Ouvrir" } else { "Open" }).clicked() {
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
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(if french {
                    "ACTUALITÉS & INFORMATIONS"
                } else {
                    "NEWS & INFORMATION"
                })
                .size(theme::FONT_SECTION)
                .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(if french {
                        "CENTRE PEACEEMU"
                    } else {
                        "PEACEEMU HUB"
                    })
                    .size(10.0)
                    .strong()
                    .color(palette.text_dim),
                );
            });
        });

        let current_version = env!("CARGO_PKG_VERSION");
        let channel = option_env!("PEACEEMU_CHANNEL").unwrap_or("dev");
        let edition = if channel == "stable" {
            if french { "Stable" } else { "Stable" }
        } else if french {
            "Développement"
        } else {
            "Development"
        };
        let update_status = self.update_status.clone();
        let published_version = self
            .update_manifest
            .as_ref()
            .map(|manifest| manifest.version.clone());
        let module_count = nes_module_manager::available_module_ids().len();

        theme::card(ui, &palette, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(if french {
                    "Les dernières informations de votre émulateur"
                } else {
                    "The latest information about your emulator"
                })
                .size(14.0)
                .color(palette.text_dim),
            );
            ui.add_space(8.0);

            egui::CollapsingHeader::new(if french {
                format!("Version installée · {current_version} ({edition})")
            } else {
                format!("Installed version · {current_version} ({edition})")
            })
            .id_source("home_news_installed_version")
            .default_open(true)
            .show(ui, |ui| {
                ui.label(if french {
                    format!("Vous utilisez PeaceEmu {current_version}, édition {edition}.")
                } else {
                    format!("You are using PeaceEmu {current_version}, {edition} edition.")
                });
                ui.label(
                    egui::RichText::new(if french {
                        "Les informations de version sont calculées à partir de cette compilation."
                    } else {
                        "Version information is read from this application build."
                    })
                    .size(12.0)
                    .color(palette.text_dim),
                );
            });

            ui.separator();
            egui::CollapsingHeader::new(if french {
                "Mises à jour".to_owned()
            } else {
                "Updates".to_owned()
            })
            .id_source("home_news_updates")
            .default_open(true)
            .show(ui, |ui| {
                ui.label(&update_status);
                if let Some(version) = published_version.as_deref() {
                    ui.label(if french {
                        format!("Version annoncée par le canal : {version}")
                    } else {
                        format!("Version announced by the channel: {version}")
                    });
                }
                if ui
                    .link(if french {
                        "Ouvrir les paramètres de mise à jour"
                    } else {
                        "Open update settings"
                    })
                    .clicked()
                {
                    self.navigate_to(DesktopPage::Settings);
                }
            });

            ui.separator();
            egui::CollapsingHeader::new(if french {
                format!("Systèmes et modules · {module_count} module(s) disponible(s)")
            } else {
                format!("Systems and modules · {module_count} module(s) available")
            })
            .id_source("home_news_modules")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(if french {
                    "Consultez la page Systèmes pour découvrir les cœurs intégrés et les modules installables."
                } else {
                    "Visit the Systems page to explore built-in cores and installable modules."
                });
                if ui
                    .link(if french { "Voir les systèmes" } else { "View systems" })
                    .clicked()
                {
                    self.navigate_to(DesktopPage::Systems);
                }
            });
        });

        ui.add_space(16.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_metrics_choose_one_two_or_three_columns_at_responsive_breakpoints() {
        assert_eq!(metric_columns(420.0), 1);
        assert_eq!(metric_columns(760.0), 2);
        assert_eq!(metric_columns(1280.0), 3);
    }
}
