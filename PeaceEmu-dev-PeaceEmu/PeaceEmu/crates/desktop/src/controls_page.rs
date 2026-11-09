//! Controls page: one keyboard mapping per console, drawn with the shared
//! theme (page header, card, palette) so it matches Appearance and Application.

use super::*;

/// Groups of actions, as (French title, English title, action indexes).
/// Indexes refer to `key_bindings::ACTIONS`.
/// The NES controller has only A/B: X/Y are not physical NES buttons and
/// must not be offered as remappable actions in this page.
const GROUPS: [(&str, &str, &[usize]); 3] = [
    ("Directions", "Directions", &[0, 1, 2, 3]),
    ("Boutons", "Buttons", &[4, 5]),
    ("Système", "System", &[8, 9]),
];

fn module_display_name(module_id: &str) -> String {
    match module_id {
        "nes" => "Nintendo (NES)".to_owned(),
        "game-boy" => "Game Boy".to_owned(),
        other => other
            .split(['-', '_'])
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// A key shown as a rounded "keycap". While capturing, it switches to the
/// accent colours so the user sees which action is waiting for a key.
fn keycap(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    text: &str,
    capturing: bool,
) -> egui::Response {
    keycap_sized(ui, palette, text, capturing, 150.0)
}

/// Same as [`keycap`] with an explicit minimum width, so a row of keycaps
/// can be laid out on a regular grid.
fn keycap_sized(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    text: &str,
    capturing: bool,
    min_width: f32,
) -> egui::Response {
    let (fill, stroke, color) = if capturing {
        (
            palette.accent_subtle,
            egui::Stroke::new(1.5_f32, palette.accent),
            palette.accent_text,
        )
    } else {
        (
            palette.panel,
            egui::Stroke::new(1.0_f32, palette.border),
            palette.text,
        )
    };
    let button = egui::Button::new(
        egui::RichText::new(text)
            .monospace()
            .size(13.0)
            .color(color),
    )
    .fill(fill)
    .stroke(stroke)
    .rounding(egui::Rounding::same(8.0))
    .min_size(egui::vec2(min_width, 32.0));
    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

const fn rgb(r: u8, g: u8, b: u8) -> egui::Color32 {
    egui::Color32::from_rgb(r, g, b)
}

const NES_LIGHT: egui::Color32 = rgb(246, 247, 250);
const NES_RED: egui::Color32 = rgb(190, 62, 72);
const NES_FACE: egui::Color32 = rgb(24, 25, 28);
const NES_DEEP: egui::Color32 = rgb(8, 9, 11);

/// Shape used to highlight a physical button (hover / selection / capture).
#[derive(Clone, Copy)]
enum Glow {
    Rect(egui::Rect, f32),
    Circle(egui::Pos2, f32),
}

impl Glow {
    fn paint(self, painter: &egui::Painter, fill: egui::Color32, stroke: egui::Stroke) {
        match self {
            Glow::Rect(rect, radius) => {
                let rounding = egui::Rounding::same(radius);
                painter.rect_filled(rect, rounding, fill);
                painter.rect_stroke(rect, rounding, stroke);
            }
            Glow::Circle(center, radius) => {
                painter.circle_filled(center, radius, fill);
                painter.circle_stroke(center, radius, stroke);
            }
        }
    }
}

/// Plate with a drop shadow, a fill and an outline (one call instead of three).
fn plate(
    painter: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    drop: f32,
    shadow: egui::Color32,
    fill: egui::Color32,
    stroke: egui::Stroke,
) {
    let rounding = egui::Rounding::same(radius);
    if drop > 0.0 {
        painter.rect_filled(rect.translate(egui::vec2(0.0, drop)), rounding, shadow);
    }
    painter.rect_filled(rect, rounding, fill);
    painter.rect_stroke(rect, rounding, stroke);
}

/// egui ships no bold font: drawing the text three times, a hair apart,
/// gives the heavy printed look of the Nintendo legends.
fn bold_text(
    painter: &egui::Painter,
    pos: egui::Pos2,
    text: &str,
    size: f32,
    color: egui::Color32,
) {
    let font = egui::FontId::proportional(size);
    let offset = (size * 0.03).max(0.4);
    for dx in [-offset, 0.0, offset] {
        painter.text(
            pos + egui::vec2(dx, 0.0),
            egui::Align2::CENTER_CENTER,
            text,
            font.clone(),
            color,
        );
    }
}

/// Draws a fully native NES controller with egui primitives (no bitmap texture).
/// The key assigned to each button is printed on it. Returns the physical
/// action clicked by the user, if any.
fn draw_nes_controller(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    selected_action: usize,
    capturing: bool,
    french: bool,
    labels: &[String],
) -> Option<usize> {
    let width = ui.available_width().min(900.0);
    let size = egui::vec2(width, width * 0.502);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    // Every fixed pixel size (bevels, strokes, radii) scales with the drawing,
    // so the controller keeps its proportions when the window is resized.
    let s = (rect.width() / 900.0).clamp(0.45, 1.2);
    let p = |x: f32, y: f32| {
        egui::pos2(
            rect.left() + rect.width() * x,
            rect.top() + rect.height() * y,
        )
    };
    let r = |x: f32, y: f32, w: f32, h: f32| {
        egui::Rect::from_center_size(p(x, y), egui::vec2(rect.width() * w, rect.height() * h))
    };
    let stroke = |w: f32, c: egui::Color32| egui::Stroke::new(w * s, c);
    let key_label = |index: usize| -> String {
        if capturing && selected_action == index {
            "…".to_owned()
        } else {
            labels.get(index).cloned().unwrap_or_default()
        }
    };

    // Cable leaving the top of the casing, drawn first so the shell covers its root.
    let (c0, c1, c2, c3) = (
        p(0.27, 0.215).to_vec2(),
        p(0.27, 0.04).to_vec2(),
        p(0.31, -0.05).to_vec2(),
        p(0.38, -0.16).to_vec2(),
    );
    let cable: Vec<egui::Pos2> = (0..=24)
        .map(|i| {
            let t = i as f32 / 24.0;
            let u = 1.0 - t;
            (c0 * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + c3 * (t * t * t))
                .to_pos2()
        })
        .collect();
    painter.add(egui::Shape::line(
        cable.clone(),
        stroke(10.0, rgb(52, 54, 58)),
    ));
    let lit: Vec<egui::Pos2> = cable
        .iter()
        .map(|point| *point - egui::vec2(2.0 * s, 0.0))
        .collect();
    painter.add(egui::Shape::line(lit, stroke(2.5, rgb(98, 100, 105))));

    // Moulded plastic shell: stacked shadows, a bright rim and a fine inner
    // line give the bevel without any bitmap.
    let shell = r(0.5, 0.585, 0.992, 0.79);
    let shell_round = egui::Rounding::same(17.0 * s);
    painter.rect_filled(
        shell.translate(egui::vec2(0.0, 5.0 * s)),
        shell_round,
        rgb(76, 78, 83),
    );
    painter.rect_filled(
        shell.translate(egui::vec2(0.0, 2.5 * s)),
        shell_round,
        rgb(151, 153, 159),
    );
    painter.rect_filled(shell, shell_round, rgb(220, 222, 226));
    painter.rect_stroke(shell, shell_round, stroke(1.5, rgb(250, 250, 251)));
    painter.rect_stroke(
        shell.shrink(3.0 * s),
        egui::Rounding::same(14.0 * s),
        stroke(1.0, rgb(187, 189, 194)),
    );
    // Mould line across the top (dark groove + light edge below it).
    painter.line_segment(
        [p(0.035, 0.235), p(0.965, 0.235)],
        stroke(1.0, rgb(190, 192, 197)),
    );
    painter.line_segment(
        [
            p(0.035, 0.235) + egui::vec2(0.0, 1.3 * s),
            p(0.965, 0.235) + egui::vec2(0.0, 1.3 * s),
        ],
        stroke(1.0, rgb(244, 245, 247)),
    );
    // Cable strain relief.
    plate(
        &painter,
        r(0.27, 0.195, 0.065, 0.032),
        3.0 * s,
        0.0,
        rgb(0, 0, 0),
        rgb(205, 207, 212),
        stroke(1.0, rgb(150, 152, 158)),
    );

    // Dark face plate.
    let face = r(0.5, 0.655, 0.936, 0.575);
    plate(
        &painter,
        face,
        7.0 * s,
        2.5 * s,
        NES_DEEP,
        NES_FACE,
        stroke(1.2, rgb(91, 92, 96)),
    );
    painter.line_segment(
        [
            face.left_top() + egui::vec2(8.0 * s, 4.0 * s),
            face.right_top() + egui::vec2(-8.0 * s, 4.0 * s),
        ],
        stroke(1.0, rgb(57, 58, 62)),
    );

    // Grey moulded bars above and below the centre controls.
    for (y, h) in [(0.425, 0.105), (0.565, 0.095), (0.945, 0.045)] {
        plate(
            &painter,
            r(0.46, y, 0.285, h),
            8.0 * s,
            1.5 * s,
            rgb(13, 14, 16),
            rgb(177, 179, 184),
            stroke(1.0, rgb(221, 223, 227)),
        );
    }

    // ---------------------------------------------------------------- D-pad
    let dc = p(0.175, 0.655);
    let unit = rect.height() * 0.102;
    let well = egui::Rect::from_center_size(dc, egui::vec2(unit * 4.1, unit * 4.1));
    plate(
        &painter,
        well,
        7.0 * s,
        2.0 * s,
        NES_DEEP,
        rgb(33, 34, 38),
        stroke(1.0, rgb(74, 76, 81)),
    );
    // The cross is concave: `convex_polygon` would fan-fill it wrongly (that
    // produced the white wedges). Fill it as two rectangles and outline the
    // real 12-point polygon instead.
    let paint_cross = |scale: f32, fill: egui::Color32, outline: egui::Stroke| {
        let (arm, len) = (unit * scale * 0.39, unit * scale * 1.40);
        let square = egui::Rounding::same(0.0);
        painter.rect_filled(
            egui::Rect::from_center_size(dc, egui::vec2(arm * 2.0, len * 2.0)),
            square,
            fill,
        );
        painter.rect_filled(
            egui::Rect::from_center_size(dc, egui::vec2(len * 2.0, arm * 2.0)),
            square,
            fill,
        );
        let points = vec![
            egui::pos2(dc.x - arm, dc.y - len),
            egui::pos2(dc.x + arm, dc.y - len),
            egui::pos2(dc.x + arm, dc.y - arm),
            egui::pos2(dc.x + len, dc.y - arm),
            egui::pos2(dc.x + len, dc.y + arm),
            egui::pos2(dc.x + arm, dc.y + arm),
            egui::pos2(dc.x + arm, dc.y + len),
            egui::pos2(dc.x - arm, dc.y + len),
            egui::pos2(dc.x - arm, dc.y + arm),
            egui::pos2(dc.x - len, dc.y + arm),
            egui::pos2(dc.x - len, dc.y - arm),
            egui::pos2(dc.x - arm, dc.y - arm),
        ];
        painter.add(egui::Shape::closed_line(points, outline));
    };
    paint_cross(1.16, rgb(185, 187, 192), stroke(2.5, NES_LIGHT));
    paint_cross(1.02, rgb(15, 16, 18), stroke(2.0, rgb(92, 94, 99)));
    paint_cross(0.88, rgb(48, 50, 54), stroke(1.5, rgb(9, 10, 12)));

    // The four arms, used for both hit-testing and the selection glow.
    let u = unit * 1.02;
    let (arm, len) = (u * 0.39, u * 1.40);
    let up = egui::Rect::from_min_max(
        egui::pos2(dc.x - arm, dc.y - len),
        egui::pos2(dc.x + arm, dc.y - arm),
    );
    let down = egui::Rect::from_min_max(
        egui::pos2(dc.x - arm, dc.y + arm),
        egui::pos2(dc.x + arm, dc.y + len),
    );
    let left = egui::Rect::from_min_max(
        egui::pos2(dc.x - len, dc.y - arm),
        egui::pos2(dc.x - arm, dc.y + arm),
    );
    let right = egui::Rect::from_min_max(
        egui::pos2(dc.x + arm, dc.y - arm),
        egui::pos2(dc.x + len, dc.y + arm),
    );
    for dir in [
        egui::vec2(0.0, -1.0),
        egui::vec2(0.0, 1.0),
        egui::vec2(-1.0, 0.0),
        egui::vec2(1.0, 0.0),
    ] {
        let side = unit * 0.28;
        let perp = egui::vec2(-dir.y, dir.x);
        let center = dc + dir * (unit * 1.0);
        painter.add(egui::Shape::convex_polygon(
            vec![
                center + dir * side,
                center - dir * (side * 0.42) + perp * (side * 0.62),
                center - dir * (side * 0.42) - perp * (side * 0.62),
            ],
            rgb(132, 135, 141),
            stroke(0.9, rgb(5, 6, 8)),
        ));
    }
    painter.circle_filled(dc, unit * 0.22, rgb(7, 8, 10));
    painter.circle_filled(
        dc - egui::vec2(unit * 0.035, unit * 0.045),
        unit * 0.105,
        rgb(151, 154, 160),
    );
    painter.circle_stroke(dc, unit * 0.22, stroke(1.0, rgb(165, 168, 174)));

    // ------------------------------------------------------- SELECT / START
    bold_text(
        &painter,
        p(0.405, 0.648),
        "SELECT",
        rect.height() * 0.056,
        NES_RED,
    );
    bold_text(
        &painter,
        p(0.535, 0.648),
        "START",
        rect.height() * 0.056,
        NES_RED,
    );
    plate(
        &painter,
        r(0.465, 0.79, 0.275, 0.195),
        7.0 * s,
        2.0 * s,
        rgb(139, 142, 149),
        NES_LIGHT,
        stroke(2.0, rgb(224, 226, 230)),
    );
    let select = r(0.405, 0.79, 0.080, 0.075);
    let start = r(0.535, 0.79, 0.080, 0.075);
    for button in [select, start] {
        plate(
            &painter,
            button,
            11.0 * s,
            2.0 * s,
            rgb(11, 12, 14),
            rgb(61, 63, 68),
            stroke(1.0, rgb(29, 30, 33)),
        );
        // Top highlight (the original had a sign error that tilted this line).
        painter.line_segment(
            [
                button.left_top() + egui::vec2(7.0 * s, 4.0 * s),
                button.right_top() + egui::vec2(-7.0 * s, 4.0 * s),
            ],
            stroke(1.0, rgb(112, 114, 119)),
        );
    }

    bold_text(
        &painter,
        p(0.765, 0.50),
        "Nintendo®",
        rect.height() * 0.083,
        NES_RED,
    );

    // ---------------------------------------------------------------- B / A
    let b = p(0.700, 0.785);
    let a = p(0.830, 0.785);
    let radius = rect.height() * 0.080;
    for center in [b, a] {
        let bezel = egui::Rect::from_center_size(center, egui::vec2(radius * 2.65, radius * 2.65));
        plate(
            &painter,
            bezel,
            5.0 * s,
            2.0 * s,
            rgb(139, 142, 149),
            NES_LIGHT,
            stroke(1.5, rgb(223, 225, 229)),
        );
        painter.rect_filled(
            bezel.shrink(5.0 * s).translate(egui::vec2(0.0, 2.5 * s)),
            egui::Rounding::same(4.0 * s),
            rgb(112, 114, 119),
        );
        // Dark rubber shadow, then a domed cap built from stacked highlights.
        painter.circle_filled(
            center + egui::vec2(0.0, 3.0 * s),
            radius * 1.05,
            rgb(83, 10, 18),
        );
        painter.circle_filled(center, radius * 0.99, rgb(139, 17, 29));
        painter.circle_filled(
            center - egui::vec2(0.0, radius * 0.07),
            radius * 0.91,
            rgb(190, 27, 39),
        );
        painter.circle_filled(
            center - egui::vec2(radius * 0.20, radius * 0.30),
            radius * 0.58,
            rgb(215, 48, 57),
        );
        painter.circle_filled(
            center - egui::vec2(radius * 0.29, radius * 0.39),
            radius * 0.20,
            rgb(244, 111, 115),
        );
        painter.circle_stroke(center, radius * 0.98, stroke(1.2, rgb(116, 12, 23)));
    }
    // Legends sit fully inside the face plate (they used to straddle its edge).
    bold_text(
        &painter,
        p(0.700, 0.912),
        "B",
        rect.height() * 0.058,
        NES_RED,
    );
    bold_text(
        &painter,
        p(0.830, 0.912),
        "A",
        rect.height() * 0.058,
        NES_RED,
    );

    // ----------------------------------------------------------- interaction
    // (action index, hit area, glow shape). Indexes follow `key_bindings::ACTIONS`.
    let targets: Vec<(usize, egui::Rect, Glow)> = vec![
        (0, up.expand(2.0 * s), Glow::Rect(up, 3.0 * s)),
        (1, down.expand(2.0 * s), Glow::Rect(down, 3.0 * s)),
        (2, left.expand(2.0 * s), Glow::Rect(left, 3.0 * s)),
        (3, right.expand(2.0 * s), Glow::Rect(right, 3.0 * s)),
        (
            9,
            select.expand2(egui::vec2(4.0 * s, 6.0 * s)),
            Glow::Rect(select, 11.0 * s),
        ),
        (
            8,
            start.expand2(egui::vec2(4.0 * s, 6.0 * s)),
            Glow::Rect(start, 11.0 * s),
        ),
        (
            5,
            egui::Rect::from_center_size(b, egui::vec2(radius * 2.4, radius * 2.4)),
            Glow::Circle(b, radius * 0.94),
        ),
        (
            4,
            egui::Rect::from_center_size(a, egui::vec2(radius * 2.4, radius * 2.4)),
            Glow::Circle(a, radius * 0.94),
        ),
    ];
    let mut clicked = None;
    let mut hovered = None;
    for (index, target, _) in &targets {
        let response = ui
            .interact(
                *target,
                egui::Id::new(("nes-controller-hit-target", *index)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.hovered() {
            hovered = Some(*index);
        }
        if response.clicked() {
            clicked = Some(*index);
        }
        let (fr, en) = key_bindings::ACTIONS[*index];
        let name = if french { fr } else { en };
        let tip = match labels.get(*index).filter(|label| !label.is_empty()) {
            Some(key) => format!("{name} — {key}"),
            None => name.to_owned(),
        };
        response.on_hover_text(tip);
    }

    // Hover tint, then the selection glow (which pulses while waiting for a key).
    let wave = if capturing {
        ui.ctx().request_repaint();
        ((ui.input(|input| input.time) as f32) * 5.0).sin() * 0.5 + 0.5
    } else {
        1.0
    };
    let selected_fill =
        palette
            .accent
            .linear_multiply(if capturing { 0.16 + 0.26 * wave } else { 0.32 });
    let selected_stroke = stroke(
        2.5,
        palette
            .accent
            .linear_multiply(if capturing { 0.55 + 0.45 * wave } else { 1.0 }),
    );
    for (index, _, glow) in &targets {
        if *index == selected_action {
            glow.paint(&painter, selected_fill, selected_stroke);
        } else if hovered == Some(*index) {
            glow.paint(
                &painter,
                palette.accent.linear_multiply(0.14),
                stroke(1.5, palette.accent.linear_multiply(0.6)),
            );
        }
    }

    // Assigned keys, printed on top of the buttons so the mapping is readable
    // straight from the controller.
    for (index, center) in [(5usize, b), (4, a)] {
        let text = key_label(index);
        if text.is_empty() {
            continue;
        }
        let n = text.chars().count().max(2) as f32;
        let font = egui::FontId::monospace((radius * 2.4 / n).min(radius * 0.62));
        let at = center + egui::vec2(0.0, radius * 0.10);
        painter.text(
            at + egui::vec2(0.0, 1.0 * s),
            egui::Align2::CENTER_CENTER,
            &text,
            font.clone(),
            egui::Color32::from_rgba_unmultiplied(70, 0, 8, 170),
        );
        painter.text(
            at,
            egui::Align2::CENTER_CENTER,
            &text,
            font,
            egui::Color32::WHITE,
        );
    }
    for (index, button) in [(9usize, select), (8, start)] {
        let text = key_label(index);
        if text.is_empty() {
            continue;
        }
        let n = text.chars().count().max(2) as f32;
        let size = (button.width() * 0.8 / (n * 0.62)).min(button.height() * 0.5);
        painter.text(
            button.center() + egui::vec2(0.0, 1.5 * s),
            egui::Align2::CENTER_CENTER,
            &text,
            egui::FontId::monospace(size),
            rgb(205, 207, 212),
        );
    }
    clicked
}

impl DesktopApp {
    pub(super) fn show_controls(&mut self, ui: &mut egui::Ui) {
        let palette = theme::Palette::new(self.dark_mode, self.accent_color);
        let french = self.language == "fr";
        let installed = self.installed_module_ids();

        theme::page_header(
            ui,
            &palette,
            if french { "Commandes" } else { "Controls" },
            if french {
                "Une configuration clavier par console."
            } else {
                "One keyboard layout per console."
            },
            |_ui| {},
        );

        if installed.is_empty() {
            theme::card(ui, &palette, |ui| {
                ui.set_width(ui.available_width());
                ui.label(if french {
                    "Installez un module depuis Consoles et modules pour configurer ses commandes."
                } else {
                    "Install a module from Systems to configure its controls."
                });
            });
            return;
        }

        // The selected console is kept in egui's memory, so the application
        // state does not need a new field.
        let selection_id = egui::Id::new("controls-selected-module");
        let stored: Option<String> = ui.data_mut(|data| data.get_temp(selection_id));
        let mut selected = match stored {
            Some(id) if installed.contains(&id) => id,
            _ => installed.iter().next().cloned().unwrap_or_default(),
        };

        // ------------------------------------------------------ console tabs
        let mut next_selected: Option<String> = None;
        ui.horizontal_wrapped(|ui| {
            for module_id in &installed {
                let is_selected = *module_id == selected;
                let text = egui::RichText::new(module_display_name(module_id))
                    .size(14.0)
                    .color(if is_selected {
                        palette.text
                    } else {
                        palette.text_dim
                    });
                let button = egui::Button::new(text)
                    .fill(if is_selected {
                        palette.border
                    } else {
                        egui::Color32::TRANSPARENT
                    })
                    .stroke(egui::Stroke::NONE)
                    .rounding(egui::Rounding::same(8.0))
                    .min_size(egui::vec2(0.0, 32.0));
                if ui
                    .add(button)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                    && !is_selected
                {
                    next_selected = Some(module_id.clone());
                }
            }
        });
        if let Some(next) = next_selected {
            // Leaving a console cancels any pending key capture.
            self.binding_capture = None;
            ui.data_mut(|data| data.insert_temp(selection_id, next.clone()));
            selected = next;
        }

        if !self.key_bindings.contains_key(&selected) {
            let bindings = key_bindings::KeyBindings::load(self.config_path.as_deref(), &selected);
            self.key_bindings.insert(selected.clone(), bindings);
        }
        let labels: Vec<String> = match self.key_bindings.get(&selected) {
            Some(bindings) => (0..key_bindings::ACTIONS.len())
                .map(|index| bindings.display_label(index, french))
                .collect(),
            None => Vec::new(),
        };
        let capturing: Option<usize> = self
            .binding_capture
            .as_ref()
            .and_then(|(id, index)| (id == &selected).then_some(*index));

        let action_id = egui::Id::new(("controls-selected-action", selected.as_str()));
        let mut selected_action: usize = ui.data_mut(|data| data.get_temp(action_id)).unwrap_or(0);
        if selected == "nes" && ![0, 1, 2, 3, 4, 5, 8, 9].contains(&selected_action) {
            selected_action = 0;
        }

        // ------------------------------------------------------------- card
        let mut clicked: Option<usize> = None;
        let mut reset_clicked = false;
        let mut cancel_clicked = false;
        ui.add_space(8.0);
        theme::card(ui, &palette, |ui| {
            ui.set_width(ui.available_width());
            if selected == "nes" {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(if french {
                            "Manette NES — Joueur 1"
                        } else {
                            "NES controller — Player 1"
                        })
                        .strong()
                        .size(16.0)
                        .color(palette.text),
                    );
                    ui.add_space(10.0);

                    if let Some(index) = draw_nes_controller(
                        ui,
                        &palette,
                        selected_action,
                        capturing == Some(selected_action),
                        french,
                        &labels,
                    ) {
                        selected_action = index;
                        clicked = Some(index);
                    }

                    ui.add_space(14.0);
                    ui.label(
                        egui::RichText::new(if french {
                            "Affectation des touches"
                        } else {
                            "Keyboard mapping"
                        })
                        .strong()
                        .color(palette.text_dim),
                    );
                    ui.add_space(8.0);

                    // Fixed-width tiles in the physical order of the pad
                    // (D-pad, then Select / Start, then B / A), two rows of
                    // four. The old `Grid` + `vertical_centered` made the
                    // first cell swallow the whole width, which pushed the
                    // columns apart and clipped the first one.
                    const MAPPING_ORDER: [usize; 8] = [0, 1, 2, 3, 9, 8, 5, 4];
                    let columns = 4usize;
                    let gap = 12.0_f32;
                    let tile_w = ((ui.available_width() - gap * (columns as f32 - 1.0))
                        / columns as f32)
                        .clamp(110.0, 170.0);
                    let row_w = tile_w * columns as f32 + gap * (columns as f32 - 1.0);
                    let pad = ((ui.available_width() - row_w) / 2.0).max(0.0);
                    for row in MAPPING_ORDER.chunks(columns) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            ui.add_space(pad);
                            for &index in row {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(tile_w, 0.0),
                                    egui::Layout::top_down(egui::Align::Center),
                                    |ui| {
                                        let (fr, en) = key_bindings::ACTIONS[index];
                                        if ui
                                            .selectable_label(
                                                selected_action == index,
                                                if french { fr } else { en },
                                            )
                                            .clicked()
                                        {
                                            selected_action = index;
                                        }
                                        let is_capturing = capturing == Some(index);
                                        let text = if is_capturing {
                                            if french {
                                                "Appuyez…".to_owned()
                                            } else {
                                                "Press…".to_owned()
                                            }
                                        } else {
                                            labels.get(index).cloned().unwrap_or_default()
                                        };
                                        if keycap_sized(ui, &palette, &text, is_capturing, tile_w)
                                            .clicked()
                                        {
                                            selected_action = index;
                                            clicked = Some(index);
                                        }
                                    },
                                );
                            }
                        });
                        ui.add_space(8.0);
                    }
                });
            } else {
                for (group_index, (fr_title, en_title, indexes)) in GROUPS.iter().enumerate() {
                    if group_index > 0 {
                        ui.add_space(6.0);
                        ui.separator();
                    }
                    ui.label(
                        egui::RichText::new(if french { *fr_title } else { *en_title })
                            .size(12.0)
                            .strong()
                            .color(palette.text_dim),
                    );
                    for &index in indexes.iter() {
                        let (fr, en) = key_bindings::ACTIONS[index];
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(if french { fr } else { en }).size(14.0));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let is_capturing = capturing == Some(index);
                                    let text: String = if is_capturing {
                                        (if french {
                                            "Appuyez sur une touche…"
                                        } else {
                                            "Press a key…"
                                        })
                                        .to_owned()
                                    } else {
                                        labels.get(index).cloned().unwrap_or_default()
                                    };
                                    if keycap(ui, &palette, &text, is_capturing).clicked() {
                                        clicked = Some(index);
                                    }
                                },
                            );
                        });
                    }
                }
            }
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .button(if french {
                        "Réinitialiser"
                    } else {
                        "Restore defaults"
                    })
                    .clicked()
                {
                    reset_clicked = true;
                }
                if capturing.is_some()
                    && ui
                        .button(if french { "Annuler" } else { "Cancel" })
                        .clicked()
                {
                    cancel_clicked = true;
                }
            });
        });
        if selected == "nes" {
            ui.data_mut(|data| data.insert_temp(action_id, selected_action));
        }
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(if french {
                "Cliquez directement sur un bouton de la manette ou sur une touche, puis appuyez sur la nouvelle. Échap annule. Si la touche est déjà utilisée, les deux actions échangent leurs touches."
            } else {
                "Click a controller button or a key, then press the new one. Esc cancels. If the key is already used, the two actions swap keys."
            })
            .size(13.0)
            .color(palette.text_dim),
        );

        // ------------------------------------------------------ apply input
        if let Some(index) = clicked {
            self.binding_capture = Some((selected.clone(), index));
        }
        if cancel_clicked {
            self.binding_capture = None;
        }
        if reset_clicked {
            if let Some(bindings) = self.key_bindings.get_mut(&selected) {
                bindings.reset();
            }
            self.binding_capture = None;
            self.persist_key_bindings(&selected);
        }

        let mut captured = None;
        if let Some((module_id, index)) = self.binding_capture.clone() {
            for event in ui.input(|input| input.events.clone()) {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    ..
                } = event
                {
                    captured = Some((module_id.clone(), index, key));
                    break;
                }
            }
        }

        if let Some((module_id, index, key)) = captured {
            self.binding_capture = None;
            if key != egui::Key::Escape {
                let result = self
                    .key_bindings
                    .get_mut(&module_id)
                    .ok_or(usize::MAX)
                    .and_then(|bindings| bindings.assign_or_swap(index, key));
                match result {
                    Ok(swapped) => {
                        self.persist_key_bindings(&module_id);
                        if let Some(other) = swapped {
                            self.message = if french {
                                format!(
                                    "Touches échangées avec « {} ».",
                                    key_bindings::ACTIONS[other].0
                                )
                            } else {
                                format!("Keys swapped with {}.", key_bindings::ACTIONS[other].1)
                            };
                        }
                    }
                    Err(_) => {
                        self.message = if french {
                            "Cette touche n’est pas prise en charge.".to_owned()
                        } else {
                            "That key is not supported.".to_owned()
                        };
                    }
                }
            }
        }
        ui.add_space(16.0);
    }
}
