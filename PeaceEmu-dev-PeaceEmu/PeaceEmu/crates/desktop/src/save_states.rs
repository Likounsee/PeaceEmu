#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Stable per-ROM save-state slot paths for the desktop frontend.
//!
//! Actual persistence and crash recovery use the shared core SaveStateStore.

use std::io;
use std::path::{Path, PathBuf};

/// Resolves the stable save-state slot for a ROM under the user's configuration directory.
pub fn state_path(config_file: &Path, content_path: &Path) -> io::Result<PathBuf> {
    let config_dir = config_file.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "configuration path has no parent",
        )
    })?;
    let canonical_content = std::fs::canonicalize(content_path)?;
    let fingerprint = stable_fingerprint(canonical_content.to_string_lossy().as_bytes());
    Ok(config_dir
        .join("states")
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
            "desktop-state-path-{}-{label}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

    #[test]
    fn state_path_is_stable_and_scoped_to_the_canonical_rom_path() {
        let root = temporary_directory("stable");
        let config = root.join("config.conf");
        let rom = root.join("game.gb");
        std::fs::write(&rom, b"rom").unwrap();

        let first = state_path(&config, &rom).unwrap();
        let second = state_path(&config, &root.join(".").join("game.gb")).unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with(root.join("states")));
        assert_eq!(
            first.extension().and_then(|value| value.to_str()),
            Some("archiasave")
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_rom_path_does_not_create_a_save_slot() {
        let root = temporary_directory("missing-rom");
        let config = root.join("config.conf");
        assert!(state_path(&config, &root.join("missing.gb")).is_err());
        assert!(!root.join("states").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
