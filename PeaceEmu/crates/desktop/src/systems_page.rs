//! Unified module inventory and lifecycle overview.

use super::*;
use emulation_core::module_management::ModuleId;
use emulation_core::module_state_store::{default_user_state_path, ModuleStateStore};
use std::collections::BTreeSet;

fn module_tile(ui: &mut egui::Ui, palette: &theme::Palette, name: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(42.0, 42.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, egui::Rounding::same(9.0), palette.accent_subtle);
    let initials: String = name
        .chars()
        .filter(|character| character.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initials,
        egui::FontId::proportional(15.0),
        palette.text,
    );
}

fn status_badge(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    theme::badge(ui, text, color);
}

impl DesktopApp {
    pub(super) fn show_systems(&mut self, ui: &mut egui::Ui) {
        if let Some(receiver) = self.nes_install_receiver.take() {
            match receiver.try_recv() {
                Ok(Ok(path)) => {
                    self.nes_installing = false;
                    self.message = if self.language == "fr" {
                        format!("Module NES installé et vérifié : {}", path.display())
                    } else {
                        format!("NES module installed and verified: {}", path.display())
                    };
                }
                Ok(Err(error)) => {
                    self.nes_installing = false;
                    self.message = if self.language == "fr" {
                        format!("Échec de l’installation du module NES : {error}")
                    } else {
                        format!("NES module installation failed: {error}")
                    };
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.nes_install_receiver = Some(receiver);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.nes_installing = false;
                    self.message = if self.language == "fr" {
                        "Le téléchargement du module NES s’est interrompu.".to_owned()
                    } else {
                        "NES module download stopped unexpectedly.".to_owned()
                    };
                }
            }
        }
        let palette = theme::Palette::new(self.dark_mode, self.accent_color);
        let french = self.language == "fr";
        let registered: BTreeSet<String> = self
            .registry
            .system_ids()
            .map(|id| id.as_str().to_owned())
            .collect();

        let state_path = match default_user_state_path() {
            Ok(path) => path,
            Err(error) => {
                theme::card(ui, &palette, |ui| {
                    ui.label(
                        egui::RichText::new(if french {
                            format!("État des modules indisponible : {error}")
                        } else {
                            format!("Module state unavailable: {error}")
                        })
                        .color(palette.danger),
                    );
                });
                return;
            }
        };
        let store = ModuleStateStore::new(state_path);
        let mut state = match store.load() {
            Ok(state) => state,
            Err(error) => {
                theme::card(ui, &palette, |ui| {
                    ui.label(
                        egui::RichText::new(if french {
                            format!("Impossible de lire l’état des modules : {error}")
                        } else {
                            format!("Could not load module state: {error}")
                        })
                        .strong()
                        .color(palette.danger),
                    );
                    ui.label(self.language_pack.text("ui.systems_page.003"));
                });
                return;
            }
        };

        // One inventory combines compiled modules and persisted package records,
        // so installed modules don't disappear merely because a core isn't bundled.
        let mut module_ids = registered.clone();
        // The NES core is a separately downloadable executable, so expose it even
        // before its package has been installed or registered in the local state.
        module_ids.insert("nes".to_owned());
        module_ids.extend(
            state
                .installed_modules()
                .iter()
                .map(|module| module.id.as_str().to_owned()),
        );
        module_ids.extend(
            state
                .selected_modules()
                .iter()
                .map(|module| module.as_str().to_owned()),
        );

        let mut selected = state.selected_modules().to_vec();
        let mut changed = false;
        let installed_count = state.installed_modules().len();
        let selected_count = selected.len();

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(self.language_pack.text("ui.systems_page.001"))
                        .size(29.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(if french {
                        "Tous vos modules dans une seule liste : état, version, activation et téléchargement."
                    } else {
                        "All modules in one list: status, version, activation and downloads."
                    })
                    .color(palette.text_dim),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                theme::badge(
                    ui,
                    &format!("{} module(s)", module_ids.len()),
                    palette.accent,
                );
            });
        });
        ui.add_space(10.0);

        let columns = if ui.available_width() < 700.0 { 1 } else { 3 };
        ui.columns(columns, |cols| {
            let stats = [
                (
                    if french { "DISPONIBLES" } else { "AVAILABLE" },
                    registered.len().to_string(),
                    if french {
                        "présents dans cette compilation"
                    } else {
                        "bundled in this build"
                    },
                ),
                (
                    if french { "ACTIVÉS" } else { "ENABLED" },
                    selected_count.to_string(),
                    if french {
                        "préférences enregistrées"
                    } else {
                        "saved selections"
                    },
                ),
                (
                    if french { "INSTALLÉS" } else { "INSTALLED" },
                    installed_count.to_string(),
                    if french {
                        "paquets enregistrés"
                    } else {
                        "recorded packages"
                    },
                ),
            ];
            for (index, (label, value, detail)) in stats.into_iter().enumerate() {
                theme::card(&mut cols[index % columns], &palette, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(label)
                            .size(10.0)
                            .strong()
                            .color(palette.text_dim),
                    );
                    ui.label(egui::RichText::new(value).size(25.0).strong());
                    ui.label(
                        egui::RichText::new(detail)
                            .size(11.0)
                            .color(palette.text_dim),
                    );
                });
            }
        });

        ui.add_space(14.0);

        let installed_nes = nes_module_manager::installed_executable_path();
        theme::card(ui, &palette, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                module_tile(ui, &palette, "NES");
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("Nintendo Entertainment System")
                            .size(15.0)
                            .strong(),
                    );
                    let status = if let Some(path) = installed_nes.as_deref() {
                        let version = path
                            .parent()
                            .and_then(|parent| parent.file_name())
                            .and_then(|name| name.to_str())
                            .unwrap_or("unknown");
                        if french {
                            format!("Module externe installé · {version}")
                        } else {
                            format!("External module installed · {version}")
                        }
                    } else if french {
                        "Module optionnel · non installé".to_owned()
                    } else {
                        "Optional module · not installed".to_owned()
                    };
                    ui.label(
                        egui::RichText::new(status)
                            .size(12.0)
                            .color(palette.text_dim),
                    );
                    ui.label(egui::RichText::new(if french {
                        "Téléchargé séparément depuis les versions publiées du dépôt PeaceEmu-NES."
                    } else {
                        "Downloaded separately from the PeaceEmu-NES repository releases."
                    }).size(12.0).color(palette.text_dim));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if self.nes_installing {
                        if french {
                            "Téléchargement…"
                        } else {
                            "Downloading…"
                        }
                    } else if installed_nes.is_some() {
                        if french {
                            "Mettre à jour"
                        } else {
                            "Update module"
                        }
                    } else if french {
                        "Télécharger"
                    } else {
                        "Download module"
                    };
                    if ui
                        .add_enabled(!self.nes_installing, egui::Button::new(label))
                        .clicked()
                    {
                        self.start_nes_module_install();
                    }
                });
            });
        });

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("MODULE").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new("ACTIVATION")
                        .strong()
                        .color(palette.text_dim),
                );
            });
        });
        ui.separator();

        if module_ids.is_empty() {
            theme::card(ui, &palette, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new(if french {
                        "Aucun module n’est enregistré pour le moment."
                    } else {
                        "No modules are registered yet."
                    })
                    .color(palette.text_dim),
                );
            });
        }

        for id in module_ids {
            let Ok(module_id) = ModuleId::new(id.clone()) else {
                continue;
            };
            let is_bundled = registered.contains(&id);
            let nes_executable = if id == "nes" {
                nes_module_manager::installed_executable_path()
            } else {
                None
            };
            let installed_version = state
                .installed_version(&module_id)
                .map(str::to_owned)
                .or_else(|| {
                    nes_executable
                        .as_ref()
                        .and_then(|path| path.parent()?.file_name()?.to_str().map(str::to_owned))
                });
            let mut is_selected = selected.contains(&module_id);
            let descriptor = self
                .registry
                .system_ids()
                .find(|system_id| system_id.as_str() == id)
                .and_then(|system_id| self.registry.registered_descriptor(system_id));

            theme::card(ui, &palette, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    module_tile(ui, &palette, &id);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&id).size(15.0).strong());
                        ui.horizontal_wrapped(|ui| {
                            if let Some(version) = installed_version.as_deref() {
                                status_badge(
                                    ui,
                                    &if french {
                                        format!("Installé · v{version}")
                                    } else {
                                        format!("Installed · v{version}")
                                    },
                                    palette.success,
                                );
                            } else if is_bundled {
                                status_badge(
                                    ui,
                                    if french {
                                        "Intégré à l’application"
                                    } else {
                                        "Bundled with app"
                                    },
                                    palette.accent,
                                );
                            } else {
                                status_badge(
                                    ui,
                                    if french {
                                        "Fichiers absents"
                                    } else {
                                        "Files missing"
                                    },
                                    palette.warning,
                                );
                            }
                            if is_selected {
                                status_badge(
                                    ui,
                                    if french { "Activé" } else { "Enabled" },
                                    palette.success,
                                );
                            } else {
                                status_badge(
                                    ui,
                                    if french { "Désactivé" } else { "Disabled" },
                                    palette.text_dim,
                                );
                            }
                        });
                        ui.add_space(3.0);
                        ui.label(
                            egui::RichText::new(
                                if let Some(version) = installed_version.as_deref() {
                                    if french {
                                        format!("Version locale : {version}")
                                    } else {
                                        format!("Local version: {version}")
                                    }
                                } else if is_bundled {
                                    if french {
                                        "Version fournie avec cette compilation".to_owned()
                                    } else {
                                        "Version supplied with this build".to_owned()
                                    }
                                } else if french {
                                    "Aucune version locale détectée".to_owned()
                                } else {
                                    "No local version detected".to_owned()
                                },
                            )
                            .size(12.0)
                            .color(palette.text_dim),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_toggle = is_bundled;
                        let response = ui.add_enabled(
                            can_toggle,
                            egui::Checkbox::new(
                                &mut is_selected,
                                if french { "Utiliser" } else { "Use" },
                            ),
                        );
                        if response.changed() {
                            changed = true;
                            if is_selected {
                                if !selected.contains(&module_id) {
                                    selected.push(module_id.clone());
                                }
                            } else {
                                selected.retain(|selected_id| selected_id != &module_id);
                            }
                        }
                    });
                });

                ui.add_space(5.0);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(if french {
                            "Téléchargement"
                        } else {
                            "Download"
                        })
                        .strong()
                        .size(12.0),
                    );
                    if installed_version.is_some() {
                        status_badge(
                            ui,
                            if french { "Installé" } else { "Installed" },
                            palette.success,
                        );
                    } else if is_bundled {
                        status_badge(
                            ui,
                            if french {
                                "Inclus dans l’application"
                            } else {
                                "Included with app"
                            },
                            palette.accent,
                        );
                    } else if id == "nes" {
                        status_badge(
                            ui,
                            if french {
                                "Prêt à télécharger"
                            } else {
                                "Ready to download"
                            },
                            palette.accent,
                        );
                    } else {
                        status_badge(
                            ui,
                            if french {
                                "Source indisponible"
                            } else {
                                "Source unavailable"
                            },
                            palette.warning,
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if id == "nes" {
                            let label = if self.nes_installing {
                                if french {
                                    "Installation…"
                                } else {
                                    "Installing…"
                                }
                            } else if installed_version.is_some() {
                                if french {
                                    "Réinstaller / mettre à jour"
                                } else {
                                    "Reinstall / update"
                                }
                            } else if french {
                                "Télécharger et installer"
                            } else {
                                "Download and install"
                            };
                            let response =
                                ui.add_enabled(!self.nes_installing, egui::Button::new(label));
                            if response.clicked() {
                                self.start_nes_module_install();
                            }
                        } else {
                            let response = ui.add_enabled(
                                false,
                                egui::Button::new(if french {
                                    "Télécharger"
                                } else {
                                    "Download"
                                }),
                            );
                            response.on_hover_text(if french {
                                "Aucune version téléchargeable n’est configurée pour ce module."
                            } else {
                                "No downloadable release is configured for this module."
                            });
                        }
                    });
                    if id == "nes" && self.nes_installing {
                        ui.label(
                            egui::RichText::new(if french {
                                "Téléchargement, contrôle SHA-256 et installation en arrière-plan…"
                            } else {
                                "Downloading, checking SHA-256 and installing in the background…"
                            })
                            .size(12.0)
                            .color(palette.text_dim),
                        );
                    }
                });

                if let Some(descriptor) = descriptor {
                    let caps = descriptor.capabilities;
                    ui.add_space(5.0);
                    ui.horizontal_wrapped(|ui| {
                        for (label, enabled) in [
                            (
                                if french { "Contenu" } else { "Content" },
                                caps.content_loading,
                            ),
                            (if french { "Entrées" } else { "Input" }, caps.input),
                            (if french { "Vidéo" } else { "Video" }, caps.video_output),
                            ("Audio", caps.audio_output),
                            (
                                if french { "États" } else { "Save states" },
                                caps.save_states,
                            ),
                            (
                                if french { "Sauvegardes" } else { "Saves" },
                                caps.persistent_saves,
                            ),
                        ] {
                            status_badge(
                                ui,
                                &format!("{} {label}", if enabled { "●" } else { "○" }),
                                if enabled {
                                    palette.success
                                } else {
                                    palette.text_dim
                                },
                            );
                        }
                    });
                }
            });
        }

        if changed {
            state.set_selection(selected);
            match store.save(&state) {
                Ok(()) => {
                    self.message = if french {
                        "Sélection des modules enregistrée.".to_owned()
                    } else {
                        "Module selection saved.".to_owned()
                    };
                }
                Err(error) => {
                    self.message = if french {
                        format!("Échec de l’enregistrement de la sélection : {error}")
                    } else {
                        format!("Could not save module selection: {error}")
                    };
                }
            }
        }
        ui.add_space(16.0);
    }
}
