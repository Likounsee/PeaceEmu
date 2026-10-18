//! Design tokens and small drawing helpers shared by the desktop pages.
//!
//! The look is deliberately neutral: flat grey surfaces, 1 px borders, rounded
//! controls and a single accent colour chosen by the user. The light and dark
//! variants share one structure, so pages ask the [`Palette`] for colours
//! instead of hard-coding them.

use eframe::egui::{self, Align2, Color32, FontId, Margin, Pos2, Rect, Rounding, Stroke, Vec2};

/// Accent colour used until the user picks another one.
pub const DEFAULT_ACCENT: Color32 = Color32::from_rgb(79, 70, 229);

/// Font size of a page title.
pub const FONT_TITLE: f32 = 30.0;
/// Font size of a section title inside a page.
pub const FONT_SECTION: f32 = 17.0;
/// Corner radius of cards.
pub const RADIUS_CARD: f32 = 12.0;

/// Colours for one appearance mode, derived from the user's accent colour.
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Icon rail and text-input background.
    pub rail: Color32,
    /// Contextual sidebar, cards and buttons.
    pub sidebar: Color32,
    /// Main content panel.
    pub panel: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    /// Accent shifted so it stays readable when used as text on a surface.
    pub accent_text: Color32,
    /// Translucent accent used for selected backgrounds.
    pub accent_subtle: Color32,
    /// Text colour that stays readable on top of the accent.
    pub on_accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
}

/// Returns `color` with its opacity replaced by `alpha`.
pub fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// Shifts a colour so it stays readable as text: lighter on dark surfaces,
/// darker on light ones. Saturated status colours are otherwise hard to read
/// (about 2.5:1) on a white or near-black background.
pub fn readable_on_surface(dark: bool, color: Color32) -> Color32 {
    let shift = |channel: u8| -> u8 {
        let value = f32::from(channel);
        let shifted = if dark {
            value + (255.0 - value) * 0.4
        } else {
            value * 0.62
        };
        shifted as u8
    };
    Color32::from_rgb(shift(color.r()), shift(color.g()), shift(color.b()))
}

fn readable_text_on(background: Color32) -> Color32 {
    let luminance = 0.299 * f32::from(background.r())
        + 0.587 * f32::from(background.g())
        + 0.114 * f32::from(background.b());
    if luminance > 165.0 {
        Color32::from_rgb(24, 24, 27)
    } else {
        Color32::WHITE
    }
}

impl Palette {
    pub fn new(dark: bool, accent: Color32) -> Self {
        let (rail, sidebar, panel, border, text, text_dim) = if dark {
            (
                Color32::from_rgb(19, 19, 21),
                Color32::from_rgb(27, 27, 30),
                Color32::from_rgb(25, 25, 28),
                Color32::from_rgb(43, 43, 48),
                Color32::from_rgb(244, 244, 245),
                Color32::from_rgb(161, 161, 170),
            )
        } else {
            (
                Color32::from_rgb(244, 244, 245),
                Color32::from_rgb(250, 250, 250),
                Color32::WHITE,
                Color32::from_rgb(228, 228, 231),
                Color32::from_rgb(24, 24, 27),
                Color32::from_rgb(113, 113, 122),
            )
        };
        Self {
            dark,
            rail,
            sidebar,
            panel,
            border,
            text,
            text_dim,
            accent,
            accent_text: readable_on_surface(dark, accent),
            accent_subtle: with_alpha(accent, if dark { 64 } else { 34 }),
            on_accent: readable_text_on(accent),
            success: Color32::from_rgb(23, 178, 106),
            warning: Color32::from_rgb(247, 144, 9),
            danger: Color32::from_rgb(240, 68, 56),
        }
    }

    /// Installs this palette as the egui style for the whole application.
    pub fn apply(&self, context: &egui::Context) {
        context.set_visuals(if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        context.style_mut(|style| {
            style.spacing.item_spacing = egui::vec2(10.0, 10.0);
            style.spacing.button_padding = egui::vec2(14.0, 8.0);
            style.spacing.window_margin = Margin::same(18.0);
            style.spacing.menu_margin = Margin::same(8.0);
            style.spacing.indent = 18.0;
            let visuals = &mut style.visuals;
            visuals.panel_fill = self.sidebar;
            visuals.window_fill = self.sidebar;
            visuals.extreme_bg_color = self.rail;
            visuals.faint_bg_color = self.sidebar;
            visuals.hyperlink_color = self.accent;
            visuals.window_stroke = Stroke::new(1.0_f32, self.border);
            visuals.window_rounding = Rounding::same(12.0);
            visuals.menu_rounding = Rounding::same(10.0);
            visuals.selection.bg_fill = self.accent_subtle;
            visuals.selection.stroke = Stroke::new(1.0_f32, self.accent);
            let rounding = Rounding::same(8.0);
            let widgets = &mut visuals.widgets;
            widgets.noninteractive.bg_fill = self.panel;
            widgets.noninteractive.weak_bg_fill = self.panel;
            widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, self.border);
            widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, self.text);
            widgets.noninteractive.rounding = rounding;
            widgets.inactive.bg_fill = self.sidebar;
            widgets.inactive.weak_bg_fill = self.sidebar;
            widgets.inactive.bg_stroke = Stroke::new(1.0_f32, self.border);
            widgets.inactive.fg_stroke = Stroke::new(1.0_f32, self.text);
            widgets.inactive.rounding = rounding;
            widgets.hovered.bg_fill = self.border;
            widgets.hovered.weak_bg_fill = self.border;
            widgets.hovered.bg_stroke = Stroke::new(1.0_f32, self.accent);
            widgets.hovered.fg_stroke = Stroke::new(1.0_f32, self.text);
            widgets.hovered.rounding = rounding;
            widgets.active.bg_fill = self.accent_subtle;
            widgets.active.weak_bg_fill = self.accent_subtle;
            widgets.active.bg_stroke = Stroke::new(1.0_f32, self.accent);
            widgets.active.fg_stroke = Stroke::new(1.0_f32, self.text);
            widgets.active.rounding = rounding;
            widgets.open.bg_fill = self.border;
            widgets.open.weak_bg_fill = self.border;
            widgets.open.bg_stroke = Stroke::new(1.0_f32, self.border);
            widgets.open.fg_stroke = Stroke::new(1.0_f32, self.text);
            widgets.open.rounding = rounding;
        });
    }
}

/// A bordered surface with rounded corners.
pub fn card<R>(
    ui: &mut egui::Ui,
    palette: &Palette,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::Frame::none()
        .fill(palette.sidebar)
        .stroke(Stroke::new(1.0_f32, palette.border))
        .rounding(Rounding::same(RADIUS_CARD))
        .inner_margin(Margin::same(16.0))
        .show(ui, add_contents)
}

/// A small pill used for statuses such as "Ready" or "Missing".
///
/// The text colour is adjusted for the current mode so it stays readable.
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    let text_color = readable_on_surface(ui.visuals().dark_mode, color);
    egui::Frame::none()
        .fill(with_alpha(color, 40))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(8.0, 2.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(12.0)
                    .strong()
                    .color(text_color),
            );
        });
}

/// The filled call-to-action button drawn with the accent colour.
pub fn primary_button(palette: &Palette, text: &str) -> impl egui::Widget {
    egui::Button::new(egui::RichText::new(text).strong().color(palette.on_accent))
        .fill(palette.accent)
        .stroke(Stroke::NONE)
        .rounding(Rounding::same(8.0))
        .min_size(egui::vec2(0.0, 36.0))
}

/// Page title, subtitle and actions, followed by a separator.
///
/// Actions are listed primary-first: they sit on the right (rightmost first)
/// when there is room, and wrap left-aligned under the title otherwise, so
/// two buttons can never overlap.
pub fn page_header(
    ui: &mut egui::Ui,
    palette: &Palette,
    title: &str,
    subtitle: &str,
    add_actions: impl FnOnce(&mut egui::Ui),
) {
    let title_block = |ui: &mut egui::Ui| {
        ui.label(egui::RichText::new(title).size(FONT_TITLE).strong());
        ui.label(egui::RichText::new(subtitle).color(palette.text_dim));
    };
    if ui.available_width() < 560.0 {
        ui.vertical(title_block);
        ui.horizontal_wrapped(add_actions);
    } else {
        ui.horizontal(|ui| {
            ui.vertical(title_block);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), add_actions);
        });
    }
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);
}

/// One settings row: title and description on the left, the control on the right.
pub fn setting_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    title: &str,
    description: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(250.0);
            ui.label(egui::RichText::new(title).strong().size(14.0));
            ui.label(
                egui::RichText::new(description)
                    .size(13.0)
                    .color(palette.text_dim),
            );
        });
        ui.add_space(16.0);
        ui.vertical(add_control);
    });
    ui.add_space(6.0);
    ui.separator();
}

/// Draws a small handheld console, themed with the palette.
pub fn paint_console(painter: &egui::Painter, rect: Rect, palette: &Palette, title: &str) {
    let rounding = Rounding::same(18.0);
    painter.rect_filled(rect, rounding, palette.border);
    painter.rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0_f32, with_alpha(palette.text_dim, 80)),
    );
    let screen = Rect::from_min_max(
        rect.min + egui::vec2(rect.width() * 0.12, rect.height() * 0.09),
        egui::pos2(
            rect.max.x - rect.width() * 0.12,
            rect.min.y + rect.height() * 0.58,
        ),
    );
    painter.rect_filled(screen, Rounding::same(8.0), palette.rail);
    painter.text(
        screen.center(),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(16.0),
        palette.accent_text,
    );
    let pad = egui::pos2(
        rect.min.x + rect.width() * 0.24,
        rect.min.y + rect.height() * 0.80,
    );
    let pad_stroke = Stroke::new(4.0_f32, palette.text_dim);
    painter.line_segment(
        [pad - egui::vec2(9.0, 0.0), pad + egui::vec2(9.0, 0.0)],
        pad_stroke,
    );
    painter.line_segment(
        [pad - egui::vec2(0.0, 9.0), pad + egui::vec2(0.0, 9.0)],
        pad_stroke,
    );
    painter.circle_filled(
        egui::pos2(
            rect.max.x - rect.width() * 0.26,
            rect.min.y + rect.height() * 0.78,
        ),
        8.0,
        palette.accent,
    );
    painter.circle_filled(
        egui::pos2(
            rect.max.x - rect.width() * 0.15,
            rect.min.y + rect.height() * 0.70,
        ),
        8.0,
        with_alpha(palette.accent, 150),
    );
}

/// Draws the round check mark shown on selected choice cards.
pub fn paint_check_badge(painter: &egui::Painter, center: Pos2, palette: &Palette) {
    painter.circle_filled(center, 9.0, palette.accent);
    let stroke = Stroke::new(2.0_f32, palette.on_accent);
    painter.line_segment(
        [
            center + egui::vec2(-4.0, 0.5),
            center + egui::vec2(-1.0, 3.5),
        ],
        stroke,
    );
    painter.line_segment(
        [
            center + egui::vec2(-1.0, 3.5),
            center + egui::vec2(4.5, -3.0),
        ],
        stroke,
    );
}

/// A clickable card with a custom preview and an accent border when selected.
pub fn choice_card(
    ui: &mut egui::Ui,
    palette: &Palette,
    selected: bool,
    size: Vec2,
    paint_preview: impl FnOnce(&egui::Painter, Rect),
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();
    paint_preview(painter, rect);
    let stroke = if selected {
        Stroke::new(2.0_f32, palette.accent)
    } else if response.hovered() {
        Stroke::new(1.0_f32, palette.text_dim)
    } else {
        Stroke::new(1.0_f32, palette.border)
    };
    painter.rect_stroke(rect, Rounding::same(10.0), stroke);
    if selected {
        paint_check_badge(painter, rect.right_top() + egui::vec2(-14.0, 14.0), palette);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Outline icons drawn with the painter, so no icon font or image is needed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Home,
    Library,
    Sessions,
    Game,
    Systems,
    Settings,
    Sidebar,
    Fullscreen,
}

pub fn paint_icon(painter: &egui::Painter, center: Pos2, size: f32, icon: Icon, color: Color32) {
    let stroke = Stroke::new(1.7_f32, color);
    let half = size / 2.0;
    let at = |x: f32, y: f32| center + egui::vec2(x * half, y * half);
    match icon {
        Icon::Home => {
            painter.add(egui::Shape::closed_line(
                vec![
                    at(-0.8, -0.1),
                    at(0.0, -0.85),
                    at(0.8, -0.1),
                    at(0.8, 0.8),
                    at(-0.8, 0.8),
                ],
                stroke,
            ));
        }
        Icon::Library => {
            for (x, y) in [(-0.9, -0.9), (0.1, -0.9), (-0.9, 0.1), (0.1, 0.1)] {
                painter.rect_stroke(
                    Rect::from_min_size(at(x, y), egui::vec2(half * 0.8, half * 0.8)),
                    Rounding::same(2.0),
                    stroke,
                );
            }
        }
        Icon::Sessions => {
            for y in [-0.6, 0.0, 0.6] {
                painter.line_segment([at(-0.35, y), at(0.9, y)], stroke);
                painter.circle_filled(at(-0.85, y), 1.5, color);
            }
        }
        Icon::Game => {
            painter.rect_stroke(
                Rect::from_min_max(at(-1.0, -0.5), at(1.0, 0.5)),
                Rounding::same(half * 0.5),
                stroke,
            );
            painter.line_segment([at(-0.55, -0.2), at(-0.55, 0.2)], stroke);
            painter.line_segment([at(-0.75, 0.0), at(-0.35, 0.0)], stroke);
            painter.circle_filled(at(0.4, -0.1), 1.6, color);
            painter.circle_filled(at(0.65, 0.12), 1.6, color);
        }
        Icon::Systems => {
            painter.add(egui::Shape::closed_line(
                vec![at(0.0, -0.9), at(0.95, -0.4), at(0.0, 0.1), at(-0.95, -0.4)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![at(-0.95, 0.2), at(0.0, 0.7), at(0.95, 0.2)],
                stroke,
            ));
        }
        Icon::Settings => {
            for (y, knob) in [(-0.6_f32, 0.35_f32), (0.0, -0.4), (0.6, 0.5)] {
                painter.line_segment([at(-0.9, y), at(0.9, y)], stroke);
                painter.circle_filled(at(knob, y), 2.6, color);
            }
        }
        Icon::Sidebar => {
            painter.rect_stroke(
                Rect::from_min_max(at(-0.9, -0.75), at(0.9, 0.75)),
                Rounding::same(3.0),
                stroke,
            );
            painter.line_segment([at(-0.3, -0.75), at(-0.3, 0.75)], stroke);
        }
        Icon::Fullscreen => {
            for (sx, sy) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = at(0.85 * sx, 0.85 * sy);
                painter.line_segment([corner, at(0.35 * sx, 0.85 * sy)], stroke);
                painter.line_segment([corner, at(0.85 * sx, 0.35 * sy)], stroke);
            }
        }
    }
}

/// A square icon button for the left rail.
pub fn rail_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: Icon,
    selected: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::click());
    let painter = ui.painter();
    if selected || response.hovered() {
        painter.rect_filled(
            rect,
            Rounding::same(10.0),
            if selected {
                palette.border
            } else {
                with_alpha(palette.border, 120)
            },
        );
    }
    let color = if selected {
        palette.text
    } else {
        palette.text_dim
    };
    paint_icon(painter, rect.center(), 20.0, icon, color);
    response
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A full-width navigation row with an icon, a label and an optional counter.
pub fn nav_item(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: Icon,
    label: &str,
    count: Option<usize>,
    selected: bool,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 38.0), egui::Sense::click());
    let painter = ui.painter();
    if selected || response.hovered() {
        painter.rect_filled(
            rect,
            Rounding::same(8.0),
            if selected {
                palette.border
            } else {
                with_alpha(palette.border, 120)
            },
        );
    }
    let color = if selected {
        palette.text
    } else {
        palette.text_dim
    };
    paint_icon(
        painter,
        rect.left_center() + egui::vec2(20.0, 0.0),
        18.0,
        icon,
        color,
    );
    painter.text(
        rect.left_center() + egui::vec2(42.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        palette.text,
    );
    if let Some(count) = count {
        let pill = Rect::from_center_size(
            rect.right_center() - egui::vec2(26.0, 0.0),
            egui::vec2(30.0, 20.0),
        );
        painter.rect_filled(
            pill,
            Rounding::same(10.0),
            if selected {
                palette.sidebar
            } else {
                palette.border
            },
        );
        painter.text(
            pill.center(),
            Align2::CENTER_CENTER,
            count.to_string(),
            FontId::proportional(12.0),
            palette.text_dim,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Logo shown at the top of the icon rail, decoded once and cached in egui's memory.
///
/// `bird.png` is currently very large (about 6 MB); a 256 px copy would keep the
/// binary and the first frame lighter. Only change the path below to swap it.
const LOGO_PNG: &[u8] = include_bytes!("../assets/branding/bird.png");

pub fn logo_texture(context: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("peaceemu_logo_texture");
    if let Some(texture) = context.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let decoded = image::load_from_memory_with_format(LOGO_PNG, image::ImageFormat::Png).ok()?;
    let thumbnail = decoded.thumbnail(128, 128).to_rgba8();
    let size = [thumbnail.width() as usize, thumbnail.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, thumbnail.as_raw());
    let texture = context.load_texture("peaceemu-logo", color_image, egui::TextureOptions::LINEAR);
    context.data_mut(|data| data.insert_temp(id, texture.clone()));
    Some(texture)
}

/// Flat cover colour derived from a title, so each game keeps a stable tile.
pub fn cover_color(title: &str) -> Color32 {
    const COVERS: [(u8, u8, u8); 6] = [
        (46, 42, 99),
        (31, 75, 69),
        (90, 46, 62),
        (55, 65, 110),
        (98, 70, 30),
        (60, 60, 70),
    ];
    let hash = title.bytes().fold(0usize, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(usize::from(byte))
    });
    let (r, g, b) = COVERS[hash % COVERS.len()];
    Color32::from_rgb(r, g, b)
}

/// Up to two capital letters taken from the words of a title.
pub fn initials(title: &str) -> String {
    let letters: String = title
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    if letters.is_empty() {
        "?".to_owned()
    } else {
        letters
    }
}
