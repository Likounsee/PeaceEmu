//! library page for the PeaceEmu desktop frontend.

use super::*;
use host_app::library::{ContentAvailability, ContentFormatDetection, ContentValidation};

#[cfg(test)]
fn content_path_status(path: &Path) -> &'static str {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => "File exists · launch validation pending",
        Ok(_) => "Unavailable · not a regular file",
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            "Missing · re-import required"
        }
        Err(_) => "Unavailable · cannot read file metadata",
    }
}

/// Flat cover colour derived from the title, so each game keeps a stable tile.
fn cover_color(title: &str) -> egui::Color32 {
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
    egui::Color32::from_rgb(r, g, b)
}

fn initials(title: &str) -> String {
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

/// Matches every search term against either the display title or the full path.
/// Requiring all terms makes multi-word searches useful without requiring an exact phrase.
fn matches_library_query(title: &str, path: &Path, query: &str) -> bool {
    let title = title.to_lowercase();
    let path = path.to_string_lossy().to_lowercase();
    let query = query.to_lowercase();
    query
        .split_whitespace()
        .all(|term| title.contains(term) || path.contains(term))
}

/// Display cover art in either a 16:9 banner or a square logo tile.
fn cover_target_aspect(texture: Option<&egui::TextureHandle>) -> f32 {
    texture
        .map(|texture| {
            let size = texture.size_vec2();
            if size.y > 0.0 && size.x / size.y >= 1.25 {
                16.0 / 9.0
            } else {
                1.0
            }
        })
        .unwrap_or(16.0 / 9.0)
}

/// Center-crop the source UVs to fill the target shape without stretching the image.
fn cover_uv(texture: &egui::TextureHandle, target_aspect: f32) -> egui::Rect {
    let size = texture.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 || target_aspect <= 0.0 {
        return egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    }
    let source_aspect = size.x / size.y;
    if source_aspect > target_aspect {
        let visible_width = target_aspect / source_aspect;
        let margin = (1.0 - visible_width) * 0.5;
        egui::Rect::from_min_max(egui::pos2(margin, 0.0), egui::pos2(1.0 - margin, 1.0))
    } else {
        let visible_height = source_aspect / target_aspect;
        let margin = (1.0 - visible_height) * 0.5;
        egui::Rect::from_min_max(egui::pos2(0.0, margin), egui::pos2(1.0, 1.0 - margin))
    }
}

fn shorten(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut shortened: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

fn availability_label(availability: ContentAvailability, french: bool) -> &'static str {
    match (availability, french) {
        // A filesystem check proves that a regular file exists, not that its
        // contents are a valid ROM or that an installed core can launch it.
        (ContentAvailability::Available, true) => "Fichier trouvé · validation au lancement",
        (ContentAvailability::Available, false) => "File found · validation pending at launch",
        (ContentAvailability::Missing, true) => "Introuvable · à réimporter",
        (ContentAvailability::Missing, false) => "Missing · re-import required",
        (ContentAvailability::NotRegularFile, true) => "Indisponible · ce n’est pas un fichier",
        (ContentAvailability::NotRegularFile, false) => "Unavailable · not a regular file",
        (ContentAvailability::Unavailable, true) => "Indisponible · lecture impossible",
        (ContentAvailability::Unavailable, false) => "Unavailable · cannot inspect file",
    }
}

fn card_availability_label(availability: ContentAvailability, french: bool) -> &'static str {
    match (availability, french) {
        (ContentAvailability::Available, true) => "Fichier trouvé",
        (ContentAvailability::Available, false) => "File found",
        (ContentAvailability::Missing, true) => "Introuvable",
        (ContentAvailability::Missing, false) => "Missing",
        (ContentAvailability::NotRegularFile, true) => "Pas un fichier",
        (ContentAvailability::NotRegularFile, false) => "Not a file",
        (ContentAvailability::Unavailable, true) => "Lecture impossible",
        (ContentAvailability::Unavailable, false) => "Cannot inspect",
    }
}

fn path_availability(path: &Path) -> ContentAvailability {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => ContentAvailability::Available,
        Ok(_) => ContentAvailability::NotRegularFile,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ContentAvailability::Missing,
        Err(_) => ContentAvailability::Unavailable,
    }
}

/// Return the current size only when the path is still a regular file.
/// Indexed sizes become stale when a ROM is replaced or removed while the app is open.
pub(super) fn live_path_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
}

pub(super) fn display_path_size(size: Option<u64>, french: bool) -> String {
    size.map(format_size).unwrap_or_else(|| {
        if french {
            "Taille indisponible".to_owned()
        } else {
            "Size unavailable".to_owned()
        }
    })
}

fn detail_row(ui: &mut egui::Ui, palette: &theme::Palette, label: &str, value: &str) {
    ui.label(
        egui::RichText::new(label)
            .size(12.0)
            .color(palette.text_dim),
    );
    ui.label(egui::RichText::new(value).size(13.0));
}

impl DesktopApp {
    fn cover_texture(
        &mut self,
        context: &egui::Context,
        game_path: &Path,
    ) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.cover_textures.get(game_path) {
            return Some(texture.clone());
        }
        let image_path = self.cover_paths.get(game_path)?.clone();
        let bytes = std::fs::read(image_path).ok()?;
        let rgba = image::load_from_memory(&bytes).ok()?.into_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        let texture = context.load_texture(
            format!("game-cover:{}", game_path.display()),
            color_image,
            egui::TextureOptions::LINEAR,
        );
        self.cover_textures
            .insert(game_path.to_path_buf(), texture.clone());
        Some(texture)
    }

    fn set_game_cover(&mut self, game_path: &Path, image_path: PathBuf, source: &str) {
        let Some(config_path) = self.config_path.clone() else {
            self.message = if self.language == "fr" {
                "Impossible d’enregistrer la jaquette : dossier de configuration indisponible."
                    .to_owned()
            } else {
                "Could not save cover: configuration directory is unavailable.".to_owned()
            };
            return;
        };
        let old = self.cover_paths.insert(game_path.to_path_buf(), image_path);
        match game_covers::persist(&config_path, &self.cover_paths) {
            Ok(()) => {
                self.cover_textures.remove(game_path);
                self.message = if self.language == "fr" {
                    format!("Jaquette enregistrée ({source}).")
                } else {
                    format!("Cover saved ({source}).")
                };
            }
            Err(error) => {
                if let Some(previous) = old {
                    self.cover_paths.insert(game_path.to_path_buf(), previous);
                } else {
                    self.cover_paths.remove(game_path);
                }
                self.message = if self.language == "fr" {
                    format!("Impossible d’enregistrer la jaquette : {error}")
                } else {
                    format!("Could not save cover: {error}")
                };
            }
        }
    }

    fn save_game_display_name(&mut self, game_path: &Path) {
        let name = self.display_name_edit.trim().to_owned();
        if name.chars().count() > 120 {
            self.message = if self.language == "fr" {
                "Le nom affiché doit contenir au maximum 120 caractères.".to_owned()
            } else {
                "Display names must be 120 characters or fewer.".to_owned()
            };
            return;
        }
        let Some(config_path) = self.config_path.clone() else {
            self.message = if self.language == "fr" {
                "Impossible d’enregistrer le nom : dossier de configuration indisponible."
                    .to_owned()
            } else {
                "Could not save name: configuration directory is unavailable.".to_owned()
            };
            return;
        };
        let previous = self.game_names.get(game_path).cloned();
        if name.is_empty() {
            self.game_names.remove(game_path);
        } else {
            self.game_names.insert(game_path.to_path_buf(), name);
        }
        match game_names::persist(&config_path, &self.game_names) {
            Ok(()) => {
                self.message = if self.language == "fr" {
                    "Nom du jeu enregistré.".to_owned()
                } else {
                    "Game display name saved.".to_owned()
                };
            }
            Err(error) => {
                if let Some(previous) = previous {
                    self.game_names.insert(game_path.to_path_buf(), previous);
                } else {
                    self.game_names.remove(game_path);
                }
                self.message = if self.language == "fr" {
                    format!("Impossible d’enregistrer le nom : {error}")
                } else {
                    format!("Could not save name: {error}")
                };
            }
        }
    }

    fn reset_game_display_name(&mut self, game_path: &Path) {
        let Some(config_path) = self.config_path.clone() else {
            return;
        };
        let original_title = self
            .controller
            .library()
            .entries()
            .find(|entry| entry.path() == game_path)
            .map(|entry| entry.title().to_owned())
            .unwrap_or_default();
        let previous = self.game_names.remove(game_path);
        match game_names::persist(&config_path, &self.game_names) {
            Ok(()) => {
                self.display_name_edit = original_title;
                self.message = if self.language == "fr" {
                    "Nom d’origine rétabli.".to_owned()
                } else {
                    "Original name restored.".to_owned()
                };
            }
            Err(error) => {
                if let Some(previous) = previous {
                    self.game_names.insert(game_path.to_path_buf(), previous);
                }
                self.message = if self.language == "fr" {
                    format!("Impossible de rétablir le nom : {error}")
                } else {
                    format!("Could not reset name: {error}")
                };
            }
        }
    }

    fn remove_game_cover(&mut self, game_path: &Path) {
        let Some(config_path) = self.config_path.clone() else {
            return;
        };
        let old = self.cover_paths.remove(game_path);
        match game_covers::persist(&config_path, &self.cover_paths) {
            Ok(()) => {
                self.cover_textures.remove(game_path);
                self.message = if self.language == "fr" {
                    "Jaquette supprimée.".to_owned()
                } else {
                    "Cover removed.".to_owned()
                };
            }
            Err(error) => {
                if let Some(previous) = old {
                    self.cover_paths.insert(game_path.to_path_buf(), previous);
                }
                self.message = if self.language == "fr" {
                    format!("Impossible de supprimer la jaquette : {error}")
                } else {
                    format!("Could not remove cover: {error}")
                };
            }
        }
    }

    pub(super) fn show_library(&mut self, ui: &mut egui::Ui) {
        let palette = theme::Palette::new(self.dark_mode, self.accent_color);
        let french = self.language == "fr";
        let total = self.controller.library().len();
        let total_size: u64 = self
            .controller
            .library()
            .entries()
            .filter_map(|entry| live_path_size(entry.path()))
            .sum();

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.library_page.001"))
                        .size(30.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(if french {
                        format!("{total} jeu(x) · {}", format_size(total_size))
                    } else {
                        format!("{total} game(s) · {}", format_size(total_size))
                    })
                    .color(palette.text_dim),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui
                    .add(theme::primary_button(
                        &palette,
                        self.language_pack.text("ui.library_page.002"),
                    ))
                    .clicked()
                {
                    self.choose_content_file();
                }
            });
        });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);

        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.library_filter)
                    .hint_text(self.language_pack.text("ui.library_page.003"))
                    .desired_width(ui.available_width().min(260.0)),
            );
            egui::ComboBox::from_id_source("library_format_filter")
                .selected_text(self.library_format.label(french))
                .show_ui(ui, |ui| {
                    for format in [LibraryFormat::All, LibraryFormat::GameBoy] {
                        ui.selectable_value(&mut self.library_format, format, format.label(french));
                    }
                });
            egui::ComboBox::from_id_source("library_sort")
                .selected_text(self.library_sort.label(french))
                .show_ui(ui, |ui| {
                    for sort in [LibrarySort::Largest, LibrarySort::Path] {
                        ui.selectable_value(&mut self.library_sort, sort, sort.label(french));
                    }
                });
            if ui
                .button(self.language_pack.text("ui.library_page.004"))
                .clicked()
            {
                self.library_filter.clear();
                self.library_format = LibraryFormat::All;
            }
        });

        let query = self.library_filter.trim().to_lowercase();
        let mut entries: Vec<_> = self
            .controller
            .library()
            .entries()
            .map(|entry| {
                let path = entry.path().to_path_buf();
                let title = self
                    .game_names
                    .get(&path)
                    .cloned()
                    .unwrap_or_else(|| entry.title().to_owned());
                (title, path.clone(), live_path_size(&path))
            })
            .filter(|(title, path, _)| {
                matches_library_query(title, path, &query) && self.library_format.matches(path)
            })
            .collect();
        match self.library_sort {
            LibrarySort::Largest => entries.sort_by(|a, b| {
                b.2.cmp(&a.2)
                    .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
            }),
            LibrarySort::Path => entries.sort_by(|a, b| a.1.cmp(&b.1)),
        }

        ui.add_space(10.0);
        let show_detail = !entries.is_empty();
        let wide = show_detail && ui.available_width() >= 760.0;
        let detail_width = 300.0;
        let grid_width = if wide {
            ui.available_width() - detail_width - 28.0
        } else {
            ui.available_width()
        };
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(grid_width);
                self.show_library_grid(ui, &palette, &entries, total, french);
            });
            if wide {
                ui.add_space(18.0);
                ui.vertical(|ui| {
                    ui.set_width(detail_width);
                    self.show_library_detail(ui, &palette, french);
                });
            }
        });
        if show_detail && !wide {
            ui.add_space(12.0);
            self.show_library_detail(ui, &palette, french);
        }
    }

    fn show_library_grid(
        &mut self,
        ui: &mut egui::Ui,
        palette: &theme::Palette,
        entries: &[(String, PathBuf, Option<u64>)],
        total: usize,
        french: bool,
    ) {
        if entries.is_empty() {
            let (heading, hint) = match (total == 0, french) {
                (true, true) => (
                    "Votre bibliothèque est vide",
                    "Importez des jeux Game Boy (.gb), Game Boy Color (.gbc) ou Game Boy Advance (.gba) pour constituer votre collection.",
                ),
                (true, false) => (
                    "Your library is empty",
                    "Import Game Boy (.gb), Game Boy Color (.gbc), or Game Boy Advance (.gba) games to build your collection.",
                ),
                (false, true) => (
                    "Aucun jeu ne correspond",
                    "Essayez une autre recherche, changez le filtre de format ou effacez la recherche.",
                ),
                (false, false) => (
                    "No matching games",
                    "Try another search, change the format filter, or clear the search.",
                ),
            };
            theme::card(ui, palette, |ui| {
                ui.label(egui::RichText::new(heading).size(18.0).strong());
                ui.label(egui::RichText::new(hint).color(palette.text_dim));
                if total == 0
                    && ui
                        .add(theme::primary_button(
                            palette,
                            self.language_pack.text("ui.library_page.005"),
                        ))
                        .clicked()
                {
                    self.choose_content_file();
                }
            });
            return;
        }

        let spacing = 12.0;
        let available = ui.available_width();
        let columns = (((available + spacing) / (156.0 + spacing)).floor() as usize).max(1);
        let card_width = ((available - spacing * (columns as f32 - 1.0)) / columns as f32 - 0.5)
            .floor()
            .max(110.0);
        ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);
        ui.horizontal_wrapped(|ui| {
            for (title, path, size) in entries {
                let selected = self.selected_content.as_ref() == Some(path);
                // Re-check the filesystem instead of trusting the library scan:
                // files can be moved or removed while the library screen stays open.
                let availability = path_availability(path);
                let file_available = availability == ContentAvailability::Available;
                let cover_texture = self.cover_texture(ui.ctx(), path);
                let cover_aspect = cover_target_aspect(cover_texture.as_ref());
                let cover_height = card_width / cover_aspect;
                let card_height = cover_height + 72.0;
                let (rect, response) = ui
                    .allocate_exact_size(egui::vec2(card_width, card_height), egui::Sense::click());
                let painter = ui.painter();
                painter.rect_filled(
                    rect,
                    egui::Rounding::same(12.0),
                    if selected {
                        palette.accent_subtle
                    } else {
                        palette.sidebar
                    },
                );
                let cover =
                    egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), cover_height));
                if let Some(texture) = &cover_texture {
                    painter.with_clip_rect(cover.intersect(rect)).image(
                        texture.id(),
                        cover,
                        cover_uv(texture, cover_aspect),
                        egui::Color32::WHITE,
                    );
                    painter.rect_stroke(
                        cover,
                        egui::Rounding {
                            nw: 12.0,
                            ne: 12.0,
                            sw: 0.0,
                            se: 0.0,
                        },
                        egui::Stroke::new(1.0, palette.sidebar),
                    );
                } else {
                    painter.rect_filled(
                        cover,
                        egui::Rounding {
                            nw: 12.0,
                            ne: 12.0,
                            sw: 0.0,
                            se: 0.0,
                        },
                        cover_color(title),
                    );
                    painter.text(
                        cover.center(),
                        egui::Align2::CENTER_CENTER,
                        initials(title),
                        egui::FontId::proportional(28.0),
                        egui::Color32::WHITE,
                    );
                }
                let max_chars = ((card_width - 24.0) / 7.2) as usize;
                painter.text(
                    rect.min + egui::vec2(12.0, cover_height + 10.0),
                    egui::Align2::LEFT_TOP,
                    shorten(title, max_chars),
                    egui::FontId::proportional(14.0),
                    palette.text,
                );
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("?")
                    .to_ascii_uppercase();
                painter.text(
                    rect.min + egui::vec2(12.0, cover_height + 32.0),
                    egui::Align2::LEFT_TOP,
                    format!("{extension} · {}", display_path_size(*size, french)),
                    egui::FontId::proportional(12.0),
                    palette.text_dim,
                );
                if !file_available {
                    painter.text(
                        rect.min + egui::vec2(12.0, cover_height + 50.0),
                        egui::Align2::LEFT_TOP,
                        card_availability_label(availability, french),
                        egui::FontId::proportional(11.0),
                        palette.warning,
                    );
                }
                painter.rect_stroke(
                    rect,
                    egui::Rounding::same(12.0),
                    if selected {
                        egui::Stroke::new(2.0_f32, palette.accent)
                    } else {
                        egui::Stroke::new(1.0_f32, palette.border)
                    },
                );
                if selected {
                    theme::paint_check_badge(
                        painter,
                        rect.right_top() + egui::vec2(-16.0, 16.0),
                        palette,
                    );
                }
                let response = response
                    .on_hover_text(path.display().to_string())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.clicked() || response.double_clicked() {
                    self.selected_content = Some(path.clone());
                    self.content_path = path.display().to_string();
                    self.cover_query = title.clone();
                    self.display_name_edit = title.clone();
                }
                if response.double_clicked() && file_available {
                    self.launch_content();
                }
            }
        });
    }

    fn show_library_detail(&mut self, ui: &mut egui::Ui, palette: &theme::Palette, french: bool) {
        let selected = self.selected_content.clone().and_then(|path| {
            self.controller
                .library()
                .entries()
                .find(|entry| entry.path() == path)
                .map(|entry| {
                    (
                        path.clone(),
                        self.game_names
                            .get(&path)
                            .cloned()
                            .unwrap_or_else(|| entry.title().to_owned()),
                        live_path_size(&path),
                    )
                })
        });
        let Some((path, title, size)) = selected else {
            theme::card(ui, palette, |ui| {
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.library_page.006"))
                        .size(16.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.library_page.007"))
                        .color(palette.text_dim),
                );
            });
            return;
        };

        let metadata = self.controller.library().get(&path).map(|entry| {
            (
                entry.format_hint().map(str::to_owned),
                entry.compatible_system_hint().map(str::to_owned),
                entry.format_detection().clone(),
                entry.validation().clone(),
                entry.availability(),
            )
        });
        // Availability is deliberately live: cached scan results become stale if
        // the user moves, removes, or replaces a ROM while this view is open.
        let availability = path_availability(&path);
        let file_available = availability == ContentAvailability::Available;

        let detail_cover_texture = self.cover_texture(ui.ctx(), &path);
        if self.display_name_edit.is_empty() {
            self.display_name_edit = title.clone();
        }
        if self.cover_query.is_empty() {
            self.cover_query = title.clone();
        }
        theme::card(ui, palette, |ui| {
            let available_width = ui.available_width();
            let cover_aspect = cover_target_aspect(detail_cover_texture.as_ref());
            let cover_width = if cover_aspect == 1.0 {
                available_width.min(260.0)
            } else {
                available_width
            };
            let cover_height = cover_width / cover_aspect;
            let mut cover_rect = egui::Rect::NOTHING;
            ui.horizontal(|ui| {
                ui.add_space(((available_width - cover_width) * 0.5).max(0.0));
                let (cover, _) = ui.allocate_exact_size(
                    egui::vec2(cover_width, cover_height),
                    egui::Sense::hover(),
                );
                cover_rect = cover;
            });
            let cover = cover_rect;
            if let Some(texture) = &detail_cover_texture {
                ui.painter().with_clip_rect(cover).image(
                    texture.id(),
                    cover,
                    cover_uv(texture, cover_aspect),
                    egui::Color32::WHITE,
                );
                ui.painter().rect_stroke(
                    cover,
                    egui::Rounding::same(10.0),
                    egui::Stroke::new(1.0, palette.sidebar),
                );
            } else {
                ui.painter()
                    .rect_filled(cover, egui::Rounding::same(10.0), cover_color(&title));
                ui.painter().text(
                    cover.center(),
                    egui::Align2::CENTER_CENTER,
                    initials(&title),
                    egui::FontId::proportional(32.0),
                    egui::Color32::WHITE,
                );
            }
            ui.label(egui::RichText::new(&title).size(18.0).strong());
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(if french {
                    "Nom affiché"
                } else {
                    "Display name"
                })
                .strong(),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.display_name_edit).hint_text(if french {
                    "Nom personnalisé du jeu"
                } else {
                    "Custom game name"
                }),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(if french {
                        "Enregistrer le nom"
                    } else {
                        "Save name"
                    })
                    .clicked()
                {
                    self.save_game_display_name(&path);
                }
                if self.game_names.contains_key(&path)
                    && ui
                        .button(if french { "Rétablir" } else { "Reset" })
                        .clicked()
                {
                    self.reset_game_display_name(&path);
                }
            });
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("?")
                .to_ascii_uppercase();
            ui.label(
                egui::RichText::new(format!("{extension} · {}", display_path_size(size, french)))
                    .color(palette.text_dim),
            );
            theme::badge(
                ui,
                availability_label(availability, french),
                if file_available {
                    palette.success
                } else {
                    palette.warning
                },
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(if french {
                    "Jaquette du jeu"
                } else {
                    "Game cover"
                })
                .strong(),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.cover_query).hint_text(if french {
                    "Titre à rechercher en ligne"
                } else {
                    "Title to search online"
                }),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(if french {
                        "Image locale…"
                    } else {
                        "Choose local image…"
                    })
                    .clicked()
                {
                    if let Some(source) = rfd::FileDialog::new()
                        .add_filter("Cover images", &["png", "jpg", "jpeg", "webp"])
                        .pick_file()
                    {
                        let result = self
                            .config_path
                            .as_deref()
                            .ok_or_else(|| "Configuration directory unavailable".to_owned())
                            .and_then(|config| game_covers::import_local(config, &source));
                        match result {
                            Ok(saved) => self.set_game_cover(&path, saved, "local"),
                            Err(error) => {
                                self.message = if french {
                                    format!("Import de jaquette impossible : {error}")
                                } else {
                                    format!("Could not import cover: {error}")
                                }
                            }
                        }
                    }
                }
                if ui
                    .button(if french {
                        "Chercher en ligne"
                    } else {
                        "Search online"
                    })
                    .clicked()
                {
                    let query = if self.cover_query.trim().is_empty() {
                        title.clone()
                    } else {
                        self.cover_query.trim().to_owned()
                    };
                    let result = self
                        .config_path
                        .as_deref()
                        .ok_or_else(|| "Configuration directory unavailable".to_owned())
                        .and_then(|config| game_covers::search_online(config, &query));
                    match result {
                        Ok((saved, source)) => self.set_game_cover(&path, saved, &source),
                        Err(error) => {
                            self.message = if french {
                                format!("Recherche de jaquette impossible : {error}")
                            } else {
                                format!("Cover search failed: {error}")
                            }
                        }
                    }
                }
                if self.cover_paths.contains_key(&path)
                    && ui
                        .button(if french {
                            "Supprimer la jaquette"
                        } else {
                            "Remove cover"
                        })
                        .clicked()
                {
                    self.remove_game_cover(&path);
                }
            });
            ui.add_space(4.0);

            let launch = ui
                .add_enabled_ui(file_available, |ui| {
                    ui.add_sized(
                        egui::vec2(ui.available_width(), 36.0),
                        theme::primary_button(
                            palette,
                            self.language_pack.text("ui.library_page.008"),
                        ),
                    )
                })
                .inner;
            if launch.clicked() {
                self.content_path = path.display().to_string();
                self.launch_content();
            }
            let forget = ui.add_sized(
                egui::vec2(ui.available_width(), 36.0),
                egui::Button::new(self.language_pack.text("ui.library_page.009")),
            );
            if forget.clicked() {
                match self.controller.forget_content(&path) {
                    Ok(_) => {
                        if self.selected_content.as_ref() == Some(&path) {
                            self.selected_content = None;
                            // Do not leave a forgotten library item in the launch-path field.
                            self.content_path.clear();
                        }
                        let removal_message = if french {
                            format!("« {} » a été retiré de la bibliothèque.", title)
                        } else {
                            format!("Removed '{}' from the library.", title)
                        };
                        self.message = removal_message.clone();
                        if !self.save_preferences() {
                            let save_error = std::mem::take(&mut self.message);
                            self.message = format!(
                                "{removal_message} The item is removed for this session, but the library change was not saved. {save_error}"
                            );
                        }
                    }
                    Err(error) => {
                        self.message = if french {
                            format!("Impossible d’oublier ce contenu : {error}")
                        } else {
                            format!("Could not forget content: {error}")
                        }
                    }
                }
            }

            ui.add_space(4.0);
            egui::CollapsingHeader::new(
                egui::RichText::new(self.language_pack.text("ui.library_page.010")).strong(),
            )
            .default_open(false)
            .show(ui, |ui| {
                detail_row(
                    ui,
                    palette,
                    self.language_pack.text("ui.library_page.011"),
                    &path.display().to_string(),
                );
                if let Some((format_hint, system_hint, detection, validation, _)) = &metadata {
                    detail_row(
                        ui,
                        palette,
                        self.language_pack.text("ui.library_page.012"),
                        format_hint
                            .as_deref()
                            .unwrap_or(self.language_pack.text("ui.library_page.013")),
                    );
                    detail_row(
                        ui,
                        palette,
                        self.language_pack.text("ui.library_page.014"),
                        system_hint
                            .as_deref()
                            .unwrap_or(self.language_pack.text("ui.library_page.015")),
                    );
                    let detected = match detection {
                        ContentFormatDetection::Pending => {
                            (self.language_pack.text("ui.library_page.016")).to_owned()
                        }
                        ContentFormatDetection::NoMatch => {
                            (self.language_pack.text("ui.library_page.017")).to_owned()
                        }
                        ContentFormatDetection::Detected(format) => format.to_string(),
                        ContentFormatDetection::Ambiguous(formats) => format!(
                            "{} {}",
                            self.language_pack.text("ui.library_page.018"),
                            formats.join(", ")
                        ),
                        ContentFormatDetection::NeedsMoreData => {
                            (self.language_pack.text("ui.library_page.019")).to_owned()
                        }
                    };
                    detail_row(
                        ui,
                        palette,
                        self.language_pack.text("ui.library_page.020"),
                        &detected,
                    );
                    let validation_text = match validation {
                        ContentValidation::Pending => {
                            (self.language_pack.text("ui.library_page.021")).to_owned()
                        }
                        ContentValidation::Accepted => {
                            (self.language_pack.text("ui.library_page.022")).to_owned()
                        }
                        ContentValidation::Rejected(detail) => format!(
                            "{} {}",
                            self.language_pack.text("ui.library_page.023"),
                            detail
                        ),
                    };
                    detail_row(ui, palette, "Validation", &validation_text);
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        card_availability_label, content_path_status, display_path_size, live_path_size,
        path_availability,
    };
    use host_app::library::ContentAvailability;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "desktop-library-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn card_status_distinguishes_missing_files_from_other_unavailable_paths() {
        assert_eq!(
            card_availability_label(ContentAvailability::Missing, true),
            "Introuvable"
        );
        assert_eq!(
            card_availability_label(ContentAvailability::NotRegularFile, false),
            "Not a file"
        );
        assert_eq!(
            card_availability_label(ContentAvailability::Unavailable, false),
            "Cannot inspect"
        );
        assert_eq!(
            card_availability_label(ContentAvailability::Available, false),
            "File found"
        );
    }

    #[test]
    fn detail_status_labels_are_localized_and_do_not_overstate_validation() {
        assert_eq!(
            super::availability_label(ContentAvailability::Available, true),
            "Fichier trouvé · validation au lancement"
        );
        assert_eq!(
            super::availability_label(ContentAvailability::Available, false),
            "File found · validation pending at launch"
        );
        assert_eq!(
            super::availability_label(ContentAvailability::Missing, true),
            "Introuvable · à réimporter"
        );
        assert_eq!(
            super::availability_label(ContentAvailability::Missing, false),
            "Missing · re-import required"
        );
        assert_eq!(
            super::availability_label(ContentAvailability::NotRegularFile, true),
            "Indisponible · ce n’est pas un fichier"
        );
        assert_eq!(
            super::availability_label(ContentAvailability::NotRegularFile, false),
            "Unavailable · not a regular file"
        );
        assert_eq!(
            super::availability_label(ContentAvailability::Unavailable, true),
            "Indisponible · lecture impossible"
        );
        assert_eq!(
            super::availability_label(ContentAvailability::Unavailable, false),
            "Unavailable · cannot inspect file"
        );
    }

    #[test]
    fn library_search_matches_all_terms_across_title_and_path_case_insensitively() {
        let path = Path::new("C:/Games/Adventure/zelda.gbc");
        assert!(super::matches_library_query(
            "The Legend of Zelda",
            path,
            "legend zelda"
        ));
        assert!(super::matches_library_query(
            "The Legend of Zelda",
            path,
            "ADVENTURE gbc"
        ));
        assert!(super::matches_library_query(
            "The Legend of Zelda",
            path,
            "  legend   C:/games "
        ));
        assert!(!super::matches_library_query(
            "The Legend of Zelda",
            path,
            "legend metroid"
        ));
        assert!(super::matches_library_query(
            "The Legend of Zelda",
            path,
            "   "
        ));
    }

    #[test]
    fn live_size_tracks_replaced_files_and_avoids_stale_sizes_for_missing_paths() {
        let path = unique_temp_path("live-size");
        assert_eq!(live_path_size(&path), None);
        assert_eq!(display_path_size(None, true), "Taille indisponible");
        assert_eq!(display_path_size(None, false), "Size unavailable");

        std::fs::write(&path, b"abc").expect("create temporary file");
        assert_eq!(live_path_size(&path), Some(3));
        std::fs::write(&path, b"replacement").expect("replace temporary file contents");
        assert_eq!(live_path_size(&path), Some(11));
        std::fs::remove_file(&path).expect("remove temporary file");
        assert_eq!(live_path_size(&path), None);
    }

    #[test]
    fn path_availability_classifies_files_directories_and_missing_paths() {
        let missing = unique_temp_path("availability-missing");
        assert_eq!(path_availability(&missing), ContentAvailability::Missing);

        let directory = unique_temp_path("availability-directory");
        std::fs::create_dir(&directory).expect("create temporary directory");
        assert_eq!(
            path_availability(&directory),
            ContentAvailability::NotRegularFile
        );
        std::fs::remove_dir(&directory).expect("remove temporary directory");

        let file = unique_temp_path("availability-file");
        std::fs::write(&file, b"rom").expect("create temporary file");
        assert_eq!(path_availability(&file), ContentAvailability::Available);
        std::fs::remove_file(&file).expect("remove temporary file");
    }

    #[test]
    fn missing_library_path_requests_reimport() {
        let path = unique_temp_path("missing");
        assert_eq!(content_path_status(&path), "Missing · re-import required");
    }

    #[test]
    fn directory_is_not_reported_as_launchable_content() {
        let path = unique_temp_path("directory");
        std::fs::create_dir(&path).expect("create temporary test directory");

        assert_eq!(
            content_path_status(&path),
            "Unavailable · not a regular file"
        );

        std::fs::remove_dir(&path).expect("remove temporary test directory");
    }

    #[test]
    fn existing_regular_file_requires_launch_validation() {
        let path = unique_temp_path("file");
        std::fs::write(&path, b"test").expect("create temporary test file");

        assert_eq!(
            content_path_status(&path),
            "File exists · launch validation pending"
        );

        std::fs::remove_file(&path).expect("remove temporary test file");
    }
}
