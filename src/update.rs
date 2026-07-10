//! Release and update mechanism: `ctx self-update`, `ctx --version --check`,
//! and the passive update notice.
//!
//! # Determinism rule
//!
//! ctx **never updates itself automatically**. The passive check only prints
//! a one-line notice to stderr; replacing the binary always requires an
//! explicit `ctx self-update` invocation.
//!
//! # How updates work
//!
//! Releases are published by `.github/workflows/release.yml`: one archive per
//! platform named `ctx-<tag>-<target>.tar.gz` (`.zip` on Windows), plus an
//! aggregated `SHA256SUMS` file. `self_update` queries the GitHub Releases
//! API, downloads the artifact for the compile-time target, verifies its
//! sha256 against `SHA256SUMS`, extracts the `ctx` binary member, and
//! atomically renames it over the running executable. On checksum mismatch
//! the update aborts with an error (exit code 2) and the installed binary is
//! left untouched.
//!
//! # Test hooks (undocumented environment variables)
//!
//! - `CTX_UPDATE_BASE_URL`: overrides the GitHub API base URL so integration
//!   tests can point at a local mock server (asset download URLs come from
//!   the release JSON itself, so the mock controls those too).
//! - `CTX_CACHE_DIR`: overrides the passive-check cache directory
//!   (default: `dirs::cache_dir()/ctx`).
//! - `CTX_UPDATE_FORCE_TTY=1`: makes the passive check treat stderr as a
//!   terminal, so the end-to-end notice path can be exercised from a test
//!   where stderr is a pipe.

use std::collections::BTreeMap;
use std::fs;
use std::io::IsTerminal;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use semver::Version;
use sha2::{Digest, Sha256};

use crate::error::{CtxError, Result};

/// GitHub API base for this repository's releases.
const DEFAULT_BASE_URL: &str = "https://api.github.com/repos/agentis-tools/ctx";

/// How often the passive check may touch the network (24 hours).
pub const PASSIVE_CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Cache file (under the cache dir) storing the unix timestamp of the last
/// passive check attempt.
const STAMP_FILE: &str = "last-update-check";

/// Network budget for the passive (notice-only) check.
const PASSIVE_TIMEOUT: Duration = Duration::from_secs(1);

/// Network budget for explicit operations (`self-update`, `--version --check`).
const EXPLICIT_TIMEOUT: Duration = Duration::from_secs(30);

// ============================================================================
// Platform / artifact mapping
// ============================================================================

/// The release target triple this binary was compiled for, or `None` when
/// the release workflow publishes no artifact for this platform.
///
/// This table mirrors the build matrix in `.github/workflows/release.yml`
/// exactly; keep the two in sync.
pub fn release_target() -> Option<&'static str> {
    if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )) {
        Some("x86_64-unknown-linux-gnu")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("x86_64-apple-darwin")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc"
    )) {
        Some("x86_64-pc-windows-msvc")
    } else {
        None
    }
}

/// Release artifact file name for a tag and target, mirroring the packaging
/// steps in `.github/workflows/release.yml` (`ctx-<tag>-<target>.tar.gz`,
/// `.zip` for Windows). `tag` includes the leading `v` (it is
/// `$GITHUB_REF_NAME` in the workflow).
pub fn artifact_name(tag: &str, target: &str) -> String {
    let ext = if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("ctx-{tag}-{target}.{ext}")
}

/// The version this binary was built as.
pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is valid semver")
}

// ============================================================================
// GitHub Releases API client
// ============================================================================

/// One downloadable asset attached to a release.
#[derive(Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub download_url: String,
}

/// A GitHub release: tag (e.g. `v0.3.0`) plus its assets.
#[derive(Debug, Clone)]
pub struct Release {
    pub tag: String,
    pub assets: Vec<Asset>,
}

impl Release {
    /// The semver version encoded in the tag (`v0.3.0` -> `0.3.0`).
    pub fn version(&self) -> Result<Version> {
        Version::parse(self.tag.trim_start_matches('v'))
            .map_err(|e| CtxError::Other(format!("release tag '{}' is not semver: {e}", self.tag)))
    }

    /// Find an asset by exact file name.
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// API base URL, overridable via `CTX_UPDATE_BASE_URL` (test hook).
fn base_url() -> String {
    std::env::var("CTX_UPDATE_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}

fn http_client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("ctx/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(CtxError::Network)
}

/// Fetch the latest (non-prerelease, non-draft) release.
pub fn latest_release(client: &reqwest::blocking::Client) -> Result<Release> {
    fetch_release(client, &format!("{}/releases/latest", base_url()))
}

/// Fetch a specific release by tag (e.g. `v0.3.0`).
pub fn release_by_tag(client: &reqwest::blocking::Client, tag: &str) -> Result<Release> {
    fetch_release(client, &format!("{}/releases/tags/{tag}", base_url()))
}

fn fetch_release(client: &reqwest::blocking::Client, url: &str) -> Result<Release> {
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()?;
    let status = response.status();
    if !status.is_success() {
        return Err(CtxError::Other(format!(
            "release query failed: {url} returned HTTP {status}"
        )));
    }
    let value: serde_json::Value = response.json()?;
    parse_release(&value)
}

/// Parse the GitHub Releases API JSON for one release.
pub fn parse_release(value: &serde_json::Value) -> Result<Release> {
    let tag = value["tag_name"]
        .as_str()
        .ok_or_else(|| CtxError::Other("release JSON has no tag_name".to_string()))?
        .to_string();
    let assets = value["assets"]
        .as_array()
        .map(|assets| {
            assets
                .iter()
                .filter_map(|a| {
                    Some(Asset {
                        name: a["name"].as_str()?.to_string(),
                        download_url: a["browser_download_url"].as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release { tag, assets })
}

// ============================================================================
// SHA256SUMS
// ============================================================================

/// Parse a `SHA256SUMS` file: one `<hex>  <name>` entry per line (the
/// `sha256sum` / `shasum -a 256` format; a leading `*` on the name marks
/// binary mode and is stripped). Returns file name -> lowercase hex digest.
pub fn parse_sha256sums(text: &str) -> BTreeMap<String, String> {
    let mut sums = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((hex, name)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let name = name.trim_start().trim_start_matches('*');
        if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) && !name.is_empty() {
            sums.insert(name.to_string(), hex.to_ascii_lowercase());
        }
    }
    sums
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

// ============================================================================
// Archive extraction
// ============================================================================

/// Extract the ctx binary member from a downloaded release archive.
///
/// Unix artifacts are `.tar.gz` containing `ctx-<tag>-<target>/ctx`; Windows
/// artifacts are `.zip` containing `ctx-<tag>-<target>/ctx.exe`.
fn extract_binary(archive: &[u8], artifact: &str) -> Result<Vec<u8>> {
    if artifact.ends_with(".tar.gz") {
        return extract_from_tar_gz(archive);
    }
    #[cfg(windows)]
    if artifact.ends_with(".zip") {
        return extract_from_zip(archive);
    }
    Err(CtxError::Other(format!(
        "cannot extract '{artifact}': unsupported archive format for this platform"
    )))
}

fn extract_from_tar_gz(archive: &[u8]) -> Result<Vec<u8>> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let is_ctx = entry.path()?.file_name().is_some_and(|name| name == "ctx");
        if is_ctx {
            let mut binary = Vec::new();
            entry.read_to_end(&mut binary)?;
            return Ok(binary);
        }
    }
    Err(CtxError::Other(
        "release archive contains no 'ctx' binary member".to_string(),
    ))
}

#[cfg(windows)]
fn extract_from_zip(archive: &[u8]) -> Result<Vec<u8>> {
    let cursor = std::io::Cursor::new(archive);
    let mut zip = zip::ZipArchive::new(cursor)
        .map_err(|e| CtxError::Other(format!("invalid release zip: {e}")))?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| CtxError::Other(format!("invalid release zip entry: {e}")))?;
        if entry.is_dir() {
            continue;
        }
        let is_ctx = entry
            .enclosed_name()
            .and_then(|p| p.file_name().map(|n| n.to_os_string()))
            .is_some_and(|name| name == "ctx.exe");
        if is_ctx {
            let mut binary = Vec::new();
            entry.read_to_end(&mut binary)?;
            return Ok(binary);
        }
    }
    Err(CtxError::Other(
        "release archive contains no 'ctx.exe' binary member".to_string(),
    ))
}

// ============================================================================
// Atomic executable replacement
// ============================================================================

/// Check that we can create files in the executable's directory before doing
/// any network work.
fn ensure_writable(dir: &Path) -> Result<()> {
    let probe = dir.join(format!(".ctx-write-probe-{}", std::process::id()));
    let outcome = fs::write(&probe, b"").and_then(|()| fs::remove_file(&probe));
    outcome.map_err(|e| {
        CtxError::Other(format!(
            "cannot update: install location '{}' is not writable ({e}); \
             re-run with elevated permissions or update through the package \
             manager that installed ctx (e.g. 'cargo install agentis-ctx')",
            dir.display()
        ))
    })
}

/// Atomically replace `exe` with `data`: write to a temp file in the same
/// directory, set executable permissions, then rename over the target.
///
/// On Windows the running executable cannot be overwritten, but it can be
/// renamed: the current `ctx.exe` is moved aside to `ctx.exe.old` first, and
/// the new binary is renamed into place. The `.old` file is removed on the
/// next `ctx self-update` run (or can be deleted manually).
fn replace_executable(exe: &Path, data: &[u8]) -> Result<()> {
    let dir = exe
        .parent()
        .ok_or_else(|| CtxError::Other("executable has no parent directory".to_string()))?;
    let staged = dir.join(format!(".ctx-update-{}", std::process::id()));
    if let Err(e) = fs::write(&staged, data) {
        let _ = fs::remove_file(&staged);
        return Err(e.into());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)) {
            let _ = fs::remove_file(&staged);
            return Err(e.into());
        }
        if let Err(e) = fs::rename(&staged, exe) {
            let _ = fs::remove_file(&staged);
            return Err(e.into());
        }
    }

    #[cfg(windows)]
    {
        let old = old_binary_path(exe);
        let _ = fs::remove_file(&old);
        if let Err(e) = fs::rename(exe, &old) {
            let _ = fs::remove_file(&staged);
            return Err(e.into());
        }
        if let Err(e) = fs::rename(&staged, exe) {
            // Roll the original back into place before failing.
            let _ = fs::rename(&old, exe);
            let _ = fs::remove_file(&staged);
            return Err(e.into());
        }
    }

    Ok(())
}

#[cfg(windows)]
fn old_binary_path(exe: &Path) -> PathBuf {
    let mut name = exe
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".old");
    exe.with_file_name(name)
}

// ============================================================================
// ctx self-update
// ============================================================================

/// Result of a [`self_update`] run.
#[derive(Debug, Clone)]
pub struct SelfUpdateReport {
    pub old_version: Version,
    pub new_version: Version,
    /// `false` when there was nothing to do (already up to date).
    pub updated: bool,
}

/// Update the running executable from the GitHub release feed.
///
/// `pin` installs an exact version (`X.Y.Z`, with or without a leading `v`)
/// instead of the latest release; pinning also allows downgrades. Returns
/// with `updated: false` when there is nothing to do. All failure modes
/// (network, missing artifact, checksum mismatch, unwritable install
/// location) return `Err`, which the CLI maps to exit code 2, and always
/// leave the installed binary untouched.
pub fn self_update(pin: Option<&str>) -> Result<SelfUpdateReport> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| CtxError::Other("cannot locate the running executable".to_string()))?;

    // Clean up the renamed-aside binary a previous Windows update left behind.
    #[cfg(windows)]
    {
        let _ = fs::remove_file(old_binary_path(&exe));
    }

    // Refuse before any network work when we could not replace the binary.
    ensure_writable(dir)?;

    let client = http_client(EXPLICIT_TIMEOUT)?;
    let release = match pin {
        Some(version) => {
            let tag = format!("v{}", version.trim_start_matches('v'));
            release_by_tag(&client, &tag)?
        }
        None => latest_release(&client)?,
    };
    let new_version = release.version()?;
    let old_version = current_version();

    // Latest: only move forward. Pinned: install anything that differs
    // (explicit downgrades are allowed).
    let update_needed = match pin {
        Some(_) => new_version != old_version,
        None => new_version > old_version,
    };
    if !update_needed {
        return Ok(SelfUpdateReport {
            old_version,
            new_version,
            updated: false,
        });
    }

    let target = release_target().ok_or_else(|| {
        CtxError::Other(format!(
            "no prebuilt release artifact for this platform ({}-{}); \
             update with 'cargo install agentis-ctx' instead",
            std::env::consts::ARCH,
            std::env::consts::OS
        ))
    })?;
    let artifact = artifact_name(&release.tag, target);
    let asset = release.asset(&artifact).ok_or_else(|| {
        CtxError::Other(format!(
            "release {} has no artifact named '{artifact}'",
            release.tag
        ))
    })?;
    let sums_asset = release.asset("SHA256SUMS").ok_or_else(|| {
        CtxError::Other(format!(
            "release {} publishes no SHA256SUMS file; refusing to install an unverifiable binary",
            release.tag
        ))
    })?;

    let sums_text = download_text(&client, &sums_asset.download_url)?;
    let sums = parse_sha256sums(&sums_text);
    let expected = sums.get(&artifact).ok_or_else(|| {
        CtxError::Other(format!(
            "SHA256SUMS for release {} has no entry for '{artifact}'",
            release.tag
        ))
    })?;

    let archive = download_bytes(&client, &asset.download_url)?;
    let actual = sha256_hex(&archive);
    if actual != *expected {
        return Err(CtxError::Other(format!(
            "checksum mismatch for '{artifact}': expected {expected}, got {actual}; \
             aborting update (the installed binary is unchanged)"
        )));
    }

    let binary = extract_binary(&archive, &artifact)?;
    replace_executable(&exe, &binary)?;

    Ok(SelfUpdateReport {
        old_version,
        new_version,
        updated: true,
    })
}

fn download_text(client: &reqwest::blocking::Client, url: &str) -> Result<String> {
    let response = client.get(url).send()?;
    let status = response.status();
    if !status.is_success() {
        return Err(CtxError::Other(format!(
            "download failed: {url} returned HTTP {status}"
        )));
    }
    Ok(response.text()?)
}

fn download_bytes(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>> {
    let response = client.get(url).send()?;
    let status = response.status();
    if !status.is_success() {
        return Err(CtxError::Other(format!(
            "download failed: {url} returned HTTP {status}"
        )));
    }
    Ok(response.bytes()?.to_vec())
}

// ============================================================================
// Explicit version check (ctx --version --check)
// ============================================================================

/// The stderr notice printed by the passive check, also reused by
/// `ctx --version --check`.
pub fn update_notice(latest: &Version, current: &Version) -> String {
    format!("ctx {latest} available (you have {current}) — run 'ctx self-update'")
}

/// Query the latest release and compare against the running binary.
///
/// Used by `ctx --version --check`; always allowed (no suppression, no 24h
/// cache). Returns `(current, latest)`.
pub fn explicit_check() -> Result<(Version, Version)> {
    let client = http_client(EXPLICIT_TIMEOUT)?;
    let latest = latest_release(&client)?.version()?;
    Ok((current_version(), latest))
}

// ============================================================================
// Passive update check
// ============================================================================

/// Cache directory for the passive-check timestamp:
/// `$CTX_CACHE_DIR` (test hook) or `dirs::cache_dir()/ctx`.
pub fn cache_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CTX_CACHE_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    dirs::cache_dir().map(|d| d.join("ctx"))
}

/// True when the 24h interval since the last recorded check has elapsed
/// (or no valid timestamp is recorded). `now` is unix seconds, injected so
/// tests control the clock.
pub fn check_is_due(stamp_file: &Path, now: u64) -> bool {
    let last = fs::read_to_string(stamp_file)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    match last {
        Some(last) => now >= last.saturating_add(PASSIVE_CHECK_INTERVAL_SECS),
        None => true,
    }
}

/// Record `now` (unix seconds) as the last check attempt.
pub fn record_check(stamp_file: &Path, now: u64) -> std::io::Result<()> {
    if let Some(parent) = stamp_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(stamp_file, format!("{now}\n"))
}

/// The passive-check suppression predicate (H3). Pure so the whole gate
/// matrix is unit-testable; `env` abstracts `std::env::var`.
///
/// The check is allowed only when **none** of these hold:
/// - `--json` is active (stdout is a machine-readable document),
/// - stderr is not a terminal (covers CI, pipes, and test harnesses),
/// - `CTX_NO_UPDATE_CHECK` is set (non-empty),
/// - a Claude Code hook/session environment is detected (`CLAUDECODE`,
///   `CLAUDE_PROJECT_DIR`, or `CLAUDE_PLUGIN_ROOT`).
///
/// Suppression means **no network call at all**, not just a swallowed notice.
pub fn passive_check_allowed<F>(json: bool, stderr_is_tty: bool, env: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    if json || !stderr_is_tty {
        return false;
    }
    if env("CTX_NO_UPDATE_CHECK").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    // Inside a Claude Code hook or session: never interfere.
    for var in ["CLAUDECODE", "CLAUDE_PROJECT_DIR", "CLAUDE_PLUGIN_ROOT"] {
        if env(var).is_some() {
            return false;
        }
    }
    true
}

/// Passive update check, called from `main` after the command has finished
/// (`ctx self-update` and `ctx --version` invocations skip it entirely).
///
/// At most one network request per 24h (timestamp cache), 1-second timeout,
/// silent on any failure, never panics, never writes to stdout. When a newer
/// release exists it prints exactly one line to stderr:
///
/// ```text
/// ctx <new> available (you have <current>) — run 'ctx self-update'
/// ```
pub fn passive_check(json: bool) {
    // CTX_UPDATE_FORCE_TTY=1 is a test-only hook: a test cannot give the
    // child process a real stderr TTY, so it injects the TTY gate instead.
    let stderr_is_tty = std::env::var("CTX_UPDATE_FORCE_TTY").map(|v| v == "1") == Ok(true)
        || std::io::stderr().is_terminal();
    if !passive_check_allowed(json, stderr_is_tty, |k| std::env::var(k).ok()) {
        return;
    }
    // Failures are silent by design: an update notice is never worth
    // breaking a working command for.
    let _ = passive_check_inner();
}

fn passive_check_inner() -> Result<()> {
    let dir = cache_dir().ok_or_else(|| CtxError::Other("no cache dir".to_string()))?;
    let stamp = dir.join(STAMP_FILE);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if !check_is_due(&stamp, now) {
        return Ok(());
    }
    // Record the attempt before the network call so failures do not turn
    // into a retry on every invocation.
    record_check(&stamp, now)?;

    let client = http_client(PASSIVE_TIMEOUT)?;
    let latest = latest_release(&client)?.version()?;
    let current = current_version();
    if latest > current {
        eprintln!("{}", update_notice(&latest, &current));
    }
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- platform / artifact mapping -------------------------------------

    #[test]
    fn test_artifact_names_mirror_release_workflow() {
        // One entry per release.yml build-matrix row; tag == $GITHUB_REF_NAME.
        let expected = [
            (
                "x86_64-unknown-linux-gnu",
                "ctx-v0.3.0-x86_64-unknown-linux-gnu.tar.gz",
            ),
            (
                "x86_64-apple-darwin",
                "ctx-v0.3.0-x86_64-apple-darwin.tar.gz",
            ),
            (
                "aarch64-apple-darwin",
                "ctx-v0.3.0-aarch64-apple-darwin.tar.gz",
            ),
            (
                "x86_64-pc-windows-msvc",
                "ctx-v0.3.0-x86_64-pc-windows-msvc.zip",
            ),
        ];
        for (target, name) in expected {
            assert_eq!(artifact_name("v0.3.0", target), name);
        }
    }

    #[test]
    fn test_release_target_matches_compile_target() {
        let expected = if cfg!(all(
            target_os = "linux",
            target_arch = "x86_64",
            target_env = "gnu"
        )) {
            Some("x86_64-unknown-linux-gnu")
        } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            Some("x86_64-apple-darwin")
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Some("aarch64-apple-darwin")
        } else if cfg!(all(
            target_os = "windows",
            target_arch = "x86_64",
            target_env = "msvc"
        )) {
            Some("x86_64-pc-windows-msvc")
        } else {
            None
        };
        assert_eq!(release_target(), expected);
        // Every supported target maps to a well-formed artifact name.
        if let Some(target) = release_target() {
            let name = artifact_name("v1.0.0", target);
            assert!(name.starts_with("ctx-v1.0.0-"));
            assert!(name.ends_with(".tar.gz") || name.ends_with(".zip"));
        }
    }

    // ---- SHA256SUMS -------------------------------------------------------

    #[test]
    fn test_parse_sha256sums_multi_line() {
        let hex_a = "a".repeat(64);
        let hex_b = "B".repeat(64);
        let text = format!(
            "{hex_a}  ctx-v0.3.0-x86_64-unknown-linux-gnu.tar.gz\n\
             \n\
             {hex_b} *ctx-v0.3.0-x86_64-pc-windows-msvc.zip\n\
             not-a-sum-line\n\
             deadbeef  too-short-digest.tar.gz\n"
        );
        let sums = parse_sha256sums(&text);
        assert_eq!(sums.len(), 2);
        assert_eq!(
            sums["ctx-v0.3.0-x86_64-unknown-linux-gnu.tar.gz"], hex_a,
            "plain entry parses"
        );
        // Binary-mode marker stripped, digest lowercased.
        assert_eq!(
            sums["ctx-v0.3.0-x86_64-pc-windows-msvc.zip"],
            hex_b.to_ascii_lowercase()
        );
    }

    #[test]
    fn test_sha256_hex_matches_known_vector() {
        // sha256("abc")
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    // ---- release JSON parsing ---------------------------------------------

    #[test]
    fn test_parse_release_and_version_comparison() {
        let value = serde_json::json!({
            "tag_name": "v0.3.0",
            "assets": [
                {"name": "SHA256SUMS", "browser_download_url": "https://x/SHA256SUMS"},
                {"name": "ctx-v0.3.0-aarch64-apple-darwin.tar.gz",
                 "browser_download_url": "https://x/ctx.tar.gz"},
                {"malformed": true}
            ]
        });
        let release = parse_release(&value).unwrap();
        assert_eq!(release.tag, "v0.3.0");
        assert_eq!(release.assets.len(), 2, "malformed asset entries skipped");
        assert_eq!(release.version().unwrap(), Version::new(0, 3, 0));
        assert!(release.asset("SHA256SUMS").is_some());
        assert!(release.asset("nope.tar.gz").is_none());

        // Version comparison semantics used by self_update / the notices.
        assert!(Version::new(1, 0, 0) > Version::new(0, 9, 9));
        assert!(Version::parse("0.3.0-rc.1").unwrap() < Version::new(0, 3, 0));
    }

    // ---- 24h cache --------------------------------------------------------

    #[test]
    fn test_check_is_due_with_injected_clock() {
        let temp = tempfile::tempdir().unwrap();
        let stamp = temp.path().join("nested").join("last-update-check");

        // No stamp file yet: due.
        assert!(check_is_due(&stamp, 1_000_000));

        record_check(&stamp, 1_000_000).unwrap();
        // Within 24h: not due (boundary excluded).
        assert!(!check_is_due(&stamp, 1_000_000));
        assert!(!check_is_due(
            &stamp,
            1_000_000 + PASSIVE_CHECK_INTERVAL_SECS - 1
        ));
        // At/after 24h: due again.
        assert!(check_is_due(
            &stamp,
            1_000_000 + PASSIVE_CHECK_INTERVAL_SECS
        ));

        // Corrupt stamp: due.
        fs::write(&stamp, "not-a-number").unwrap();
        assert!(check_is_due(&stamp, 1_000_000));
    }

    // ---- suppression matrix -------------------------------------------------

    #[test]
    fn test_passive_check_suppression_matrix() {
        let no_env = |_: &str| -> Option<String> { None };
        let env_with = |key: &'static str| move |k: &str| (k == key).then(|| "1".to_string());

        // Clean interactive invocation: allowed.
        assert!(passive_check_allowed(false, true, no_env));

        // --json active: suppressed.
        assert!(!passive_check_allowed(true, true, no_env));
        // stderr not a TTY: suppressed (covers CI and tests).
        assert!(!passive_check_allowed(false, false, no_env));
        // Opt-out env var: suppressed.
        assert!(!passive_check_allowed(
            false,
            true,
            env_with("CTX_NO_UPDATE_CHECK")
        ));
        // ...but an empty value does not suppress.
        assert!(passive_check_allowed(false, true, |k: &str| (k
            == "CTX_NO_UPDATE_CHECK")
            .then(String::new)));
        // Claude Code hook environment: suppressed, any of the three vars.
        for var in ["CLAUDECODE", "CLAUDE_PROJECT_DIR", "CLAUDE_PLUGIN_ROOT"] {
            assert!(
                !passive_check_allowed(false, true, env_with(var)),
                "{var} must suppress the passive check"
            );
        }
        // Combinations still suppress.
        assert!(!passive_check_allowed(true, false, env_with("CLAUDECODE")));
    }

    // ---- notice text --------------------------------------------------------

    #[test]
    fn test_update_notice_format() {
        let notice = update_notice(&Version::new(9, 9, 9), &Version::new(0, 2, 1));
        assert_eq!(
            notice,
            "ctx 9.9.9 available (you have 0.2.1) — run 'ctx self-update'"
        );
    }

    // ---- archive extraction --------------------------------------------------

    #[test]
    fn test_extract_binary_from_tar_gz() {
        let payload = b"#!/bin/sh\necho fake ctx\n".to_vec();
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                "ctx-v9.9.9-aarch64-apple-darwin/ctx",
                payload.as_slice(),
            )
            .unwrap();
        let archive = builder.into_inner().unwrap().finish().unwrap();

        let extracted = extract_binary(&archive, "ctx-v9.9.9-aarch64-apple-darwin.tar.gz").unwrap();
        assert_eq!(extracted, payload);
    }

    #[test]
    fn test_extract_binary_missing_member_errors() {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(5);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "dir/README.md", &b"hello"[..])
            .unwrap();
        let archive = builder.into_inner().unwrap().finish().unwrap();

        let err = extract_binary(&archive, "ctx-v9.9.9-x.tar.gz").unwrap_err();
        assert!(err.to_string().contains("no 'ctx' binary member"));
    }

    // ---- atomic replacement ---------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn test_replace_executable_atomically_swaps_content() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("ctx");
        fs::write(&exe, b"old binary").unwrap();

        replace_executable(&exe, b"new binary").unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new binary");
        let mode = fs::metadata(&exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o755, 0o755, "replacement is executable");

        // No staging leftovers.
        let leftovers: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() != "ctx")
            .collect();
        assert!(leftovers.is_empty(), "leftovers: {leftovers:?}");
    }

    #[test]
    fn test_ensure_writable_accepts_tempdir() {
        let temp = tempfile::tempdir().unwrap();
        ensure_writable(temp.path()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn test_ensure_writable_rejects_readonly_dir() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("ro");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();

        let err = ensure_writable(&dir).unwrap_err();
        assert!(err.to_string().contains("not writable"), "{err}");

        // Restore so the tempdir can be cleaned up.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
