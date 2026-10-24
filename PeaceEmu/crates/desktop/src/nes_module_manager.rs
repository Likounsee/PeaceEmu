//! Optional NES module download and installation.
#![forbid(unsafe_code)]

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;
use ureq::ResponseExt;

const MODULE_CATALOG: &str = include_str!("../assets/modules/catalog.json");
const MAX_RELEASES_BYTES: u64 = 2 * 1024 * 1024;
const EXE_NAME: &str = "peaceemu-nes-desktop.exe";
const CHECKSUM_NAME: &str = "peaceemu-nes-desktop.exe.sha256";

#[derive(Deserialize)]
struct ModuleCatalog {
    schema_version: u32,
    modules: Vec<ModuleDescriptor>,
}

#[derive(Clone, Deserialize)]
struct ModuleDescriptor {
    id: String,
    repository: String,
    release_api: String,
    tag_prefix: String,
    minimum_peaceemu_version: String,
    platforms: Vec<String>,
    assets: ModuleAssets,
    max_download_bytes: u64,
}

#[derive(Clone, Deserialize)]
struct ModuleAssets {
    executable: String,
    sha256: String,
}

fn nes_module_descriptor() -> Result<ModuleDescriptor, String> {
    let catalog: ModuleCatalog = serde_json::from_str(MODULE_CATALOG)
        .map_err(|error| format!("Embedded module catalogue is invalid: {error}"))?;
    if catalog.schema_version != 1 {
        return Err("Unsupported embedded module catalogue schema.".to_owned());
    }
    let module = catalog
        .modules
        .into_iter()
        .find(|module| module.id == "nes")
        .ok_or_else(|| "The NES module is missing from the module catalogue.".to_owned())?;
    let expected_api = format!(
        "https://api.github.com/repos/{}/releases?per_page=20",
        module.repository
    );
    if module.repository.split('/').count() != 2
        || module
            .repository
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_')))
        || module.release_api != expected_api
        || module.tag_prefix != "nes-module-"
        || parse_semver(&module.minimum_peaceemu_version).is_none()
        || module.platforms.is_empty()
        || module.assets.executable != EXE_NAME
        || module.assets.sha256 != CHECKSUM_NAME
        || module.max_download_bytes == 0
        || module.max_download_bytes > 100 * 1024 * 1024
    {
        return Err("The NES module catalogue entry failed validation.".to_owned());
    }
    Ok(module)
}

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
    version_number_with_prefix(tag, "nes-module-")
}

fn version_number_with_prefix(tag: &str, prefix: &str) -> Option<(u64, u64, u64)> {
    let version = tag.strip_prefix(prefix)?;
    let mut parts = version.split('.');
    let major = parse_version_component(parts.next()?)?;
    let minor = parse_version_component(parts.next()?)?;
    let patch = parse_version_component(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn parse_semver(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let major = parse_version_component(parts.next()?)?;
    let minor = parse_version_component(parts.next()?)?;
    let patch = parse_version_component(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn current_platform_id() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x86_64"
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "windows-aarch64"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x86_64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-aarch64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "macos-x86_64"
    } else {
        "unsupported"
    }
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

/// Returns the newest installed NES executable whose persisted digest still matches.
pub(crate) fn installed_executable_path() -> Option<PathBuf> {
    let root = module_root().ok()?;
    let mut versions = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let number = version_number(&name)?;
            let exe = entry.path().join(EXE_NAME);
            let checksum = entry.path().join(CHECKSUM_NAME);
            let bytes = fs::read(&exe).ok()?;
            let expected = fs::read_to_string(checksum).ok()?;
            checksum_matches(&bytes, &expected).then_some((number, exe))
        })
        .collect::<Vec<_>>();
    versions.sort_by_key(|(number, _)| *number);
    versions.pop().map(|(_, path)| path)
}

fn checksum_matches(bytes: &[u8], checksum_text: &str) -> bool {
    let Some(expected) = checksum_text.split_whitespace().next() else {
        return false;
    };
    expected.len() == 64
        && expected.bytes().all(|byte| byte.is_ascii_hexdigit())
        && format!("{:x}", Sha256::digest(bytes)).eq_ignore_ascii_case(expected)
}

fn persist_checksum(directory: &std::path::Path, expected: &str) -> Result<(), String> {
    let checksum_path = directory.join(CHECKSUM_NAME);
    let temporary_path = directory.join(format!(".{CHECKSUM_NAME}.part"));
    if fs::read_to_string(&checksum_path)
        .ok()
        .and_then(|contents| contents.split_whitespace().next().map(str::to_owned))
        .is_some_and(|installed| installed.eq_ignore_ascii_case(expected))
    {
        return Ok(());
    }
    fs::write(&temporary_path, format!("{expected}  {EXE_NAME}\n"))
        .map_err(|error| format!("Could not persist NES module checksum: {error}"))?;
    if checksum_path.exists() {
        fs::remove_file(&checksum_path)
            .map_err(|error| format!("Could not replace NES module checksum: {error}"))?;
    }
    fs::rename(&temporary_path, &checksum_path)
        .map_err(|error| format!("Could not activate NES module checksum: {error}"))?;
    Ok(())
}
/// Downloads the newest published module release, verifies its SHA-256 sidecar,
/// and installs it into a version-specific directory without overwriting prior builds.
pub(crate) fn install_latest() -> Result<PathBuf, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_body(Some(Duration::from_secs(20)))
        .build()
        .into();
    let module = nes_module_descriptor()?;
    let platform = current_platform_id();
    if !module
        .platforms
        .iter()
        .any(|supported| supported == platform)
    {
        return Err(format!(
            "The NES module is not available for this platform yet ({platform})."
        ));
    }
    let app_version = parse_semver(env!("CARGO_PKG_VERSION"))
        .expect("the desktop package version must be valid semantic versioning");
    let minimum_version = parse_semver(&module.minimum_peaceemu_version)
        .expect("the validated catalogue minimum version must be valid");
    if app_version < minimum_version {
        return Err(format!(
            "The NES module requires PeaceEmu {} or newer.",
            module.minimum_peaceemu_version
        ));
    }
    let mut response = agent
        .get(&module.release_api)
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
        .filter(|release| {
            !release.draft
                && version_number_with_prefix(&release.tag_name, &module.tag_prefix).is_some()
        })
        .filter(|release| {
            release
                .assets
                .iter()
                .any(|asset| asset.name == module.assets.executable)
                && release
                    .assets
                    .iter()
                    .any(|asset| asset.name == module.assets.sha256)
        })
        .max_by_key(|release| {
            version_number_with_prefix(&release.tag_name, &module.tag_prefix)
                .expect("release version was validated")
        })
        .ok_or_else(|| "No compatible downloadable NES module release was found.".to_owned())?;
    let exe_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == module.assets.executable)
        .expect("release asset presence was checked");
    let checksum_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == module.assets.sha256)
        .expect("release checksum presence was checked");
    if exe_asset.size == 0
        || exe_asset.size > module.max_download_bytes
        || !official_release_asset_url(
            &exe_asset.browser_download_url,
            &module.repository,
            &release.tag_name,
        )
        || !official_release_asset_url(
            &checksum_asset.browser_download_url,
            &module.repository,
            &release.tag_name,
        )
    {
        return Err("NES release contains invalid or oversized download metadata.".to_owned());
    }
    let exe_bytes = download_asset(
        &agent,
        &exe_asset.browser_download_url,
        module.max_download_bytes,
    )?;
    if exe_bytes.len() as u64 != exe_asset.size {
        return Err("NES module download size does not match the release metadata.".to_owned());
    }
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
        if checksum_matches(&existing, expected) {
            persist_checksum(&destination, expected)?;
            return Ok(final_path);
        }
        // The downloaded bytes have already passed the release SHA-256 check.
        // Replace a damaged same-version executable instead of leaving updates stuck.
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
        if final_path.exists() {
            fs::remove_file(&final_path)
                .map_err(|error| format!("Could not replace damaged NES module: {error}"))?;
        }
        fs::rename(&staging, &final_path)
            .map_err(|error| format!("Could not activate NES module: {error}"))?;
        Ok(final_path.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    let installed = result?;
    persist_checksum(&destination, expected)?;
    Ok(installed)
}

fn official_release_asset_url(url: &str, repository: &str, tag: &str) -> bool {
    url.starts_with(&format!(
        "https://github.com/{repository}/releases/download/{tag}/"
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
    fn installed_module_checksum_must_match_the_binary() {
        let bytes = b"verified module bytes";
        let digest = format!("{:x}", Sha256::digest(bytes));
        assert!(checksum_matches(bytes, &format!("{digest}  {EXE_NAME}")));
        assert!(!checksum_matches(
            b"tampered module bytes",
            &format!("{digest}  {EXE_NAME}")
        ));
        assert!(!checksum_matches(bytes, "not-a-sha256"));
        assert!(!checksum_matches(bytes, ""));
    }

    #[test]
    fn checksum_marker_is_written_reused_and_repaired() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must be valid")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("peaceemu-nes-checksum-{nonce}"));
        fs::create_dir_all(&directory).expect("temporary test directory must be created");
        let expected = format!("{:x}", Sha256::digest(b"module bytes"));
        let checksum_path = directory.join(CHECKSUM_NAME);

        persist_checksum(&directory, &expected).expect("checksum must be persisted");
        assert!(checksum_matches(
            b"module bytes",
            &fs::read_to_string(&checksum_path).expect("checksum must be readable")
        ));

        persist_checksum(&directory, &expected).expect("matching checksum should be reused");
        fs::write(&checksum_path, "broken").expect("checksum marker should be corrupted");
        persist_checksum(&directory, &expected).expect("corrupt checksum should be repaired");
        assert!(checksum_matches(
            b"module bytes",
            &fs::read_to_string(&checksum_path).expect("repaired checksum must be readable")
        ));
        fs::remove_dir_all(directory).expect("temporary test directory must be removed");
    }

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
    fn embedded_module_catalogue_is_versioned_and_contains_nes() {
        let module = nes_module_descriptor().expect("checked-in NES module entry must be valid");
        assert_eq!(module.id, "nes");
        assert_eq!(module.repository, "Likounsee/PeaceEmu-NES");
        assert_eq!(
            module.release_api,
            "https://api.github.com/repos/Likounsee/PeaceEmu-NES/releases?per_page=20"
        );
        assert_eq!(module.tag_prefix, "nes-module-");
        assert_eq!(module.minimum_peaceemu_version, "0.2.3");
        assert!(module
            .platforms
            .iter()
            .any(|platform| platform == "windows-x86_64"));
        assert_eq!(module.assets.executable, EXE_NAME);
        assert_eq!(module.assets.sha256, CHECKSUM_NAME);
        assert!(module.max_download_bytes > 0);
    }

    #[test]
    fn semantic_version_parser_rejects_noncanonical_versions() {
        assert_eq!(parse_semver("0.2.3"), Some((0, 2, 3)));
        assert_eq!(parse_semver("1.10.0"), Some((1, 10, 0)));
        assert_eq!(parse_semver("0.02.3"), None);
        assert_eq!(parse_semver("0.2"), None);
        assert_eq!(parse_semver("0.2.3-beta"), None);
    }

    #[test]
    fn catalogue_version_parser_uses_the_declared_tag_prefix() {
        assert_eq!(
            version_number_with_prefix("nes-module-0.3.1", "nes-module-"),
            Some((0, 3, 1))
        );
        assert_eq!(
            version_number_with_prefix("gba-module-0.3.1", "nes-module-"),
            None
        );
    }

    #[test]
    fn only_accepts_assets_from_the_expected_repository_and_tag() {
        assert!(official_release_asset_url(
            "https://github.com/Likounsee/PeaceEmu-NES/releases/download/nes-module-0.1.3/peaceemu-nes-desktop.exe",
            "Likounsee/PeaceEmu-NES",
            "nes-module-0.1.3"
        ));
        assert!(!official_release_asset_url(
            "https://example.com/peaceemu-nes-desktop.exe",
            "Likounsee/PeaceEmu-NES",
            "nes-module-0.1.3"
        ));
        assert!(!official_release_asset_url(
            "https://github.com/evil/Repo/releases/download/nes-module-0.1.3/peaceemu-nes-desktop.exe",
            "Likounsee/PeaceEmu-NES",
            "nes-module-0.1.3"
        ));
    }
}
