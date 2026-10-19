//! Application chrome: icon rail and contextual sidebar.

use super::*;
use crate::theme::{Icon, Palette};

const RAIL_WIDTH: f32 = 72.0;
const SIDEBAR_WIDTH: f32 = 232.0;
const SIDEBAR_PADDING: f32 = 16.0;

impl DesktopPage {
    fn icon(self) -> Icon {
        match self {
            Self::Home => Icon::Home,
            Self::Library => Icon::Library,
            Self::Sessions => Icon::Game,
            Self::Game => Icon::Game,
            Self::Systems => Icon::Systems,
            Self::Settings => Icon::Settings,
        }
    }

    /// Pages with sub-sections show the contextual sidebar next to the rail.
    fn has_sidebar(self) -> bool {
        matches!(self, Self::Library | Self::Settings)
    }
}

impl DesktopApp {
    pub(super) fn show_shell(&mut self, context: &egui::Context) {
        let palette = Palette::new(self.dark_mode, self.accent_color);
        self.show_rail(context, &palette);
        self.show_context_sidebar(context, &palette);
    }

    fn show_rail(&mut self, context: &egui::Context, palette: &Palette) {
        egui::SidePanel::left("rail")
            .resizable(false)
            .exact_width(RAIL_WIDTH)
            .frame(
                egui::Frame::none()
                    .fill(palette.rail)
                    .inner_margin(egui::Margin::symmetric(14.0, 16.0)),
            )
            .show(context, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 8.0);
                // Logo box at the top of the rail.
                if let Some(texture) = theme::logo_texture(context) {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
                    let texture_size = texture.size_vec2();
                    let scale = 36.0 / texture_size.x.max(texture_size.y).max(1.0);
                    let image_rect =
                        egui::Rect::from_center_size(rect.center(), texture_size * scale);
                    ui.painter().image(
                        texture.id(),
                        image_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    ui.add_space(8.0);
                }
                for page in [
                    DesktopPage::Home,
                    DesktopPage::Library,
                    DesktopPage::Sessions,
                    DesktopPage::Systems,
                ] {
                    if theme::rail_button(
                        ui,
                        palette,
                        page.icon(),
                        self.active_page == page,
                        page.title(&self.language_pack),
                    )
                    .clicked()
                    {
                        self.navigate_to(page);
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if theme::rail_button(
                        ui,
                        palette,
                        Icon::Settings,
                        self.active_page == DesktopPage::Settings,
                        DesktopPage::Settings.title(&self.language_pack),
                    )
                    .clicked()
                    {
                        self.navigate_to(DesktopPage::Settings);
                    }
                    if self.active_page.has_sidebar()
                        && theme::rail_button(
                            ui,
                            palette,
                            Icon::Sidebar,
                            self.sidebar_open,
                            self.language_pack.text("ui.shell.002"),
                        )
                        .clicked()
                    {
                        self.sidebar_open = !self.sidebar_open;
                    }
                });
            });
    }

    /// Measure the active sidebar's content so labels and counters never rely on
    /// a fixed width. Keep the animation proportional to this measured width.
    fn context_sidebar_width(&self, context: &egui::Context) -> f32 {
        let labels: Vec<(&str, f32)> = match self.active_page {
            DesktopPage::Library => vec![
                (self.language_pack.text("ui.shell.003"), 18.0),
                (self.language_pack.text("ui.shell.004"), 13.0),
                (self.language_pack.text("ui.shell.005"), 14.0),
                ("Game Boy (GB / GBC / GBA)", 14.0),
            ],
            DesktopPage::Settings => vec![
                (self.language_pack.text("ui.shell.006"), 18.0),
                (self.language_pack.text("ui.shell.007"), 13.0),
                (self.language_pack.text("ui.shell.008"), 14.0),
                ("Application", 14.0),
            ],
            _ => Vec::new(),
        };
        let text_width = context.fonts(|fonts| {
            labels
                .iter()
                .map(|(label, size)| {
                    fonts
                        .layout_no_wrap(
                            (*label).to_owned(),
                            egui::FontId::proportional(*size),
                            egui::Color32::WHITE,
                        )
                        .size()
                        .x
                })
                .fold(0.0_f32, f32::max)
        });
        // Navigation rows reserve space for the icon and the optional count pill.
        let nav_label_width = context.fonts(|fonts| {
            labels
                .iter()
                .filter(|(_, size)| *size == 14.0)
                .map(|(label, _)| {
                    fonts
                        .layout_no_wrap(
                            (*label).to_owned(),
                            egui::FontId::proportional(14.0),
                            egui::Color32::WHITE,
                        )
                        .size()
                        .x
                })
                .fold(0.0_f32, f32::max)
        });
        let navigation_width = nav_label_width + 42.0 + 44.0;
        (text_width.max(navigation_width) + 2.0 * SIDEBAR_PADDING + 4.0).max(SIDEBAR_WIDTH)
    }

    fn show_context_sidebar(&mut self, context: &egui::Context, palette: &Palette) {
        let visible = self.sidebar_open && self.active_page.has_sidebar();
        let desired_width = self.context_sidebar_width(context);
        // The sidebar width follows its measured content while retaining the open/close animation.
        let openness =
            context.animate_bool_with_time(egui::Id::new("context_sidebar_open"), visible, 0.15);
        if openness <= 0.0 {
            return;
        }
        egui::SidePanel::left("context_sidebar")
            .resizable(false)
            .exact_width(desired_width * openness)
            .frame(egui::Frame::none().fill(palette.sidebar))
            .show(context, |ui| {
                let clip = ui.clip_rect().intersect(ui.max_rect());
                ui.set_clip_rect(clip);
                egui::Frame::none()
                    .inner_margin(egui::Margin::symmetric(SIDEBAR_PADDING, 20.0))
                    .show(ui, |ui| {
                        ui.set_width((desired_width - 2.0 * SIDEBAR_PADDING).max(0.0));
                        match self.active_page {
                            DesktopPage::Library => self.show_library_sidebar(ui, palette),
                            DesktopPage::Settings => self.show_settings_sidebar(ui, palette),
                            _ => {}
                        }
                    });
            });
    }

    fn show_library_sidebar(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let mut all = 0usize;
        let mut game_boy = 0usize;
        for entry in self.controller.library().entries() {
            all += 1;
            if LibraryFormat::GameBoy.matches(entry.path()) {
                game_boy += 1;
            }
        }
        ui.label(
            egui::RichText::new(self.language_pack.text("ui.shell.003"))
                .size(18.0)
                .strong(),
        );
        ui.label(
            egui::RichText::new(self.language_pack.text("ui.shell.004"))
                .size(13.0)
                .color(palette.text_dim),
        );
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        for (format, label, count) in [
            (
                LibraryFormat::All,
                self.language_pack.text("ui.shell.005"),
                all,
            ),
            (
                LibraryFormat::GameBoy,
                "Game Boy (GB / GBC / GBA)",
                game_boy,
            ),
        ] {
            if theme::nav_item(
                ui,
                palette,
                Icon::Game,
                label,
                Some(count),
                self.library_format == format,
            )
            .clicked()
            {
                self.library_format = format;
            }
        }
    }

    fn show_settings_sidebar(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.label(
            egui::RichText::new(self.language_pack.text("ui.shell.006"))
                .size(18.0)
                .strong(),
        );
        ui.label(
            egui::RichText::new(self.language_pack.text("ui.shell.007"))
                .size(13.0)
                .color(palette.text_dim),
        );
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        for (section, label, icon) in [
            (
                SettingsSection::Appearance,
                self.language_pack.text("ui.shell.008"),
                Icon::Settings,
            ),
            (SettingsSection::Application, "Application", Icon::Systems),
        ] {
            if theme::nav_item(
                ui,
                palette,
                icon,
                label,
                None,
                self.settings_section == section,
            )
            .clicked()
            {
                self.settings_section = section;
            }
        }
    }
}
