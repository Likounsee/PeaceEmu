//! Persistent last-successful-launch validation markers for library entries.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn history_path(config_file: &Path) -> Option<PathBuf> {
    Some(config_file.parent()?.join("validation-history.json"))
}

/// Loads canonical content paths that passed validation on their latest launch.
pub(crate) fn load(config_file: &Path) -> HashSet<PathBuf> {
    let Some(path) = history_path(config_file) else {
        return HashSet::new();
    };
    let Ok(bytes) = fs::read(path) else {
        return HashSet::new();
    };
    serde_json::from_slice::<Vec<String>>(&bytes)
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

/// Persists successful paths independently from the ordinary UI preferences.
pub(crate) fn save(config_file: Option<&Path>, paths: &HashSet<PathBuf>) -> std::io::Result<()> {
    let Some(config_file) = config_file else {
        return Ok(());
    };
    let Some(path) = history_path(config_file) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut values: Vec<String> = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    values.sort();
    let bytes = serde_json::to_vec_pretty(&values).map_err(std::io::Error::other)?;
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn successful_launch_markers_survive_reload() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("peaceemu-validation-history-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let config = root.join("preferences.json");
        let game = root.join("game.nes");
        let mut paths = HashSet::new();
        paths.insert(game.clone());

        save(Some(&config), &paths).unwrap();
        assert!(load(&config).contains(&game));
        fs::remove_dir_all(root).unwrap();
    }
}
