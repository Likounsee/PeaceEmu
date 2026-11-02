//! User-configurable desktop keyboard bindings.

use eframe::egui;
use emulation_core::core_io::InputControl;
use std::{fs, path::Path};

pub(crate) const ACTIONS: [(&str, &str); 10] = [
    ("Haut", "Up"),
    ("Bas", "Down"),
    ("Gauche", "Left"),
    ("Droite", "Right"),
    ("A", "Action1"),
    ("B", "Action2"),
    ("X", "Action3"),
    ("Y", "Action4"),
    ("Start", "Start"),
    ("Select", "Select"),
];

const DEFAULTS: [egui::Key; 10] = [
    egui::Key::ArrowUp,
    egui::Key::ArrowDown,
    egui::Key::ArrowLeft,
    egui::Key::ArrowRight,
    egui::Key::Z,
    egui::Key::X,
    egui::Key::A,
    egui::Key::S,
    egui::Key::Enter,
    // Space is reliably delivered as a normal key event by egui in the game viewport.
    egui::Key::Space,
];

pub(crate) struct KeyBindings {
    keys: [egui::Key; 10],
}

impl KeyBindings {
    pub(crate) fn load(config: Option<&Path>, module_id: &str) -> Self {
        let mut bindings = Self { keys: DEFAULTS };
        let Some(directory) = config.and_then(Path::parent) else {
            return bindings;
        };
        if !valid_module_id(module_id) {
            return bindings;
        }
        let module_path = directory
            .join("keybindings")
            .join(format!("{module_id}.conf"));
        // Prefer the module-specific file. Fall back to the legacy global file
        // so existing users retain their previous mapping after upgrading.
        let contents = read_binding_file(&module_path)
            .or_else(|| read_binding_file(&directory.join("keybindings.conf")));
        let Some(contents) = contents else {
            return bindings;
        };
        for line in contents.lines() {
            let Some((index, value)) = line.split_once('=') else {
                continue;
            };
            let Ok(index) = index.parse::<usize>() else {
                continue;
            };
            if index >= bindings.keys.len() {
                continue;
            }
            let Some(key) = parse_key(value) else {
                continue;
            };
            let duplicate = bindings
                .keys
                .iter()
                .enumerate()
                .any(|(other, existing)| other != index && *existing == key);
            if !duplicate {
                bindings.keys[index] = key;
            }
        }
        bindings
    }

    pub(crate) fn set(&mut self, index: usize, key: egui::Key) -> Result<(), usize> {
        if index >= self.keys.len() {
            return Err(index);
        }
        if key_name(key) == "Unsupported" {
            return Err(usize::MAX);
        }
        if let Some(other) = self.keys.iter().position(|existing| *existing == key) {
            if other != index {
                return Err(other);
            }
        }
        self.keys[index] = key;
        Ok(())
    }

    /// Assigns `key` to the action at `index`. When another action already
    /// uses that key, the two actions swap keys instead of refusing.
    ///
    /// Returns the other action when a swap happened. The error value is the
    /// same as [`KeyBindings::set`]: `usize::MAX` for an unsupported key.
    pub(crate) fn assign_or_swap(
        &mut self,
        index: usize,
        key: egui::Key,
    ) -> Result<Option<usize>, usize> {
        match self.set(index, key) {
            Ok(()) => Ok(None),
            Err(other) if other < self.keys.len() => {
                self.keys.swap(index, other);
                Ok(Some(other))
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn control(&self, key: egui::Key) -> Option<InputControl> {
        self.keys
            .iter()
            .position(|candidate| *candidate == key)
            .map(|index| match index {
                0 => InputControl::Up,
                1 => InputControl::Down,
                2 => InputControl::Left,
                3 => InputControl::Right,
                4 => InputControl::Action1,
                5 => InputControl::Action2,
                6 => InputControl::Action3,
                7 => InputControl::Action4,
                8 => InputControl::Start,
                _ => InputControl::Select,
            })
    }

    pub(crate) fn reset(&mut self) {
        self.keys = DEFAULTS;
    }

    pub(crate) fn label(&self, index: usize) -> String {
        key_name(self.keys[index]).to_owned()
    }

    /// Human-readable key name for the settings page: arrows as symbols and
    /// translated names for the common special keys.
    pub(crate) fn display_label(&self, index: usize, french: bool) -> String {
        match self.keys[index] {
            egui::Key::ArrowUp => "↑".to_owned(),
            egui::Key::ArrowDown => "↓".to_owned(),
            egui::Key::ArrowLeft => "←".to_owned(),
            egui::Key::ArrowRight => "→".to_owned(),
            egui::Key::Enter if french => "Entrée".to_owned(),
            egui::Key::Backspace if french => "Retour arrière".to_owned(),
            egui::Key::Space if french => "Espace".to_owned(),
            egui::Key::Escape if french => "Échap".to_owned(),
            _ => self.label(index),
        }
    }

    pub(crate) fn save(&self, config: Option<&Path>, module_id: &str) -> std::io::Result<()> {
        let directory = config.and_then(Path::parent).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "configuration directory unavailable",
            )
        })?;
        if !valid_module_id(module_id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid module identifier",
            ));
        }
        let directory = directory.join("keybindings");
        fs::create_dir_all(&directory)?;
        let mut contents = String::from("# PeaceEmu module keyboard bindings v1\n");
        for (index, key) in self.keys.iter().enumerate() {
            contents.push_str(&format!("{index}={}\n", key_name(*key)));
        }
        // Write to a temporary file first so a crash cannot leave a truncated
        // mapping behind.
        let target = directory.join(format!("{module_id}.conf"));
        let temporary = directory.join(format!("{module_id}.conf.tmp"));
        fs::write(&temporary, contents)?;
        fs::rename(&temporary, &target)
    }
}

fn valid_module_id(module_id: &str) -> bool {
    !module_id.is_empty()
        && module_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn read_binding_file(path: &Path) -> Option<String> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > 4096 {
        return None;
    }
    fs::read_to_string(path).ok()
}

fn key_name(key: egui::Key) -> &'static str {
    match key {
        egui::Key::ArrowUp => "ArrowUp",
        egui::Key::ArrowDown => "ArrowDown",
        egui::Key::ArrowLeft => "ArrowLeft",
        egui::Key::ArrowRight => "ArrowRight",
        egui::Key::Enter => "Enter",
        egui::Key::Backspace => "Backspace",
        egui::Key::Space => "Space",
        egui::Key::Tab => "Tab",
        egui::Key::Escape => "Escape",
        egui::Key::A => "A",
        egui::Key::B => "B",
        egui::Key::C => "C",
        egui::Key::D => "D",
        egui::Key::E => "E",
        egui::Key::F => "F",
        egui::Key::G => "G",
        egui::Key::H => "H",
        egui::Key::I => "I",
        egui::Key::J => "J",
        egui::Key::K => "K",
        egui::Key::L => "L",
        egui::Key::M => "M",
        egui::Key::N => "N",
        egui::Key::O => "O",
        egui::Key::P => "P",
        egui::Key::Q => "Q",
        egui::Key::R => "R",
        egui::Key::S => "S",
        egui::Key::T => "T",
        egui::Key::U => "U",
        egui::Key::V => "V",
        egui::Key::W => "W",
        egui::Key::X => "X",
        egui::Key::Y => "Y",
        egui::Key::Z => "Z",
        _ => "Unsupported",
    }
}

fn parse_key(value: &str) -> Option<egui::Key> {
    Some(match value {
        "ArrowUp" => egui::Key::ArrowUp,
        "ArrowDown" => egui::Key::ArrowDown,
        "ArrowLeft" => egui::Key::ArrowLeft,
        "ArrowRight" => egui::Key::ArrowRight,
        "Enter" => egui::Key::Enter,
        "Backspace" => egui::Key::Backspace,
        "Space" => egui::Key::Space,
        "Tab" => egui::Key::Tab,
        "Escape" => egui::Key::Escape,
        "A" => egui::Key::A,
        "B" => egui::Key::B,
        "C" => egui::Key::C,
        "D" => egui::Key::D,
        "E" => egui::Key::E,
        "F" => egui::Key::F,
        "G" => egui::Key::G,
        "H" => egui::Key::H,
        "I" => egui::Key::I,
        "J" => egui::Key::J,
        "K" => egui::Key::K,
        "L" => egui::Key::L,
        "M" => egui::Key::M,
        "N" => egui::Key::N,
        "O" => egui::Key::O,
        "P" => egui::Key::P,
        "Q" => egui::Key::Q,
        "R" => egui::Key::R,
        "S" => egui::Key::S,
        "T" => egui::Key::T,
        "U" => egui::Key::U,
        "V" => egui::Key::V,
        "W" => egui::Key::W,
        "X" => egui::Key::X,
        "Y" => egui::Key::Y,
        "Z" => egui::Key::Z,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_existing_layout() {
        let bindings = KeyBindings { keys: DEFAULTS };
        assert_eq!(bindings.control(egui::Key::ArrowUp), Some(InputControl::Up));
        assert_eq!(bindings.control(egui::Key::Z), Some(InputControl::Action1));
        assert_eq!(
            bindings.control(egui::Key::Space),
            Some(InputControl::Select)
        );
    }

    #[test]
    fn duplicate_key_is_rejected() {
        let mut bindings = KeyBindings { keys: DEFAULTS };
        assert_eq!(bindings.set(4, egui::Key::ArrowUp), Err(0));
        assert_eq!(bindings.control(egui::Key::Z), Some(InputControl::Action1));
    }

    #[test]
    fn duplicate_key_is_swapped_by_assign_or_swap() {
        let mut bindings = KeyBindings { keys: DEFAULTS };
        assert_eq!(bindings.assign_or_swap(4, egui::Key::ArrowUp), Ok(Some(0)));
        assert_eq!(
            bindings.control(egui::Key::ArrowUp),
            Some(InputControl::Action1)
        );
        assert_eq!(bindings.control(egui::Key::Z), Some(InputControl::Up));
    }

    #[test]
    fn assign_or_swap_rejects_unsupported_keys() {
        let mut bindings = KeyBindings { keys: DEFAULTS };
        assert_eq!(bindings.assign_or_swap(4, egui::Key::F1), Err(usize::MAX));
        assert_eq!(bindings.control(egui::Key::Z), Some(InputControl::Action1));
    }

    #[test]
    fn reassignment_changes_runtime_mapping() {
        let mut bindings = KeyBindings { keys: DEFAULTS };
        bindings.set(4, egui::Key::Q).unwrap();
        assert_eq!(bindings.control(egui::Key::Q), Some(InputControl::Action1));
        assert_eq!(bindings.control(egui::Key::Z), None);
    }

    #[test]
    fn module_identifiers_reject_path_traversal() {
        assert!(valid_module_id("game-boy"));
        assert!(!valid_module_id("../nes"));
        assert!(!valid_module_id(""));
    }

    #[test]
    fn module_binding_files_are_isolated() {
        let root = std::env::temp_dir().join(format!("peaceemu-bindings-{}", std::process::id()));
        let config = root.join("preferences.json");
        let mut nes = KeyBindings { keys: DEFAULTS };
        nes.set(4, egui::Key::Q).unwrap();
        nes.save(Some(&config), "nes").unwrap();
        let loaded_nes = KeyBindings::load(Some(&config), "nes");
        let loaded_gb = KeyBindings::load(Some(&config), "game-boy");
        assert_eq!(
            loaded_nes.control(egui::Key::Q),
            Some(InputControl::Action1)
        );
        assert_eq!(loaded_gb.control(egui::Key::Z), Some(InputControl::Action1));
        std::fs::remove_dir_all(root).unwrap();
    }
}
