//! Persistent per-game display names, keyed by the imported content path.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn names_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("game-names.conf")
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn unescape(value: &str) -> Option<String> {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        match chars.next()? {
            '\\' => output.push('\\'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            _ => return None,
        }
    }
    Some(output)
}

pub(crate) fn load(config_path: &Path) -> HashMap<PathBuf, String> {
    let mut names = HashMap::new();
    let Ok(contents) = fs::read_to_string(names_path(config_path)) else {
        return names;
    };
    for line in contents.lines() {
        let Some((game, name)) = line.split_once('\t') else {
            continue;
        };
        if let (Some(game), Some(name)) = (unescape(game), unescape(name)) {
            let name = name.trim();
            if !name.is_empty() && name.chars().count() <= 120 {
                names.insert(PathBuf::from(game), name.to_owned());
            }
        }
    }
    names
}

pub(crate) fn persist(config_path: &Path, names: &HashMap<PathBuf, String>) -> std::io::Result<()> {
    let path = names_path(config_path);
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing config parent")
    })?;
    fs::create_dir_all(parent)?;
    let mut rows: Vec<_> = names.iter().collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    let mut contents = String::new();
    for (game, name) in rows {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            continue;
        }
        contents.push_str(&escape(&game.to_string_lossy()));
        contents.push('\t');
        contents.push_str(&escape(name));
        contents.push('\n');
    }
    let temp = path.with_extension(format!("conf.tmp-{}", std::process::id()));
    let backup = path.with_extension("conf.bak");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    if path.exists() {
        if backup.exists() {
            fs::remove_file(&backup)?;
        }
        fs::rename(&path, &backup)?;
    }
    if let Err(error) = fs::rename(&temp, &path) {
        if backup.exists() {
            let _ = fs::rename(&backup, &path);
        }
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn names_round_trip_and_ignore_empty_values() {
        let root = std::env::temp_dir().join(format!(
            "peaceemu-game-names-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let config = root.join("desktop.conf");
        let game = PathBuf::from("C:/Games/a\tb.gb");
        let mut names = HashMap::new();
        names.insert(game.clone(), "Mon jeu préféré".to_owned());
        persist(&config, &names).unwrap();
        assert_eq!(
            load(&config).get(&game).map(String::as_str),
            Some("Mon jeu préféré")
        );
        names.insert(game.clone(), "   ".to_owned());
        persist(&config, &names).unwrap();
        assert!(!load(&config).contains_key(&game));
        fs::remove_dir_all(root).unwrap();
    }
}
