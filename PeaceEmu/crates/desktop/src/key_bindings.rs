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

const LEGACY_DEFAULTS: [egui::Key; 10] = [
    egui::Key::ArrowUp,
    egui::Key::ArrowDown,
    egui::Key::ArrowLeft,
    egui::Key::ArrowRight,
    egui::Key::Z,
    egui::Key::X,
    egui::Key::A,
    egui::Key::S,
    egui::Key::Enter,
    egui::Key::Backspace,
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
            let virtual_key = windows_virtual_key(key);
            let duplicate = bindings
                .keys
                .iter()
                .enumerate()
                .any(|(other, existing)| {
                    other != index && windows_virtual_key(*existing) == virtual_key
                });
            if !duplicate {
                bindings.keys[index] = key;
            }
        }
        // Upgrade files saved with the previous built-in defaults, without
        // overwriting a user's intentionally customized Select binding.
        if bindings.keys == LEGACY_DEFAULTS {
            bindings.keys = DEFAULTS;
        }
        bindings
    }

    pub(crate) fn set(&mut self, index: usize, key: egui::Key) -> Result<(), usize> {
        if index >= self.keys.len() {
            return Err(index);
        }
        if windows_virtual_key(key) == 0 {
            return Err(usize::MAX);
        }
        let virtual_key = windows_virtual_key(key);
        if let Some(other) = self.keys.iter().position(|existing| windows_virtual_key(*existing) == virtual_key) {
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

    pub(crate) fn control_for_virtual_key(&self, virtual_key: i32) -> Option<InputControl> {
        self.keys
            .iter()
            .position(|key| windows_virtual_key(*key) == virtual_key)
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

    /// Returns the Windows virtual-key codes expected by the standalone NES module,
    /// in its hardware order: A, B, Select, Start, Up, Down, Left, Right.
    pub(crate) fn nes_virtual_key_codes(&self) -> [i32; 8] {
        [4, 5, 9, 8, 0, 1, 2, 3].map(|index| windows_virtual_key(self.keys[index]))
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
            egui::Key::F25 if french => "Ctrl".to_owned(),
            egui::Key::F26 if french => "Alt".to_owned(),
            egui::Key::F27 if french => "Maj".to_owned(),
            egui::Key::F28 if french => "Windows".to_owned(),
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

fn windows_virtual_key(key: egui::Key) -> i32 {
    match key {
        egui::Key::ArrowUp => 0x26, egui::Key::ArrowDown => 0x28,
        egui::Key::ArrowLeft => 0x25, egui::Key::ArrowRight => 0x27,
        egui::Key::Escape => 0x1B, egui::Key::Tab => 0x09,
        egui::Key::Backspace => 0x08, egui::Key::Enter => 0x0D,
        egui::Key::Space => 0x20, egui::Key::Insert => 0x2D,
        egui::Key::Delete => 0x2E, egui::Key::Home => 0x24,
        egui::Key::End => 0x23, egui::Key::PageUp => 0x21,
        egui::Key::PageDown => 0x22,
        egui::Key::Copy => 0xE2, egui::Key::Cut => 0xE3, egui::Key::Paste => 0xE4,
        egui::Key::Colon | egui::Key::Semicolon => 0xBA,
        egui::Key::Comma => 0xBC, egui::Key::Minus => 0xBD,
        egui::Key::Period => 0xBE, egui::Key::Plus | egui::Key::Equals => 0xBB,
        egui::Key::Slash | egui::Key::Questionmark => 0xBF,
        egui::Key::Backslash | egui::Key::Pipe => 0xDC,
        egui::Key::OpenBracket => 0xDB, egui::Key::CloseBracket => 0xDD,
        egui::Key::Backtick => 0xC0,
        egui::Key::Num0 => 0x30, egui::Key::Num1 => 0x31,
        egui::Key::Num2 => 0x32, egui::Key::Num3 => 0x33,
        egui::Key::Num4 => 0x34, egui::Key::Num5 => 0x35,
        egui::Key::Num6 => 0x36, egui::Key::Num7 => 0x37,
        egui::Key::Num8 => 0x38, egui::Key::Num9 => 0x39,
        egui::Key::A => 0x41, egui::Key::B => 0x42, egui::Key::C => 0x43,
        egui::Key::D => 0x44, egui::Key::E => 0x45, egui::Key::F => 0x46,
        egui::Key::G => 0x47, egui::Key::H => 0x48, egui::Key::I => 0x49,
        egui::Key::J => 0x4A, egui::Key::K => 0x4B, egui::Key::L => 0x4C,
        egui::Key::M => 0x4D, egui::Key::N => 0x4E, egui::Key::O => 0x4F,
        egui::Key::P => 0x50, egui::Key::Q => 0x51, egui::Key::R => 0x52,
        egui::Key::S => 0x53, egui::Key::T => 0x54, egui::Key::U => 0x55,
        egui::Key::V => 0x56, egui::Key::W => 0x57, egui::Key::X => 0x58,
        egui::Key::Y => 0x59, egui::Key::Z => 0x5A,
        egui::Key::F1 => 0x70, egui::Key::F2 => 0x71,
        egui::Key::F3 => 0x72, egui::Key::F4 => 0x73,
        egui::Key::F5 => 0x74, egui::Key::F6 => 0x75,
        egui::Key::F7 => 0x76, egui::Key::F8 => 0x77,
        egui::Key::F9 => 0x78, egui::Key::F10 => 0x79,
        egui::Key::F11 => 0x7A, egui::Key::F12 => 0x7B,
        egui::Key::F13 => 0x7C, egui::Key::F14 => 0x7D,
        egui::Key::F15 => 0x7E, egui::Key::F16 => 0x7F,
        egui::Key::F17 => 0x80, egui::Key::F18 => 0x81,
        egui::Key::F19 => 0x82, egui::Key::F20 => 0x83,
        egui::Key::F21 => 0x84, egui::Key::F22 => 0x85,
        egui::Key::F23 => 0x86, egui::Key::F24 => 0x87,
        // Reserved egui key slots represent standalone modifier keys.
        egui::Key::F25 => 0x11, // VK_CONTROL
        egui::Key::F26 => 0x12, // VK_MENU (Alt)
        egui::Key::F27 => 0x10, // VK_SHIFT
        egui::Key::F28 => 0x5B, // VK_LWIN
        egui::Key::F29 | egui::Key::F30 | egui::Key::F31 | egui::Key::F32 |
        egui::Key::F33 | egui::Key::F34 | egui::Key::F35 => 0,
    }
}

fn key_name(key: egui::Key) -> &'static str {
    match key {
        egui::Key::F25 => "Ctrl",
        egui::Key::F26 => "Alt",
        egui::Key::F27 => "Shift",
        egui::Key::F28 => "Super",
        _ => key.name(),
    }
}

fn parse_key(value: &str) -> Option<egui::Key> {
    match value.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(egui::Key::F25),
        "alt" => Some(egui::Key::F26),
        "shift" => Some(egui::Key::F27),
        "super" | "windows" | "win" | "meta" => Some(egui::Key::F28),
        _ => egui::Key::from_name(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_nes_hardware_order_as_windows_virtual_keys() {
        let bindings = KeyBindings { keys: DEFAULTS };
        assert_eq!(
            bindings.nes_virtual_key_codes(),
            [0x5A, 0x58, 0x20, 0x0D, 0x26, 0x28, 0x25, 0x27]
        );

        let mut customized = KeyBindings { keys: DEFAULTS };
        customized.set(9, egui::Key::Backspace).unwrap();
        assert_eq!(customized.nes_virtual_key_codes()[2], 0x08);
    }

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
    fn previously_saved_default_select_binding_is_migrated_to_space() {
        let root =
            std::env::temp_dir().join(format!("peaceemu-legacy-bindings-{}", std::process::id()));
        let config = root.join("preferences.json");
        let directory = root.join("keybindings");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("nes.conf"),
            "# PeaceEmu module keyboard bindings v1\n0=ArrowUp\n1=ArrowDown\n2=ArrowLeft\n3=ArrowRight\n4=Z\n5=X\n6=A\n7=S\n8=Enter\n9=Backspace\n",
        )
        .unwrap();

        let loaded = KeyBindings::load(Some(&config), "nes");
        assert_eq!(loaded.control(egui::Key::Space), Some(InputControl::Select));
        assert_eq!(loaded.control(egui::Key::Backspace), None);

        std::fs::remove_dir_all(root).unwrap();
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
    fn assigns_function_and_extended_egui_keys() {
        let mut bindings = KeyBindings { keys: DEFAULTS };
        bindings.assign_or_swap(4, egui::Key::F13).unwrap();
        assert_eq!(bindings.nes_virtual_key_codes()[0], 0x7C);
        assert_eq!(bindings.label(4), "F13");
        assert_eq!(parse_key("F24"), Some(egui::Key::F24));
        assert_eq!(parse_key("Questionmark"), Some(egui::Key::Questionmark));
        assert_eq!(windows_virtual_key(egui::Key::F25), 0x11);
        assert_eq!(windows_virtual_key(egui::Key::F26), 0x12);
        assert_eq!(windows_virtual_key(egui::Key::F27), 0x10);
        assert_eq!(windows_virtual_key(egui::Key::F28), 0x5B);
        assert_eq!(parse_key("Ctrl"), Some(egui::Key::F25));
        assert_eq!(parse_key("Alt"), Some(egui::Key::F26));
        assert_eq!(parse_key("Shift"), Some(egui::Key::F27));
        assert_eq!(parse_key("Windows"), Some(egui::Key::F28));
        assert_eq!(windows_virtual_key(egui::Key::F35), 0);
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
