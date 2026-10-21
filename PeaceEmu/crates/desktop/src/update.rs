#![forbid(unsafe_code)]

//! Application update metadata and staged-artifact integrity checks.
//! This is deliberately separate from module downloads and activation.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use ureq::ResponseExt;

pub(crate) const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/Likounsee/PeaceEmu/PeaceEmu/PeaceEmu/update/stable.json";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateManifest {
    pub(crate) schema_version: u32,
    pub(crate) version: String,
    pub(crate) release_notes_url: String,
    #[serde(default)]
    pub(crate) downloads: BTreeMap<String, UpdateDownload>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateDownload {
    pub(crate) url: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
}

impl UpdateManifest {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "Unsupported update manifest schema: {}",
                self.schema_version
            ));
        }
        parse_version(&self.version)
            .ok_or_else(|| format!("Invalid application version: {}", self.version))?;
        validate_release_notes_url(&self.release_notes_url)?;
        for (platform, download) in &self.downloads {
            if !is_supported_platform_key(platform) {
                return Err(format!("Unsupported update platform key: {platform}."));
            }
            validate_download_url_for_version(&download.url, &self.version)?;
            if download.size_bytes == 0 || download.size_bytes > MAX_DOWNLOAD_BYTES {
                return Err(format!("Invalid download size for platform {platform}."));
            }
            if download.sha256.len() != 64
                || !download.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(format!("Invalid SHA-256 digest for platform {platform}."));
            }
        }
        Ok(())
    }

    pub(crate) fn download_for_current_platform(&self) -> Option<&UpdateDownload> {
        self.downloads.get(current_platform_key())
    }
}

pub(crate) fn fetch_manifest() -> Result<UpdateManifest, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_recv_body(Some(Duration::from_secs(5)))
        .build()
        .into();
    let mut response = agent
        .get(MANIFEST_URL)
        .call()
        .map_err(|error| format!("Could not reach the official update manifest: {error}"))?;
    let mut body = String::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|error| format!("Could not read update manifest: {error}"))?;
    if body.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("Update manifest exceeds the 1 MiB safety limit.".to_owned());
    }
    let manifest: UpdateManifest = serde_json::from_str(&body)
        .map_err(|error| format!("Official update manifest is invalid: {error}"))?;
    manifest.validate()?;
    Ok(manifest)
}

/// Downloads a published application artifact into a private staging directory,
/// verifies its declared length and SHA-256, and never executes or installs it.
pub(crate) fn download_and_verify(
    download: &UpdateDownload,
    version: &str,
) -> Result<PathBuf, String> {
    parse_version(version)
        .ok_or_else(|| format!("Invalid application version for update staging: {version}"))?;
    validate_download_url_for_version(&download.url, version)?;
    if download.size_bytes == 0 || download.size_bytes > MAX_DOWNLOAD_BYTES {
        return Err("Update download size is outside the accepted limits.".to_owned());
    }
    if download.sha256.len() != 64 || !download.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Update download has a malformed SHA-256 digest.".to_owned());
    }

    let application_directory = std::env::temp_dir().join("PeaceEmu");
    fs::create_dir_all(&application_directory)
        .map_err(|error| format!("Could not create update application directory: {error}"))?;
    ensure_real_staging_directory(&application_directory)?;

    let directory = application_directory.join("updates");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create update staging directory: {error}"))?;
    ensure_real_staging_directory(&directory)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("System clock is invalid: {error}"))?
        .as_nanos();
    let safe_version: String = version
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '.' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    let partial_path = directory.join(format!("desktop-{safe_version}-{nonce}.part"));
    let verified_path = directory.join(format!("desktop-{safe_version}-{nonce}.verified"));

    // Only clean up paths that this invocation successfully created. A collision on
    // either create_new() or rename() must never cause us to delete a pre-existing file.
    let mut partial_created = false;
    let mut verified_created = false;
    let result = (|| {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_body(Some(Duration::from_secs(30)))
            .build()
            .into();
        let mut response = agent
            .get(&download.url)
            .call()
            .map_err(|error| format!("Could not download the update artifact: {error}"))?;
        // GitHub release URLs commonly redirect to release-assets.githubusercontent.com.
        // Validate the final destination too, rather than trusting only the manifest URL.
        validate_download_url(&response.get_uri().to_string())?;
        validate_content_length(
            response
                .headers()
                .get("Content-Length")
                .and_then(|value| value.to_str().ok()),
            download.size_bytes,
        )?;
        let mut reader = response.body_mut().as_reader();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial_path)
            .map_err(|error| format!("Could not create staged update file: {error}"))?;
        partial_created = true;
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = reader
                .read(&mut buffer)
                .map_err(|error| format!("Update download was interrupted: {error}"))?;
            if count == 0 {
                break;
            }
            total = total.saturating_add(count as u64);
            if total > download.size_bytes || total > MAX_DOWNLOAD_BYTES {
                return Err("Update download exceeded its declared size limit.".to_owned());
            }
            file.write_all(&buffer[..count])
                .map_err(|error| format!("Could not write staged update: {error}"))?;
            hasher.update(&buffer[..count]);
        }
        file.sync_all()
            .map_err(|error| format!("Could not flush staged update: {error}"))?;
        if total != download.size_bytes {
            return Err(format!(
                "Update download size mismatch (expected {} bytes, received {total}).",
                download.size_bytes
            ));
        }
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(&download.sha256) {
            return Err(
                "Downloaded update SHA-256 does not match the official manifest.".to_owned(),
            );
        }
        // A rename may replace an existing destination on some platforms. Creating
        // a hard link is atomic and fails if the verified path already exists.
        create_verified_link(&partial_path, &verified_path)?;
        verified_created = true;
        fs::remove_file(&partial_path).map_err(|error| {
            format!("Could not remove temporary update file after verification: {error}")
        })?;
        partial_created = false;
        Ok(verified_path.clone())
    })();

    if result.is_err() {
        if partial_created {
            let _ = fs::remove_file(&partial_path);
        }
        if verified_created {
            let _ = fs::remove_file(&verified_path);
        }
    }
    result
}

/// Reject a contradictory or malformed HTTP length before creating a staged file.
fn validate_content_length(header: Option<&str>, expected_size: u64) -> Result<(), String> {
    let Some(header) = header else {
        // Chunked or otherwise length-less responses are still bounded by the streaming check.
        return Ok(());
    };
    let declared_size = header
        .parse::<u64>()
        .map_err(|_| "Update server returned an invalid Content-Length header.".to_owned())?;
    if declared_size != expected_size {
        return Err(format!(
            "Update server Content-Length mismatch (expected {expected_size} bytes, declared {declared_size})."
        ));
    }
    Ok(())
}

fn ensure_real_staging_directory(directory: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("Could not inspect update staging directory: {error}"))?;
    if !metadata.file_type().is_dir() {
        return Err(
            "Update staging path must be a real directory, not a symbolic link or file.".to_owned(),
        );
    }
    Ok(())
}

fn create_verified_link(partial_path: &Path, verified_path: &Path) -> Result<(), String> {
    fs::hard_link(partial_path, verified_path).map_err(|error| {
        format!(
            "Could not finalize verified update file without overwriting an existing file: {error}"
        )
    })
}

fn validate_release_notes_url(url: &str) -> Result<(), String> {
    validate_https_url(url, "release notes")?;
    if url.contains('?') || url.contains('#') {
        return Err("Release notes URL must not contain a query or fragment.".to_owned());
    }
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| "Release notes must use HTTPS.".to_owned())?;
    let (authority, path) = rest
        .split_once('/')
        .map(|(authority, path)| (authority, format!("/{path}")))
        .unwrap_or((rest, String::new()));
    if !authority.eq_ignore_ascii_case("github.com")
        || authority.contains('@')
        || authority.contains(':')
        || !(path == "/Likounsee/PeaceEmu/releases"
            || path.starts_with("/Likounsee/PeaceEmu/releases/"))
    {
        return Err(
            "Release notes must point to the official PeaceEmu GitHub releases page.".to_owned(),
        );
    }
    // Keep the path canonical so encoded separators or dot-segments cannot
    // redirect the browser to a different GitHub route than the one validated.
    let lower_path = path.to_ascii_lowercase();
    if lower_path.contains("%2e")
        || lower_path.contains("%2f")
        || lower_path.contains("%5c")
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|segment| segment == "." || segment == "..")
    {
        return Err(
            "Release notes URL must not contain encoded separators, control characters, or path traversal segments."
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_download_url(url: &str) -> Result<(), String> {
    validate_https_url(url, "download")?;
    // Do not normalize user-info or ports away before checking the host: doing so
    // can make a URL whose actual destination is untrusted appear to be GitHub-hosted.
    let authority = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default();
    if authority.contains('@') || authority.contains(':') {
        return Err(
            "Update artifact URLs must use an official GitHub host without credentials or custom ports."
                .to_owned(),
        );
    }
    let authority = authority.to_ascii_lowercase();
    if !matches!(
        authority.as_str(),
        "github.com" | "objects.githubusercontent.com" | "release-assets.githubusercontent.com"
    ) {
        return Err(
            "Update artifacts must be hosted on official GitHub release domains.".to_owned(),
        );
    }
    // A github.com URL must identify this project's release asset, not merely
    // any page or release in any repository on the trusted domain. Redirected
    // release-assets.githubusercontent.com URLs are checked by the host allowlist.
    if authority == "github.com" {
        let path = url
            .strip_prefix("https://")
            .and_then(|rest| rest.split_once('/').map(|(_, path)| format!("/{path}")))
            .unwrap_or_default();
        if !path.starts_with("/Likounsee/PeaceEmu/releases/download/") {
            return Err("Update artifacts must come from an PeaceEmu GitHub release.".to_owned());
        }
        // Manifest artifact URLs are stable release-asset paths, not signed
        // redirect URLs. Reject query/fragment tricks and dot-segments rather
        // than letting URL/server path normalization change the requested asset.
        if path.contains('?') || path.contains('#') {
            return Err(
                "PeaceEmu release artifact URLs must not contain a query or fragment.".to_owned(),
            );
        }
        let lower_path = path.to_ascii_lowercase();
        // Encoded separators and backslashes can be interpreted differently by
        // proxies and origin servers. Reject them alongside encoded dot segments
        // so validation and the eventual HTTP request agree on path boundaries.
        if lower_path.contains("%2e")
            || lower_path.contains("%2f")
            || lower_path.contains("%5c")
            || path.contains('\\')
            || path.chars().any(char::is_control)
            || path
                .split('/')
                .any(|segment| segment == "." || segment == "..")
        {
            return Err("PeaceEmu release artifact URLs must not contain encoded separators, control characters, or path traversal segments.".to_owned());
        }
        let segments: Vec<_> = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();
        if segments.len() < 6
            || segments[4].is_empty()
            || segments[5..].iter().any(|s| s.is_empty())
        {
            return Err(
                "PeaceEmu release artifact URL must identify a specific release asset.".to_owned(),
            );
        }
    }
    Ok(())
}

/// Ensure a GitHub release asset is tagged with the same version as its manifest.
fn validate_download_url_for_version(url: &str, version: &str) -> Result<(), String> {
    validate_download_url(url)?;
    let Some(rest) = url.strip_prefix("https://") else {
        return Err("Update artifact URL must use HTTPS.".to_owned());
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.eq_ignore_ascii_case("github.com") {
        let path = rest
            .split_once('/')
            .map(|(_, path)| path)
            .unwrap_or_default();
        let segments: Vec<_> = path.split('/').collect();
        let expected_tag = format!("v{version}");
        if segments.get(4).copied() != Some(expected_tag.as_str()) {
            return Err(format!(
                "Update artifact release tag does not match manifest version {version}."
            ));
        }
    }
    Ok(())
}

pub(crate) fn is_newer_version(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let parse_component = |part: &str| {
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        part.parse().ok()
    };
    let major = parse_component(parts.next()?)?;
    let minor = parse_component(parts.next()?)?;
    let patch = parse_component(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn validate_https_url(url: &str, label: &str) -> Result<(), String> {
    if url
        .chars()
        .any(|character| character.is_control() || character == char::from(92))
    {
        return Err(format!(
            "The {label} URL contains a forbidden control character or backslash."
        ));
    }
    let authority = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .unwrap_or_default();
    if authority.is_empty()
        || authority.starts_with('.')
        || authority.ends_with('.')
        || authority.contains('@')
        || authority.chars().any(char::is_whitespace)
    {
        return Err(format!(
            "The {label} URL must use HTTPS and contain a valid host."
        ));
    }
    Ok(())
}

/// Launches a verified Windows installer after this app exits. Successful installs remove the installer.
#[cfg(target_os = "windows")]
pub(crate) fn launch_verified_windows_installer(
    path: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Stdio};

    let staging_dir = std::env::temp_dir().join("PeaceEmu").join("updates");
    let staged_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if path.parent() != Some(staging_dir.as_path()) || !staged_name.ends_with(".verified") {
        return Err(
            "The staged update is outside the expected private staging directory.".to_owned(),
        );
    }
    let powershell = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .ok_or_else(|| "Could not locate the Windows system directory.".to_owned())?
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    if !powershell.is_file() {
        return Err("Windows PowerShell was not found at its expected system path.".to_owned());
    }
    verify_staged_file(path, expected_size, expected_sha256)?;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("System clock is invalid: {error}"))?
        .as_nanos();
    let installer_path = staging_dir.join(format!("PeaceEmu-Update-{nonce}.exe"));
    let script_path = staging_dir.join(format!("install-update-{nonce}.ps1"));
    if installer_path.exists() || script_path.exists() {
        return Err(
            "A temporary update helper path already exists; refusing to overwrite it.".to_owned(),
        );
    }
    fs::rename(path, &installer_path)
        .map_err(|error| format!("Could not prepare the verified installer: {error}"))?;

    let script = r#"
param([string]$InstallerPath, [int]$ParentPid, [string]$ScriptPath)
$ErrorActionPreference = 'Stop'
try {
    $deadline = [DateTime]::UtcNow.AddMinutes(2)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (-not (Get-Process -Id $ParentPid -ErrorAction SilentlyContinue)) { break }
        Start-Sleep -Milliseconds 200
    }
    if (Get-Process -Id $ParentPid -ErrorAction SilentlyContinue) { exit 20 }
    $setup = Start-Process -FilePath $InstallerPath -Wait -PassThru
    if ($setup.ExitCode -eq 0) {
        for ($attempt = 0; $attempt -lt 10; $attempt++) {
            try {
                Remove-Item -LiteralPath $InstallerPath -Force
                break
            } catch {
                Start-Sleep -Milliseconds 300
            }
        }
    }
} finally {
    Remove-Item -LiteralPath $ScriptPath -Force -ErrorAction SilentlyContinue
}
"#;

    let write_result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&script_path)
            .map_err(|error| format!("Could not create the update launcher: {error}"))?;
        file.write_all(script.as_bytes())
            .map_err(|error| format!("Could not write the update launcher: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Could not flush the update launcher: {error}"))?;

        let mut command = Command::new(&powershell);
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-File",
            ])
            .arg(&script_path)
            .arg("-InstallerPath")
            .arg(&installer_path)
            .arg("-ParentPid")
            .arg(std::process::id().to_string())
            .arg("-ScriptPath")
            .arg(&script_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000);
        command
            .spawn()
            .map_err(|error| format!("Could not start the Windows update installer: {error}"))?;
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = fs::remove_file(&script_path);
        let _ = fs::rename(&installer_path, path);
        return Err(error);
    }
    Ok(())
}

/// Verifies a staged update without executing it or changing installed files.
#[allow(dead_code)] // Used by the Windows-only installer launcher on Windows builds.
pub(crate) fn verify_staged_file(
    path: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    if expected_size == 0 || expected_size > MAX_DOWNLOAD_BYTES {
        return Err("Staged update size is outside the accepted limits.".to_owned());
    }
    if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Expected SHA-256 digest is malformed.".to_owned());
    }
    // Do not follow a symlink supplied in place of the staged artifact. Verification
    // must hash the regular file at this exact path, not an arbitrary target.
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect staged update: {error}"))?;
    if !metadata.file_type().is_file() || metadata.len() != expected_size {
        return Err(format!(
            "Staged update size mismatch (expected {expected_size} bytes, found {}).",
            metadata.len()
        ));
    }
    let mut file =
        File::open(path).map_err(|error| format!("Could not open staged update: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not hash staged update: {error}"))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err("Staged update SHA-256 does not match the trusted manifest.".to_owned());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn write_test_file(path: &Path, contents: &[u8]) {
    let mut file = File::create(path).expect("test file should be created");
    file.write_all(contents)
        .expect("test file should be written");
}

fn is_supported_platform_key(platform: &str) -> bool {
    matches!(
        platform,
        "windows-x86_64" | "macos-aarch64" | "macos-x86_64" | "linux-x86_64"
    )
}

fn current_platform_key() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x86_64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-aarch64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "macos-x86_64"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x86_64"
    } else {
        "unsupported"
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::ensure_real_staging_directory;
    use super::{
        is_newer_version, validate_content_length, validate_download_url, verify_staged_file,
        write_test_file, UpdateManifest,
    };
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_file() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        std::env::temp_dir().join(format!("desktop-update-{}-{nonce}.bin", std::process::id()))
    }

    #[test]
    fn finalizing_verified_update_never_overwrites_an_existing_destination() {
        let source = temporary_file();
        let mut destination = temporary_file();
        destination.set_extension("existing");
        write_test_file(&source, b"verified update");
        write_test_file(&destination, b"keep this file");

        let result = super::create_verified_link(&source, &destination);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(&destination).expect("existing destination should remain"),
            b"keep this file"
        );
        assert_eq!(
            std::fs::read(&source).expect("source should remain after collision"),
            b"verified update"
        );
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(destination);
    }

    #[test]
    fn finalizing_verified_update_preserves_contents_at_the_new_path() {
        let source = temporary_file();
        let mut destination = temporary_file();
        destination.set_extension("verified");
        write_test_file(&source, b"verified update");

        super::create_verified_link(&source, &destination)
            .expect("new verified destination should be created");

        assert_eq!(
            std::fs::read(&destination).expect("verified destination should be readable"),
            b"verified update"
        );
        assert_eq!(
            std::fs::read(&source).expect("source should remain until caller removes it"),
            b"verified update"
        );
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(destination);
    }

    #[test]
    fn validates_a_manifest_without_publishing_unavailable_downloads() {
        let manifest: UpdateManifest = serde_json::from_str(
            r#"{"schema_version":1,"version":"0.1.0","release_notes_url":"https://github.com/Likounsee/PeaceEmu/releases","downloads":{}}"#,
        )
        .expect("manifest JSON should parse");
        manifest.validate().expect("empty release channel is valid");
        assert!(manifest.download_for_current_platform().is_none());
    }

    #[test]
    fn accepts_each_supported_update_platform_key() {
        for platform in [
            "windows-x86_64",
            "macos-aarch64",
            "macos-x86_64",
            "linux-x86_64",
        ] {
            let manifest: UpdateManifest = serde_json::from_value(serde_json::json!({
                "schema_version": 1,
                "version": "0.2.0",
                "release_notes_url": "https://github.com/Likounsee/PeaceEmu/releases",
                "downloads": {
                    (platform): {
                        "url": "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/app",
                        "size_bytes": 1,
                        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    }
                }
            }))
            .expect("manifest JSON should parse");
            manifest
                .validate()
                .expect("supported platform key should be accepted");
        }
    }

    #[test]
    fn rejects_unknown_update_platform_keys_instead_of_silently_ignoring_them() {
        let manifest: UpdateManifest = serde_json::from_str(
            r#"{"schema_version":1,"version":"0.2.0","release_notes_url":"https://github.com/Likounsee/PeaceEmu/releases","downloads":{"linux-x64":{"url":"https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/app","size_bytes":1,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}"#,
        )
        .expect("manifest JSON should parse");
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn rejects_unknown_manifest_and_download_fields() {
        let unknown_manifest_field = serde_json::from_str::<UpdateManifest>(
            r#"{"schema_version":1,"version":"0.2.0","release_notes_url":"https://github.com/Likounsee/PeaceEmu/releases","downloads":{},"unexpected":true}"#,
        );
        assert!(unknown_manifest_field.is_err());

        let unknown_download_field = serde_json::from_str::<UpdateManifest>(
            r#"{"schema_version":1,"version":"0.2.0","release_notes_url":"https://github.com/Likounsee/PeaceEmu/releases","downloads":{"linux-x86_64":{"url":"https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/app","size_bytes":1,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","signature":"ignored-by-old-clients"}}}"#,
        );
        assert!(unknown_download_field.is_err());
    }

    #[test]
    fn rejects_release_notes_outside_the_official_project_releases() {
        for url in [
            "https://example.com/releases",
            "https://github.com/another/project/releases",
            "https://github.com/Likounsee/PeaceEmu/releases-evil",
            "https://attacker@github.com/Likounsee/PeaceEmu/releases",
            "https://github.com:8443/Likounsee/PeaceEmu/releases",
            "https://github.com/Likounsee/PeaceEmu/releases?redirect=example.com",
            "https://github.com/Likounsee/PeaceEmu/releases#other-project",
            "https://github.com/Likounsee/PeaceEmu/releases/../../another-project",
            "https://github.com/Likounsee/PeaceEmu/releases/%2e%2e/another-project",
            "https://github.com/Likounsee/PeaceEmu/releases%2f..%2fanother-project",
            "https://github.com/Likounsee/PeaceEmu/releases\\\\..\\\\another-project",
        ] {
            assert!(
                super::validate_release_notes_url(url).is_err(),
                "untrusted release notes URL should be rejected: {url}"
            );
        }
        assert!(super::validate_release_notes_url(
            "https://github.com/Likounsee/PeaceEmu/releases"
        )
        .is_ok());
        assert!(super::validate_release_notes_url(
            "https://github.com/Likounsee/PeaceEmu/releases/tag/v0.2.0"
        )
        .is_ok());
    }

    #[test]
    fn rejects_insecure_urls_and_malformed_hashes() {
        let manifest: UpdateManifest = serde_json::from_str(
            r#"{"schema_version":1,"version":"0.2.0","release_notes_url":"http://example.com/notes","downloads":{}}"#,
        )
        .expect("manifest JSON should parse");
        assert!(manifest.validate().is_err());
        let leading_zero_version: UpdateManifest = serde_json::from_str(
            r#"{"schema_version":1,"version":"01.2.3","release_notes_url":"https://github.com/Likounsee/PeaceEmu/releases","downloads":{}}"#,
        )
        .expect("manifest JSON should parse");
        assert!(leading_zero_version.validate().is_err());

        for malformed in [
            "https://",
            "https:///example.com/notes",
            "https://user@example.com/notes",
            "https://example.com\\\\@attacker.invalid/notes",
            "https://example.com/notes\\\\\\n",
        ] {
            assert!(
                super::validate_https_url(malformed, "release notes").is_err(),
                "malformed HTTPS URL should be rejected: {malformed}"
            );
        }
    }

    #[test]
    fn rejects_unsupported_manifest_schema_and_invalid_download_metadata() {
        let unsupported: UpdateManifest = serde_json::from_str(
            r#"{"schema_version":2,"version":"0.2.0","release_notes_url":"https://github.com/Likounsee/PeaceEmu/releases","downloads":{}}"#,
        )
        .expect("manifest JSON should parse");
        assert!(unsupported.validate().is_err());

        let valid_url = "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/app.exe";
        for (size, digest) in [
            (0, "a".repeat(64)),
            (1, "not-a-sha256-digest".to_owned()),
            (1, "g".repeat(64)),
        ] {
            let manifest: UpdateManifest = serde_json::from_value(serde_json::json!({
                "schema_version": 1,
                "version": "0.2.0",
                "release_notes_url": "https://github.com/Likounsee/PeaceEmu/releases",
                "downloads": {
                    "windows-x86_64": {
                        "url": valid_url,
                        "size_bytes": size,
                        "sha256": digest
                    }
                }
            }))
            .expect("manifest JSON should parse");
            assert!(
                manifest.validate().is_err(),
                "invalid size or digest must be rejected"
            );
        }
    }

    #[test]
    fn release_asset_tag_must_match_manifest_version() {
        let valid = "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/app.exe";
        assert!(super::validate_download_url_for_version(valid, "0.2.0").is_ok());
        assert!(super::validate_download_url_for_version(valid, "0.2.1").is_err());
        assert!(super::validate_download_url_for_version(valid, "1.0.0").is_err());
    }

    #[test]
    fn rejects_untrusted_update_download_hosts() {
        assert!(validate_download_url(
            "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/app.exe"
        )
        .is_ok());
        assert!(validate_download_url("https://example.com/app.exe").is_err());
        assert!(validate_download_url(
            "https://github.com/other/project/releases/download/v0.2.0/app.exe"
        )
        .is_err());
        assert!(
            validate_download_url("https://github.com/Likounsee/PeaceEmu/releases/tag/v0.2.0")
                .is_err()
        );
        for traversal in [
            "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/../other.exe",
            "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/%2e%2e/other.exe",
            "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/folder%2fapp.exe",
            "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/folder%5capp.exe",
            "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/folder\\\\app.exe",
            "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/app.exe?redirect=example.com",
            "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/app.exe#other",
        ] {
            assert!(
                validate_download_url(traversal).is_err(),
                "release asset URL must reject path/query/fragment tricks: {traversal}"
            );
        }
        assert!(validate_download_url("http://github.com/app.exe").is_err());
        // GitHub's release downloads can redirect to these dedicated asset hosts.
        assert!(validate_download_url(
            "https://release-assets.githubusercontent.com/github-production-release-asset/example"
        )
        .is_ok());
        assert!(validate_download_url(
            "https://objects.githubusercontent.com/github-production-release-asset/example"
        )
        .is_ok());
        // User-info, custom ports, and lookalike domains must be rejected even
        // when the URL claims to use a trusted redirect host.
        for untrusted in [
            "https://attacker@github.com/app.exe",
            "https://github.com:8443/app.exe",
            "https://github.com.evil.example/app.exe",
            "https://attacker@release-assets.githubusercontent.com/file",
            "https://release-assets.githubusercontent.com:8443/file",
            "https://release-assets.githubusercontent.com.evil.example/file",
            "https://objects.githubusercontent.com:443/file",
        ] {
            assert!(
                validate_download_url(untrusted).is_err(),
                "untrusted update destination should be rejected: {untrusted}"
            );
        }
    }

    #[test]
    fn compares_numeric_versions_without_lexicographic_errors() {
        assert!(is_newer_version("0.10.0", "0.9.9"));
        assert!(is_newer_version("1.0.0", "0.99.99"));
        assert!(!is_newer_version("0.1.0", "0.1.0"));
        for malformed in [
            "invalid",
            "1..3",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "+1.2.3",
            "1.2.-3",
            "1.2.3-beta",
            " 1.2.3",
        ] {
            assert!(
                !is_newer_version(malformed, "0.1.0"),
                "malformed version must not be considered newer: {malformed}"
            );
        }
    }

    #[test]
    fn update_staging_rejects_noncanonical_versions_before_network_access() {
        let download = super::UpdateDownload {
            url: "https://github.com/Likounsee/PeaceEmu/releases/download/v0.2.0/app.exe"
                .to_owned(),
            size_bytes: 1,
            sha256: "a".repeat(64),
        };

        for malformed in ["", "01.2.3", "1.2", "1.2.3-beta", "1.2.3.4"] {
            assert!(
                super::download_and_verify(&download, malformed).is_err(),
                "invalid version must be rejected before downloading: {malformed}"
            );
        }
    }

    #[test]
    fn validates_http_content_length_before_staging() {
        assert!(validate_content_length(None, 10).is_ok());
        assert!(validate_content_length(Some("10"), 10).is_ok());
        assert!(validate_content_length(Some("9"), 10).is_err());
        assert!(validate_content_length(Some("not-a-number"), 10).is_err());
        assert!(validate_content_length(Some("-1"), 10).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn update_staging_directory_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let target = temporary_file();
        let link = temporary_file().with_extension("staging-link");
        std::fs::create_dir(&target).expect("staging target directory should be created");
        symlink(&target, &link).expect("staging symbolic link should be created");

        assert!(ensure_real_staging_directory(&link).is_err());
        assert!(ensure_real_staging_directory(&target).is_ok());

        let _ = std::fs::remove_file(link);
        let _ = std::fs::remove_dir(target);
    }

    #[cfg(unix)]
    #[test]
    fn update_application_directory_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let target = temporary_file();
        let link = temporary_file().with_extension("application-link");
        std::fs::create_dir(&target).expect("target directory should be created");
        symlink(&target, &link).expect("application directory link should be created");

        assert!(ensure_real_staging_directory(&link).is_err());
        assert!(ensure_real_staging_directory(&target).is_ok());

        let _ = std::fs::remove_file(link);
        let _ = std::fs::remove_dir(target);
    }

    #[cfg(unix)]
    #[test]
    fn staged_file_verification_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let target = temporary_file();
        let link = temporary_file().with_extension("symlink");
        let contents = b"trusted staged update";
        write_test_file(&target, contents);
        symlink(&target, &link).expect("symbolic-link fixture should be created");
        let digest = format!("{:x}", Sha256::digest(contents));

        assert!(
            verify_staged_file(&link, contents.len() as u64, &digest).is_err(),
            "a staged update path must not resolve through a symbolic link"
        );

        let _ = std::fs::remove_file(link);
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn staged_file_must_match_size_and_sha256() {
        let path = temporary_file();
        let contents = b"trusted staged update";
        write_test_file(&path, contents);
        let digest = format!("{:x}", Sha256::digest(contents));
        verify_staged_file(&path, contents.len() as u64, &digest)
            .expect("valid staged file should pass");
        assert!(verify_staged_file(&path, contents.len() as u64 + 1, &digest).is_err());
        assert!(verify_staged_file(&path, contents.len() as u64, &"0".repeat(64)).is_err());

        // A staged file can be changed after download; re-verification must catch it.
        write_test_file(&path, b"tampered update contents");
        assert!(verify_staged_file(&path, contents.len() as u64, &digest).is_err());
        let _ = std::fs::remove_file(path);
    }
}
