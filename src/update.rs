//! Self-update from GitHub Releases.
//!
//! The latest release of [`REPO`] is read from the GitHub REST API. A release
//! is installable when its tag (`v1.2.3`) is newer than this build and it has
//! an `Inventory.exe` asset with a SHA-256 checksum (GitHub's own asset
//! `digest`, or an `Inventory.exe.sha256` asset). The download is verified
//! before the running executable is swapped with `self_replace`.
//!
//! This is the same approach as the `self_update` crate (which also uses
//! `self_replace` underneath); it is hand-rolled here only so that download
//! progress can be shown in the app window instead of a console.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// GitHub `owner/name` that publishes releases.
pub const REPO: &str = "meowyourhead/inventory-tracker";
/// Version of this build (from Cargo.toml).
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Build date, set by build.rs.
pub const BUILD_DATE: &str = env!("INVENTORY_BUILD_DATE");

const ASSET_NAME: &str = "Inventory.exe";
const CHECKSUM_NAME: &str = "Inventory.exe.sha256";
/// Refuse absurd downloads (the exe is ~10 MB).
const MAX_EXE_BYTES: u64 = 200 * 1024 * 1024;
const USER_AGENT: &str = "Inventory-Updater";

#[derive(Debug, Clone)]
pub struct Release {
    pub version: Version,
    pub name: String,
    pub notes: String,
    pub published_at: String,
    pub page_url: String,
    pub asset_url: String,
    pub asset_size: u64,
    /// Lower-case hex SHA-256 of the asset, if GitHub reported one.
    pub sha256: Option<String>,
    checksum_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// Parses `1`, `1.2`, `v1.2.3`, `1.2.3-beta` (pre-release suffix ignored).
    pub fn parse(text: &str) -> Option<Self> {
        let core = text
            .trim()
            .trim_start_matches(['v', 'V'])
            .split(['-', '+'])
            .next()?;
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self(major, minor, patch))
    }

    pub fn current() -> Self {
        Self::parse(CURRENT).unwrap_or(Self(0, 0, 0))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    published_at: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
    digest: Option<String>,
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .user_agent(USER_AGENT)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(timeout))
        .build()
        .into()
}

fn http_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(404) => "No release has been published yet".into(),
        ureq::Error::StatusCode(403 | 429) => {
            "GitHub is rate-limiting update checks; try again later".into()
        }
        ureq::Error::StatusCode(code) => format!("GitHub returned HTTP {code}"),
        other => format!("Cannot reach GitHub: {other}"),
    }
}

/// Download links must point at this repository's release files.
fn is_release_url(url: &str) -> bool {
    url.starts_with(&format!("https://github.com/{REPO}/releases/download/"))
}

fn normalize_sha256(text: &str) -> Option<String> {
    let hex = text.trim().strip_prefix("sha256:").unwrap_or(text.trim());
    let hex: String = hex.chars().take(64).collect();
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_lowercase())
}

/// Fetches the latest published (non-draft, non-prerelease) release.
pub fn latest_release() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut response = agent(Duration::from_secs(20))
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(http_error)?;
    let raw = response
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| format!("Cannot read the GitHub response: {e}"))?;
    let release: GhRelease =
        serde_json::from_str(&raw).map_err(|e| format!("Unexpected GitHub response: {e}"))?;

    let version = Version::parse(&release.tag_name)
        .ok_or_else(|| format!("Release tag {:?} is not a version number", release.tag_name))?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(ASSET_NAME))
        .ok_or_else(|| format!("Release {} has no {ASSET_NAME} file", release.tag_name))?;
    if !is_release_url(&asset.browser_download_url) {
        return Err("Release download link is not on GitHub".into());
    }
    let checksum_url = release
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(CHECKSUM_NAME))
        .map(|a| a.browser_download_url.clone())
        .filter(|u| is_release_url(u));

    Ok(Release {
        name: release
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(release.tag_name),
        version,
        notes: release.body.unwrap_or_default(),
        published_at: release.published_at.unwrap_or_default(),
        page_url: release.html_url,
        asset_url: asset.browser_download_url.clone(),
        asset_size: asset.size,
        sha256: asset.digest.as_deref().and_then(normalize_sha256),
        checksum_url,
    })
}

fn expected_sha256(release: &Release) -> Result<String, String> {
    if let Some(hash) = &release.sha256 {
        return Ok(hash.clone());
    }
    let url = release
        .checksum_url
        .as_deref()
        .ok_or("The release has no checksum, so it cannot be verified")?;
    let text = agent(Duration::from_secs(30))
        .get(url)
        .call()
        .map_err(http_error)?
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .map_err(|e| format!("Cannot read the checksum: {e}"))?;
    normalize_sha256(&text).ok_or_else(|| "The release checksum file is malformed".into())
}

/// Downloads `release` into `dest`, verifying size and SHA-256.
/// `progress(done, total)` is called as bytes arrive.
fn download(release: &Release, dest: &Path, progress: &dyn Fn(u64, u64)) -> Result<(), String> {
    if release.asset_size == 0 || release.asset_size > MAX_EXE_BYTES {
        return Err("The release file has an unexpected size".into());
    }
    let expected = expected_sha256(release)?;
    let total = release.asset_size;

    let mut response = agent(Duration::from_secs(30 * 60))
        .get(&release.asset_url)
        .call()
        .map_err(http_error)?;
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_EXE_BYTES)
        .reader();
    let mut file = fs::File::create(dest)
        .map_err(|e| format!("Cannot write the update next to the app ({}): {e}", dest.display()))?;

    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let mut reported = 0u64;
    let mut header = [0u8; 2];
    progress(0, total);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        if done < 2 {
            for (i, b) in chunk.iter().take(2 - done as usize).enumerate() {
                header[done as usize + i] = *b;
            }
        }
        hasher.update(chunk);
        file.write_all(chunk)
            .map_err(|e| format!("Cannot write the update: {e}"))?;
        done += n as u64;
        if done > total {
            return Err("The download is larger than announced".into());
        }
        // Throttle UI updates to roughly every 1% (and always the last one).
        if done == total || done - reported > total / 100 {
            reported = done;
            progress(done, total);
        }
    }
    file.sync_all()
        .map_err(|e| format!("Cannot write the update: {e}"))?;
    drop(file);

    if done != total {
        return Err(format!("Download incomplete ({done} of {total} bytes)"));
    }
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        return Err("The downloaded file failed its checksum; it was not installed".into());
    }
    if &header != b"MZ" {
        return Err("The downloaded file is not a Windows program".into());
    }
    Ok(())
}

/// Downloads and verifies `release`, then replaces the running executable.
/// The caller restarts the app afterwards.
pub fn install(release: &Release, progress: &dyn Fn(u64, u64)) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        return Err("Updates are disabled in development builds".into());
    }
    let exe = std::env::current_exe().map_err(|e| format!("Cannot locate the app: {e}"))?;
    let dir = exe
        .parent()
        .ok_or("Cannot locate the app folder")?
        .to_path_buf();
    let staged = dir.join("Inventory.exe.update");

    let result = download(release, &staged, progress).and_then(|()| {
        self_replace::self_replace(&staged)
            .map_err(|e| format!("Cannot replace the app executable: {e}"))
    });
    let _ = fs::remove_file(&staged);
    result.map(|()| exe)
}

/// Waits (up to `timeout`) for the previous app process to exit so that the
/// restarted app does not run alongside it.
#[cfg(windows)]
pub fn wait_for_process_exit(pid: u32, timeout: Duration) {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    // SAFETY: OpenProcess has no memory preconditions; a null handle means
    // the process is already gone (or not accessible) and is handled below.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return;
    }
    let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: `handle` is a valid process handle owned here and closed once.
    unsafe {
        WaitForSingleObject(handle, millis);
        CloseHandle(handle);
    }
}

#[cfg(not(windows))]
pub fn wait_for_process_exit(_pid: u32, timeout: Duration) {
    std::thread::sleep(timeout.min(Duration::from_secs(2)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(Version::parse("v1.0.0"), Some(Version(1, 0, 0)));
        assert_eq!(Version::parse("1.2"), Some(Version(1, 2, 0)));
        assert_eq!(Version::parse("V2"), Some(Version(2, 0, 0)));
        assert_eq!(Version::parse("1.4.2-beta.1"), Some(Version(1, 4, 2)));
        assert_eq!(Version::parse("release"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert!(Version(1, 10, 0) > Version(1, 9, 9));
    }

    #[test]
    fn normalizes_checksums() {
        let hex = "AB".repeat(32);
        assert_eq!(normalize_sha256(&format!("sha256:{hex}")), Some("ab".repeat(32)));
        assert_eq!(
            normalize_sha256(&format!("{hex}  Inventory.exe\n")),
            Some("ab".repeat(32))
        );
        assert_eq!(normalize_sha256("sha256:1234"), None);
    }

    #[test]
    fn only_this_repo_release_urls_are_trusted() {
        assert!(is_release_url(&format!(
            "https://github.com/{REPO}/releases/download/v1.0.0/Inventory.exe"
        )));
        assert!(!is_release_url("https://example.com/Inventory.exe"));
        assert!(!is_release_url(
            "https://github.com/someone/else/releases/download/v1/Inventory.exe"
        ));
    }
}
