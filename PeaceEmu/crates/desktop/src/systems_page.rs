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

fn module_display_name(id: &str) -> String {
    match id {
        "nes" => "Nintendo (NES)".to_owned(),
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

fn module_inventory(
    registered: &BTreeSet<String>,
    available: &BTreeSet<String>,
    installed: &BTreeSet<String>,
) -> BTreeSet<String> {
    registered
        .union(available)
        .cloned()
        .chain(installed.iter().cloned())
        .collect()
}

fn persist_nes_install_state(version: Option<&str>) -> Result<(), String> {
    let path = default_user_state_path().map_err(|error| error.to_string())?;
    let store = ModuleStateStore::new(path);
    let mut state = store.load().map_err(|error| error.to_string())?;
    let id = ModuleId::new("nes").map_err(|error| error.to_string())?;

    if let Some(version) = version {
        state
            .record_installed(id.clone(), version.to_owned())
            .map_err(|error| error.to_string())?;
        let mut selected = state.selected_modules().to_vec();
        selected.push(id);
        state.set_selection(selected);
    } else {
        state.record_uninstalled(&id);
        let selected = state
            .selected_modules()
            .iter()
            .filter(|selected| **selected != id)
            .cloned()
            .collect::<Vec<_>>();
        state.set_selection(selected);
    }
    store.save(&state).map_err(|error| error.to_string())
}

fn version_from_installed_path(path: &std::path::Path) -> Option<String> {
    path.parent()?
        .file_name()?
        .to_str()?
        .strip_prefix("nes-module-")
        .map(str::to_owned)
}

impl DesktopApp {
    pub(super) fn show_systems(&mut self, ui: &mut egui::Ui) {
        if let Some(receiver) = self.nes_install_receiver.take() {
            match receiver.try_recv() {
                Ok(Ok(path)) => {
                    self.nes_installing = false;
                    let persisted = version_from_installed_path(&path)
                        .ok_or_else(|| "version du paquet installé introuvable".to_owned())
                        .and_then(|version| persist_nes_install_state(Some(&version)));
                    self.message = match persisted {
                        Ok(()) if self.language == "fr" => {
                            format!("Nintendo (NES) installé et vérifié : {}", path.display())
                        }
                        Ok(()) => format!("Nintendo (NES) installed and verified: {}", path.display()),
                        Err(error) if self.language == "fr" => format!(
                            "Module NES installé, mais son état n’a pas pu être enregistré : {error}"
                        ),
                        Err(error) => format!(
                            "NES module installed, but its state could not be saved: {error}"
                        ),
                    };
                }
                Ok(Err(error)) => {
                    self.nes_installing = false;
                    self.message = if self.language == "fr" {
                        format!("Échec de l’installation de Nintendo (NES) : {error}")
                    } else {
                        format!("Nintendo (NES) installation failed: {error}")
                    };
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.nes_install_receiver = Some(receiver);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.nes_installing = false;
                    self.message = if self.language == "fr" {
                        "Le téléchargement du module Nintendo (NES) s’est interrompu.".to_owned()
                    } else {
                        "Nintendo (NES) module download stopped unexpectedly.".to_owned()
                    };
                }
            }
        }

        if let Some(receiver) = self.nes_uninstall_receiver.take() {
            match receiver.try_recv() {
                Ok(Ok(removed)) => {
                    self.nes_uninstalling = false;
                    let persisted = persist_nes_install_state(None);
                    self.message = match (persisted, self.language == "fr") {
                        (Ok(()), true) => format!(
                            "Nintendo (NES) désinstallé. {removed} version(s) supprimée(s). Vos jeux et sauvegardes sont conservés."
                        ),
                        (Ok(()), false) => format!(
                            "Nintendo (NES) uninstalled. Removed {removed} version(s). Your games and saves were kept."
                        ),
                        (Err(error), true) => format!(
                            "Module désinstallé, mais l’état local n’a pas pu être mis à jour : {error}"
                        ),
                        (Err(error), false) => format!(
                            "Module uninstalled, but local state could not be updated: {error}"
                        ),
                    };
                }
                Ok(Err(error)) => {
                    self.nes_uninstalling = false;
                    self.message = if self.language == "fr" {
                        format!("Échec de la désinstallation de Nintendo (NES) : {error}")
                    } else {
                        format!("Nintendo (NES) uninstall failed: {error}")
                    };
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.nes_uninstall_receiver = Some(receiver);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.nes_uninstalling = false;
                    self.message = if self.language == "fr" {
                        "La désinstallation du module Nintendo (NES) s’est interrompue.".to_owned()
                    } else {
                        "Nintendo (NES) uninstall stopped unexpectedly.".to_owned()
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
        let state = match store.load() {
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

        // Availability is based on the embedded catalogue and this host target,
        // not on how many modules happen to be compiled into the desktop shell.
        let catalogue_available: BTreeSet<String> =
            nes_module_manager::available_module_ids().into_iter().collect();
        let installed_external: BTreeSet<String> =
            if nes_module_manager::installed_executable_path().is_some() {
                ["nes".to_owned()].into_iter().collect()
            } else {
                BTreeSet::new()
            };
        let installed_recorded: BTreeSet<String> = state
            .installed_modules()
            .iter()
            .map(|module| module.id.as_str().to_owned())
            .collect();
        let installed_ids: BTreeSet<String> = registered
            .union(&installed_recorded)
            .cloned()
            .chain(installed_external.iter().cloned())
            .collect();
        let available_ids: BTreeSet<String> = registered
            .union(&catalogue_available)
            .cloned()
            .collect();
        let module_ids = module_inventory(&registered, &catalogue_available, &installed_ids);

        theme::page_header(
            ui,
            &palette,
            self.language_pack.text("ui.systems_page.001"),
            if french {
                "Installez un module pour ajouter sa console. L’installation l’active automatiquement."
            } else {
                "Install a module to add its console. Installing it makes it ready to use."
            },
            |_| {},
        );

        let columns = if ui.available_width() < 620.0 { 1 } else { 3 };
        ui.columns(columns, |cols| {
            let stats = [
                (
                    if french { "DISPONIBLES" } else { "AVAILABLE" },
                    available_ids.len().to_string(),
                    if french {
                        "catalogue compatible + cœurs intégrés"
                    } else {
                        "compatible catalogue + built-in cores"
                    },
                ),
                (
                    if french { "INSTALLÉS" } else { "INSTALLED" },
                    installed_ids.len().to_string(),
                    if french {
                        "prêts à être utilisés"
                    } else {
                        "ready to use"
                    },
                ),
                (
                    if french { "INTÉGRÉS" } else { "BUILT IN" },
                    registered.len().to_string(),
                    if french {
                        "fournis avec l’application"
                    } else {
                        "shipped with the app"
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
        ui.label(
            egui::RichText::new(if french { "MODULES" } else { "MODULES" })
                .strong()
                .size(13.0),
        );
        ui.separator();

        if module_ids.is_empty() {
            theme::card(ui, &palette, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new(if french {
                        "Aucun module n’est disponible pour cette plateforme."
                    } else {
                        "No modules are available for this platform."
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
                .or_else(|| nes_executable.as_ref().and_then(|path| version_from_installed_path(path)));
            let is_installed = is_bundled || installed_version.is_some();
            let is_available = available_ids.contains(&id);
            let display_name = module_display_name(&id);

            theme::card(ui, &palette, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    module_tile(ui, &palette, &display_name);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&display_name).size(16.0).strong());
                        ui.label(
                            egui::RichText::new(if id == "nes" {
                                "Nintendo Entertainment System"
                            } else {
                                id.as_str()
                            })
                            .size(12.0)
                            .color(palette.text_dim),
                        );
                        ui.horizontal_wrapped(|ui| {
                            if is_installed {
                                status_badge(
                                    ui,
                                    &if french { "Installé" } else { "Installed" },
                                    palette.success,
                                );
                            } else if is_available {
                                status_badge(
                                    ui,
                                    &if french { "Disponible" } else { "Available" },
                                    palette.accent,
                                );
                            } else {
                                status_badge(
                                    ui,
                                    &if french { "Indisponible" } else { "Unavailable" },
                                    palette.warning,
                                );
                            }
                            if is_bundled {
                                status_badge(
                                    ui,
                                    &if french { "Inclus" } else { "Built in" },
                                    palette.accent,
                                );
                            }
                            if let Some(version) = installed_version.as_deref() {
                                status_badge(
                                    ui,
                                    &format!("v{version}"),
                                    palette.success,
                                );
                            }
                        });
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if id == "nes" {
                            if is_installed {
                                if ui
                                    .add_enabled(
                                        !self.nes_installing && !self.nes_uninstalling,
                                        egui::Button::new(if french { "Désinstaller" } else { "Uninstall" }),
                                    )
                                    .clicked()
                                {
                                    self.nes_uninstall_confirm_open = true;
                                }
                            }
                            let label = if self.nes_installing {
                                if french { "Installation…" } else { "Installing…" }
                            } else if self.nes_uninstalling {
                                if french { "Désinstallation…" } else { "Uninstalling…" }
                            } else if is_installed {
                                if french { "Mettre à jour" } else { "Update" }
                            } else if is_available {
                                if french { "Installer" } else { "Install" }
                            } else if french {
                                "Non disponible"
                            } else {
                                "Unavailable"
                            };
                            let can_install = is_available
                                && !self.nes_installing
                                && !self.nes_uninstalling;
                            if ui
                                .add_enabled(can_install, egui::Button::new(label))
                                .clicked()
                            {
                                self.start_nes_module_install();
                            }
                        } else if is_bundled {
                            status_badge(
                                ui,
                                &if french { "Prêt à l’emploi" } else { "Ready to use" },
                                palette.success,
                            );
                        }
                    });
                });

                if let Some(descriptor) = self
                    .registry
                    .system_ids()
                    .find(|system_id| system_id.as_str() == id)
                    .and_then(|system_id| self.registry.registered_descriptor(system_id))
                {
                    let caps = descriptor.capabilities;
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        for (label, enabled) in [
                            (if french { "Contenu" } else { "Content" }, caps.content_loading),
                            (if french { "Entrées" } else { "Input" }, caps.input),
                            (if french { "Vidéo" } else { "Video" }, caps.video_output),
                            ("Audio", caps.audio_output),
                            (if french { "États" } else { "Save states" }, caps.save_states),
                            (if french { "Sauvegardes" } else { "Saves" }, caps.persistent_saves),
                        ] {
                            status_badge(
                                ui,
                                &format!("{} {label}", if enabled { "●" } else { "○" }),
                                if enabled { palette.success } else { palette.text_dim },
                            );
                        }
                    });
                }
            });
        }

        let mut confirm_uninstall = false;
        let mut close_confirmation = false;
        if self.nes_uninstall_confirm_open {
            let mut open = true;
            egui::Window::new(if french {
                "Désinstaller Nintendo (NES) ?"
            } else {
                "Uninstall Nintendo (NES)?"
            })
            .id(egui::Id::new("confirm_nes_module_uninstall"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.set_min_width(320.0);
                ui.label(if french {
                    "Seuls les fichiers du module seront supprimés. Vos ROM, sauvegardes et paramètres resteront intacts."
                } else {
                    "Only module files will be removed. Your ROMs, saves and settings will remain untouched."
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(if french { "Annuler" } else { "Cancel" }).clicked() {
                        close_confirmation = true;
                    }
                    if ui
                        .add_enabled(
                            !self.nes_uninstalling,
                            egui::Button::new(if self.nes_uninstalling {
                                if french { "Désinstallation…" } else { "Uninstalling…" }
                            } else if french {
                                "Confirmer la désinstallation"
                            } else {
                                "Confirm uninstall"
                            }),
                        )
                        .clicked()
                    {
                        confirm_uninstall = true;
                        close_confirmation = true;
                    }
                });
            });
            if !open {
                close_confirmation = true;
            }
        }
        if close_confirmation {
            self.nes_uninstall_confirm_open = false;
        }
        if confirm_uninstall {
            self.start_nes_module_uninstall();
        }

        ui.add_space(16.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nes_module_has_a_human_friendly_nintendo_system_name() {
        assert_eq!(module_display_name("nes"), "Nintendo (NES)");
    }

    #[test]
    fn module_inventory_deduplicates_built_in_catalogue_and_installed_entries() {
        let registered = ["game-boy".to_owned(), "nes".to_owned()].into_iter().collect();
        let available = ["nes".to_owned(), "nintendo-switch".to_owned()]
            .into_iter()
            .collect();
        let installed = ["nes".to_owned()].into_iter().collect();

        let inventory = module_inventory(&registered, &available, &installed);
        assert_eq!(inventory.len(), 3);
        assert!(inventory.contains("nes"));
        assert!(inventory.contains("game-boy"));
        assert!(inventory.contains("nintendo-switch"));
    }

    #[test]
    fn module_display_names_are_readable_for_hyphenated_ids() {
        assert_eq!(module_display_name("game-boy"), "Game Boy");
        assert_eq!(module_display_name("nintendo-switch"), "Nintendo Switch");
    }

    #[test]
    fn installed_path_version_uses_the_release_version_not_the_tag_prefix() {
        assert_eq!(
            version_from_installed_path(std::path::Path::new(
                "modules/nes/nes-module-0.2.3/peaceemu-nes-desktop.exe"
            )),
            Some("0.2.3".to_owned())
        );
    }
}
