//! Appearance page: theme, accent colour, interface scale and language.

use super::*;

const ACCENT_PRESETS: [(&str, egui::Color32); 6] = [
    ("Indigo", egui::Color32::from_rgb(79, 70, 229)),
    ("Blue", egui::Color32::from_rgb(37, 99, 235)),
    ("Teal", egui::Color32::from_rgb(13, 148, 136)),
    ("Green", egui::Color32::from_rgb(22, 163, 74)),
    ("Orange", egui::Color32::from_rgb(234, 88, 12)),
    ("Rose", egui::Color32::from_rgb(219, 39, 119)),
];

/// Draws a miniature application window for the theme choice cards.
fn paint_theme_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    dark: bool,
    accent: egui::Color32,
) {
    let (window, side, block, line) = if dark {
        (
            egui::Color32::from_rgb(25, 25, 28),
            egui::Color32::from_rgb(36, 36, 40),
            egui::Color32::from_rgb(43, 43, 48),
            egui::Color32::from_rgb(63, 63, 70),
        )
    } else {
        (
            egui::Color32::WHITE,
            egui::Color32::from_rgb(244, 244, 245),
            egui::Color32::from_rgb(244, 244, 245),
            egui::Color32::from_rgb(212, 212, 216),
        )
    };
    painter.rect_filled(rect, egui::Rounding::same(10.0), window);
    let sidebar = egui::Rect::from_min_size(rect.min, egui::vec2(40.0, rect.height()));
    painter.rect_filled(
        sidebar,
        egui::Rounding {
            nw: 10.0,
            ne: 0.0,
            sw: 10.0,
            se: 0.0,
        },
        side,
    );
    for index in 0..3 {
        let y = rect.top() + 14.0 + index as f32 * 10.0;
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(rect.left() + 8.0, y), egui::vec2(24.0, 4.0)),
            egui::Rounding::same(2.0),
            line,
        );
    }
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(rect.right() - 54.0, rect.top() + 12.0),
            egui::vec2(42.0, 9.0),
        ),
        egui::Rounding::same(3.0),
        accent,
    );
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(rect.left() + 52.0, rect.top() + 32.0),
            egui::pos2(rect.right() - 12.0, rect.bottom() - 12.0),
        ),
        egui::Rounding::same(6.0),
        block,
    );
}

impl DesktopApp {
    pub(super) fn show_appearance(&mut self, ui: &mut egui::Ui) {
        let palette = theme::Palette::new(self.dark_mode, self.accent_color);
        let previous_dark_mode = self.dark_mode;
        let previous_ui_scale = self.ui_scale;
        let previous_accent_color = self.accent_color;

        ui.label(
            egui::RichText::new(self.language_pack.text("ui.appearance_page.001"))
                .size(30.0)
                .strong(),
        );
        ui.label(
            egui::RichText::new(self.language_pack.text("ui.appearance_page.002"))
                .color(palette.text_dim),
        );
        ui.add_space(8.0);
        ui.separator();

        theme::setting_row(
            ui,
            &palette,
            self.language_pack.text("ui.appearance_page.003"),
            self.language_pack.text("ui.appearance_page.004"),
            |ui| {
                let accent = self.accent_color;
                ui.horizontal(|ui| {
                    for (dark, label) in [
                        (false, self.language_pack.text("ui.appearance_page.005")),
                        (true, self.language_pack.text("ui.appearance_page.006")),
                    ] {
                        ui.vertical(|ui| {
                            let selected = self.dark_mode == dark;
                            let response = theme::choice_card(
                                ui,
                                &palette,
                                selected,
                                egui::vec2(190.0, 96.0),
                                |painter, rect| paint_theme_preview(painter, rect, dark, accent),
                            );
                            if response.clicked() {
                                self.dark_mode = dark;
                            }
                            ui.label(egui::RichText::new(label).strong());
                        });
                    }
                });
            },
        );

        theme::setting_row(
            ui,
            &palette,
            self.language_pack.text("ui.appearance_page.007"),
            self.language_pack.text("ui.appearance_page.008"),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (name, color) in ACCENT_PRESETS {
                        let selected = self.accent_color == color;
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::click());
                        ui.painter().circle_filled(rect.center(), 12.0, color);
                        if selected {
                            ui.painter().circle_stroke(
                                rect.center(),
                                15.0,
                                egui::Stroke::new(2.0_f32, color),
                            );
                        }
                        if response.clicked() {
                            self.accent_color = color;
                        }
                        response.on_hover_text(name);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(self.language_pack.text("ui.appearance_page.009"));
                    ui.color_edit_button_srgba(&mut self.accent_color);
                    ui.monospace(format!(
                        "#{:02X}{:02X}{:02X}",
                        self.accent_color.r(),
                        self.accent_color.g(),
                        self.accent_color.b()
                    ));
                });
            },
        );

        let language_label = self.language_pack.text("ui.appearance_page.010").to_owned();
        let language_description = self.language_pack.text("ui.appearance_page.011").to_owned();
        theme::setting_row(
            ui,
            &palette,
            &language_label,
            &language_description,
            |ui| {
                let previous_language = self.language.clone();
                egui::ComboBox::from_id_source("application-language")
                    .selected_text(self.language_pack.text("ui.appearance_page.012"))
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.language, "fr".to_owned(), "Français");
                        ui.selectable_value(&mut self.language, "en".to_owned(), "English");
                    });
                if previous_language != self.language {
                    self.message = if self.language == "fr" {
                        "Langue modifiée.".to_owned()
                    } else {
                        "Language changed.".to_owned()
                    };
                    self.save_preferences();
                }
            },
        );

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui
                .button(self.language_pack.text("ui.appearance_page.013"))
                .clicked()
            {
                self.dark_mode = true;
                self.accent_color = theme::DEFAULT_ACCENT;
                self.ui_scale = 1.0;
            }
            ui.label(
                egui::RichText::new(self.language_pack.text("ui.appearance_page.014"))
                    .size(13.0)
                    .color(palette.text_dim),
            );
        });

        // Persist visual preferences as soon as the user changes them.
        // Comparing against this frame's snapshot avoids writing the config every frame.
        if previous_dark_mode != self.dark_mode
            || (previous_ui_scale - self.ui_scale).abs() > f32::EPSILON
            || previous_accent_color != self.accent_color
        {
            self.save_preferences();
        }
    }
}
