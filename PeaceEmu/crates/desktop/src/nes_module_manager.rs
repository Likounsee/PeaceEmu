//! Optional NES module download and installation.
#![forbid(unsafe_code)]

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;
use ureq::ResponseExt;

const RELEASES_URL: &str =
    "https://api.github.com/repos/Likounsee/PeaceEmu-NES/releases?per_page=20";
const MAX_RELEASES_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MODULE_BYTES: u64 = 100 * 1024 * 1024;
const EXE_NAME: &str = "peaceemu-nes-desktop.exe";
const CHECKSUM_NAME: &str = "peaceemu-nes-desktop.exe.sha256";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

fn module_root() -> Result<PathBuf, String> {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(local)
            .join("PeaceEmu")
            .join("modules")
            .join("nes"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("PeaceEmu")
            .join("modules")
            .join("nes"));
    }
    Err("Could not locate a user-writable PeaceEmu data directory.".to_owned())
}

fn version_number(tag: &str) -> Option<(u64, u64, u64)> {
    let version = tag.strip_prefix("nes-module-")?;
    let mut parts = version.split('.');
    let major = parse_version_component(parts.next()?)?;
    let minor = parse_version_component(parts.next()?)?;
    let patch = parse_version_component(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn parse_version_component(component: &str) -> Option<u64> {
    if component.is_empty()
        || (component.len() > 1 && component.starts_with('0'))
        || !component.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    component.parse().ok()
}

/// Returns the newest successfully installed NES module executable, if present.
pub(crate) fn installed_executable_path() -> Option<PathBuf> {
    let root = module_root().ok()?;
    let mut versions = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let number = version_number(&name)?;
            let exe = entry.path().join(EXE_NAME);
            exe.is_file().then_some((number, exe))
        })
        .collect::<Vec<_>>();
    versions.sort_by_key(|(number, _)| *number);
    versions.pop().map(|(_, path)| path)
}

/// Downloads the newest published module release, verifies its SHA-256 sidecar,
/// and installs it into a version-specific directory without overwriting prior builds.
pub(crate) fn install_latest() -> Result<PathBuf, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_body(Some(Duration::from_secs(20)))
        .build()
        .into();
    let mut response = agent
        .get(RELEASES_URL)
        .header("User-Agent", "PeaceEmu-module-manager")
        .call()
        .map_err(|error| format!("Could not retrieve the NES module catalogue: {error}"))?;
    let mut json = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_RELEASES_BYTES + 1)
        .read_to_end(&mut json)
        .map_err(|error| format!("Could not read NES release catalogue: {error}"))?;
    if json.len() as u64 > MAX_RELEASES_BYTES {
        return Err("NES release catalogue exceeds the safety limit.".to_owned());
    }
    let releases: Vec<Release> = serde_json::from_slice(&json)
        .map_err(|error| format!("NES release catalogue is invalid: {error}"))?;
    let release = releases
        .into_iter()
        .filter(|release| !release.draft && version_number(&release.tag_name).is_some())
        .filter(|release| {
            release.assets.iter().any(|asset| asset.name == EXE_NAME)
                && release
                    .assets
                    .iter()
                    .any(|asset| asset.name == CHECKSUM_NAME)
        })
        .max_by_key(|release| {
            version_number(&release.tag_name).expect("release version was validated")
        })
        .ok_or_else(|| "No compatible downloadable NES module release was found.".to_owned())?;
    let exe_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == EXE_NAME)
        .expect("release asset presence was checked");
    let checksum_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == CHECKSUM_NAME)
        .expect("release checksum presence was checked");
    if exe_asset.size == 0
        || exe_asset.size > MAX_MODULE_BYTES
        || !official_release_asset_url(&exe_asset.browser_download_url, &release.tag_name)
        || !official_release_asset_url(&checksum_asset.browser_download_url, &release.tag_name)
    {
        return Err("NES release contains invalid or oversized download metadata.".to_owned());
    }
    let exe_bytes = download_asset(&agent, &exe_asset.browser_download_url, exe_asset.size)?;
    let checksum_bytes = download_asset(&agent, &checksum_asset.browser_download_url, 1024)?;
    let checksum_text = std::str::from_utf8(&checksum_bytes)
        .map_err(|_| "NES release checksum is not valid UTF-8.".to_owned())?;
    let expected = checksum_text
        .split_whitespace()
        .next()
        .ok_or_else(|| "NES release checksum is empty.".to_owned())?;
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("NES release checksum has an invalid SHA-256 digest.".to_owned());
    }
    let actual = format!("{:x}", Sha256::digest(&exe_bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err("NES module SHA-256 verification failed; nothing was installed.".to_owned());
    }

    let destination = module_root()?.join(&release.tag_name);
    fs::create_dir_all(&destination)
        .map_err(|error| format!("Could not create NES module directory: {error}"))?;
    let final_path = destination.join(EXE_NAME);
    if final_path.is_file() {
        let existing = fs::read(&final_path)
            .map_err(|error| format!("Could not verify the installed NES module: {error}"))?;
        if format!("{:x}", Sha256::digest(&existing)).eq_ignore_ascii_case(expected) {
            return Ok(final_path);
        }
        return Err(
            "An installed NES module with this version tag has a bad checksum; remove it before retrying."
                .to_owned(),
        );
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("System clock is invalid: {error}"))?
        .as_nanos();
    let staging = destination.join(format!(".module-{nonce}.part"));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .map_err(|error| format!("Could not stage NES module: {error}"))?;
        file.write_all(&exe_bytes)
            .map_err(|error| format!("Could not write NES module: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Could not flush NES module: {error}"))?;
        fs::rename(&staging, &final_path)
            .map_err(|error| format!("Could not activate NES module: {error}"))?;
        Ok(final_path.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result
}

fn official_release_asset_url(url: &str, tag: &str) -> bool {
    url.starts_with(&format!(
        "https://github.com/Likounsee/PeaceEmu-NES/releases/download/{tag}/"
    )) && !url
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace() || byte == b'\\')
}

fn download_asset(agent: &ureq::Agent, url: &str, maximum: u64) -> Result<Vec<u8>, String> {
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| format!("Could not download NES module asset: {error}"))?;
    let final_url = response.get_uri().to_string();
    if !(final_url.starts_with("https://github.com/")
        || final_url.starts_with("https://release-assets.githubusercontent.com/")
        || final_url.starts_with("https://objects.githubusercontent.com/"))
    {
        return Err("NES module download redirected to an untrusted host.".to_owned());
    }
    if let Some(length) = response
        .headers()
        .get("Content-Length")
        .and_then(|value| value.to_str().ok())
    {
        let length = length
            .parse::<u64>()
            .map_err(|_| "NES module has an invalid Content-Length.".to_owned())?;
        if length == 0 || length > maximum {
            return Err("NES module asset exceeds the accepted download size.".to_owned());
        }
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read NES module asset: {error}"))?;
    if bytes.is_empty() || bytes.len() as u64 > maximum {
        return Err("NES module asset is empty or exceeds the accepted download size.".to_owned());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_orders_canonical_semantic_release_versions() {
        assert_eq!(version_number("nes-module-0.1.7"), Some((0, 1, 7)));
        assert_eq!(version_number("nes-module-0.1.0"), Some((0, 1, 0)));
        assert_eq!(version_number("nes-module-0.2.1"), Some((0, 2, 1)));
        assert_eq!(version_number("nes-module-1.0.0"), Some((1, 0, 0)));
        assert!(version_number("nes-module-0.10.0") > version_number("nes-module-0.2.9"));
        assert_eq!(version_number("latest"), None);
        assert_eq!(version_number("nes-module-0.01.2"), None);
        assert_eq!(version_number("nes-module-0.1"), None);
        assert_eq!(version_number("nes-module-0.1.2-extra"), None);
    }

    #[test]
    fn only_accepts_assets_from_the_expected_repository_and_tag() {
        assert!(official_release_asset_url(
            "https://github.com/Likounsee/PeaceEmu-NES/releases/download/nes-module-0.1.3/peaceemu-nes-desktop.exe",
            "nes-module-0.1.3"
        ));
        assert!(!official_release_asset_url(
            "https://example.com/peaceemu-nes-desktop.exe",
            "nes-module-0.1.3"
        ));
    }
}
