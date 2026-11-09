#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Stable per-ROM paths for persistent cartridge RAM saves.

use std::io;
use std::path::{Path, PathBuf};

/// Resolves the persistent game-save slot for a ROM under the user's configuration directory.
pub fn save_path(config_file: &Path, content_path: &Path) -> io::Result<PathBuf> {
    let config_dir = config_file.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "configuration path has no parent",
        )
    })?;
    // Session paths are canonicalized when a ROM is launched. If the user
    // deletes or moves that ROM while the session is still open, retain the
    // known absolute path so its battery RAM can still be flushed safely.
    let canonical_content = match std::fs::canonicalize(content_path) {
        Ok(path) => path,
        Err(_) if content_path.is_absolute() => content_path.to_path_buf(),
        Err(error) => return Err(error),
    };
    let fingerprint = stable_fingerprint(canonical_content.to_string_lossy().as_bytes());
    Ok(config_dir
        .join("saves")
        .join(format!("{fingerprint:016x}.archiasave")))
}

fn stable_fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "host-app-game-save-path-{}-{label}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

    #[test]
    fn game_save_path_is_stable_and_separate_from_machine_states() {
        let root = temporary_directory("stable");
        let config = root.join("config.conf");
        let rom = root.join("game.gb");
        std::fs::write(&rom, b"rom").unwrap();

        let first = save_path(&config, &rom).unwrap();
        let second = save_path(&config, &root.join(".").join("game.gb")).unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with(root.join("saves")));
        assert_ne!(
            first,
            crate::save_states::state_path(&config, &rom).unwrap()
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_absolute_rom_keeps_a_stable_slot_without_creating_directories() {
        let root = temporary_directory("missing-rom");
        let config = root.join("config.conf");
        let slot = save_path(&config, &root.join("missing.gb")).unwrap();
        assert!(slot.starts_with(root.join("saves")));
        assert!(!root.join("saves").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
