//! Per-game cover artwork storage for the desktop library.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_COVER_BYTES: u64 = 10 * 1024 * 1024;

fn cover_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("covers")
}

fn map_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("covers.conf")
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

pub(crate) fn load(config_path: &Path) -> HashMap<PathBuf, PathBuf> {
    let mut covers = HashMap::new();
    let Ok(contents) = fs::read_to_string(map_path(config_path)) else {
        return covers;
    };
    for line in contents.lines() {
        let Some((game, image)) = line.split_once('\t') else {
            continue;
        };
        if let (Some(game), Some(image)) = (unescape(game), unescape(image)) {
            let image = PathBuf::from(image);
            if image.is_file() {
                covers.insert(PathBuf::from(game), image);
            }
        }
    }
    covers
}

pub(crate) fn persist(
    config_path: &Path,
    covers: &HashMap<PathBuf, PathBuf>,
) -> std::io::Result<()> {
    let path = map_path(config_path);
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing config parent")
    })?;
    fs::create_dir_all(parent)?;

    let mut rows: Vec<_> = covers.iter().collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    let mut contents = String::new();
    for (game, image) in rows {
        contents.push_str(&escape(&game.to_string_lossy()));
        contents.push('\t');
        contents.push_str(&escape(&image.to_string_lossy()));
        contents.push('\n');
    }

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos();
    let temp = path.with_extension(format!("conf.tmp-{}-{nonce}", std::process::id()));
    let backup = path.with_extension("conf.bak");
    let write_result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }

    let had_previous = path.exists();
    if had_previous {
        if backup.exists() {
            fs::remove_file(&backup)?;
        }
        if let Err(error) = fs::rename(&path, &backup) {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
    }
    if let Err(error) = fs::rename(&temp, &path) {
        if had_previous {
            let _ = fs::rename(&backup, &path);
        }
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    if had_previous {
        let _ = fs::remove_file(backup);
    }
    Ok(())
}

fn safe_extension(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "webp" => Some("webp"),
        _ => None,
    }
}

fn store_bytes(config_path: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_COVER_BYTES {
        return Err("Image must be smaller than 10 MiB.".to_owned());
    }

    let reader = image::io::Reader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Unsupported or invalid image. Use PNG, JPEG, or WebP.".to_owned())?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| "Could not read image dimensions.".to_owned())?;
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("Image dimensions must be between 1 and 4096 pixels.".to_owned());
    }
    image::load_from_memory(bytes)
        .map_err(|_| "Unsupported or invalid image. Use PNG, JPEG, or WebP.".to_owned())?;

    let directory = cover_dir(config_path);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let extension = match image::guess_format(bytes).ok() {
        Some(image::ImageFormat::Png) => "png",
        Some(image::ImageFormat::Jpeg) => "jpg",
        Some(image::ImageFormat::WebP) => "webp",
        _ => return Err("Unsupported image format. Use PNG, JPEG, or WebP.".to_owned()),
    };
    let safe_name: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(48)
        .collect();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let target = directory.join(format!(
        "{}-{}-{nonce}.{}",
        safe_name,
        std::process::id(),
        extension
    ));
    fs::write(&target, bytes).map_err(|error| error.to_string())?;
    Ok(target)
}

pub(crate) fn import_local(config_path: &Path, source: &Path) -> Result<PathBuf, String> {
    if safe_extension(source).is_none() {
        return Err("Use a PNG, JPEG, or WebP image.".to_owned());
    }
    let metadata = fs::metadata(source).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_COVER_BYTES {
        return Err("Image must be a regular file smaller than 10 MiB.".to_owned());
    }
    let bytes = fs::read(source).map_err(|error| error.to_string())?;
    store_bytes(
        config_path,
        source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("local-cover"),
        &bytes,
    )
}
