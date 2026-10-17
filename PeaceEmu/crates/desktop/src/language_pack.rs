//! Runtime-loaded language packs. Translation data stays outside the executable.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_LANGUAGE_PACK_BYTES: u64 = 1024 * 1024;

pub(crate) struct LanguagePack {
    strings: HashMap<String, String>,
}

impl LanguagePack {
    /// Load the selected pack next to the executable, with a source-tree fallback for development.
    /// No translation data is compiled into the executable.
    pub(crate) fn load(language: &str) -> Self {
        let language = if language == "fr" { "fr" } else { "en" };
        let mut candidates = Vec::new();
        if let Some(directory) = std::env::current_exe()
            .ok()
            .and_then(|executable| executable.parent().map(Path::to_path_buf))
        {
            candidates.push(directory.join("languages"));
        }
        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/languages"));

        for directory in candidates {
            if let Ok(strings) = Self::load_from_directory(language, &directory) {
                return Self { strings };
            }
        }

        Self {
            strings: HashMap::new(),
        }
    }

    fn load_from_directory(
        language: &str,
        directory: &Path,
    ) -> std::io::Result<HashMap<String, String>> {
        let path = directory.join(format!("{language}.json"));
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_LANGUAGE_PACK_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "language pack must be a regular file no larger than 1 MiB",
            ));
        }

        let bytes = fs::read(&path)?;
        if bytes.len() as u64 > MAX_LANGUAGE_PACK_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "language pack exceeds the 1 MiB limit",
            ));
        }

        serde_json::from_slice(&bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }

    pub(crate) fn text(&self, key: &str) -> &str {
        self.strings.get(key).map(String::as_str).unwrap_or(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "peaceemu-language-pack-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn reads_translations_from_an_external_json_file() {
        let directory = temporary_directory();
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("fr.json"),
            r#"{"settings.application.title":"Application"}"#,
        )
        .unwrap();

        let pack = LanguagePack {
            strings: LanguagePack::load_from_directory("fr", &directory).unwrap(),
        };
        assert_eq!(pack.text("settings.application.title"), "Application");
        assert_eq!(pack.text("missing.key"), "missing.key");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_malformed_language_pack_json() {
        let directory = temporary_directory();
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("en.json"), "{not-json").unwrap();

        assert_eq!(
            LanguagePack::load_from_directory("en", &directory)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_language_pack_symlinks() {
        let directory = temporary_directory();
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("actual.json"), "{}").unwrap();
        std::os::unix::fs::symlink(directory.join("actual.json"), directory.join("en.json"))
            .unwrap();

        assert_eq!(
            LanguagePack::load_from_directory("en", &directory)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
