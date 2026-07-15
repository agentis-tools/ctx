//! End-to-end CLI tests for `ctx self-update`, the passive update notice,
//! and `ctx --version [--check]`.
//!
//! All network traffic goes to a local [`MockServer`] via the
//! `CTX_UPDATE_BASE_URL` override; the timestamp cache is isolated per test
//! via `CTX_CACHE_DIR`. Tests that replace a binary never touch the real
//! test binary: they copy it to a temp dir and run the copy.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ctx::testutil::MockServer;
use ctx::update::{artifact_name, release_target};
use sha2::{Digest, Sha256};

/// Version the test binary was built as.
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Env vars that influence the update code paths; every test starts from a
/// clean slate (the test process itself may run inside Claude Code or CI).
const UPDATE_ENV_VARS: [&str; 7] = [
    "CLAUDECODE",
    "CLAUDE_PROJECT_DIR",
    "CLAUDE_PLUGIN_ROOT",
    "CTX_NO_UPDATE_CHECK",
    "CTX_UPDATE_FORCE_TTY",
    "CTX_UPDATE_BASE_URL",
    "CTX_CACHE_DIR",
];

fn ctx_command(bin: &Path, cwd: &Path) -> Command {
    let mut cmd = Command::new(bin);
    cmd.current_dir(cwd);
    for var in UPDATE_ENV_VARS {
        cmd.env_remove(var);
    }
    cmd
}

fn run(cmd: &mut Command) -> Output {
    cmd.output().expect("failed to run ctx binary")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// ============================================================================
// Release fixture
// ============================================================================

/// Serve a complete fake release on `server`: release JSON (latest + by-tag),
/// SHA256SUMS, and a tar.gz artifact whose `ctx` member is `payload`.
/// `corrupt_checksum` publishes a wrong digest for the artifact.
fn serve_release(server: &MockServer, tag: &str, payload: &[u8], corrupt_checksum: bool) {
    let target = release_target().expect("tests require a supported release platform");
    let artifact = artifact_name(tag, target);
    let archive = build_tar_gz(tag, target, payload);

    let mut digest = sha256(&archive);
    if corrupt_checksum {
        // Flip the first hex digit.
        let flipped = if digest.starts_with('0') { "1" } else { "0" };
        digest.replace_range(0..1, flipped);
    }
    // Multi-line SHA256SUMS, like the aggregated release file.
    let sums = format!(
        "{}  ctx-{tag}-unrelated-target.tar.gz\n{digest}  {artifact}\n",
        "0".repeat(64)
    );

    let base = server.url();
    let release_json = serde_json::json!({
        "tag_name": tag,
        "assets": [
            {
                "name": artifact,
                "browser_download_url": format!("{base}/dl/{artifact}"),
            },
            {
                "name": "SHA256SUMS",
                "browser_download_url": format!("{base}/dl/SHA256SUMS"),
            },
        ],
    })
    .to_string();

    server.add_route("/releases/latest", "application/json", release_json.clone());
    server.add_route(
        &format!("/releases/tags/{tag}"),
        "application/json",
        release_json,
    );
    server.add_route("/dl/SHA256SUMS", "text/plain", sums);
    server.add_route(&format!("/dl/{artifact}"), "application/gzip", archive);
}

/// Build a release-shaped tar.gz: `ctx-<tag>-<target>/ctx` with `payload`.
fn build_tar_gz(tag: &str, target: &str, payload: &[u8]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(gz);
    let mut header = tar::Header::new_gnu();
    header.set_size(payload.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder
        .append_data(&mut header, format!("ctx-{tag}-{target}/ctx"), payload)
        .expect("failed to append tar member");
    builder
        .into_inner()
        .expect("failed to finish tar")
        .finish()
        .expect("failed to finish gzip")
}

/// Copy the real test binary to `dir` so replace tests never touch it.
fn copy_binary(dir: &Path) -> PathBuf {
    let copy = dir.join("ctx");
    fs::copy(env!("CARGO_BIN_EXE_ctx"), &copy).expect("failed to copy ctx binary");
    copy
}

// ============================================================================
// (1) self-update replaces the binary
// ============================================================================

#[cfg(unix)]
#[test]
fn test_self_update_replaces_binary_and_prints_versions() {
    use std::os::unix::fs::PermissionsExt;

    let server = MockServer::start();
    let payload = b"FAKE-CTX-BINARY v9.9.9\n";
    serve_release(&server, "v9.9.9", payload, false);

    let temp = tempfile::tempdir().unwrap();
    let bin = copy_binary(temp.path());

    let out = run(ctx_command(&bin, temp.path())
        .arg("self-update")
        .env("CTX_UPDATE_BASE_URL", server.url()));
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
    assert!(
        stdout_of(&out).contains(&format!("ctx {CURRENT} → 9.9.9")),
        "stdout: {}",
        stdout_of(&out)
    );

    // The temp binary now IS the served payload, and stayed executable.
    assert_eq!(fs::read(&bin).unwrap(), payload);
    let mode = fs::metadata(&bin).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "replacement must be executable");
}

#[cfg(unix)]
#[test]
fn test_self_update_pinned_version_uses_tag_route() {
    let server = MockServer::start();
    let payload = b"FAKE-CTX-BINARY pinned\n";
    serve_release(&server, "v9.9.9", payload, false);

    let temp = tempfile::tempdir().unwrap();
    let bin = copy_binary(temp.path());

    let out = run(ctx_command(&bin, temp.path())
        .args(["self-update", "--version", "9.9.9"])
        .env("CTX_UPDATE_BASE_URL", server.url()));
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
    assert_eq!(fs::read(&bin).unwrap(), payload);
}

#[test]
fn test_self_update_pinned_current_version_is_up_to_date_json() {
    let server = MockServer::start();
    let tag = format!("v{CURRENT}");
    serve_release(&server, &tag, b"unused\n", false);

    // Nothing gets replaced, so the real test binary is safe to use.
    let temp = tempfile::tempdir().unwrap();
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_ctx"));
    let out = run(ctx_command(&bin, temp.path())
        .args(["self-update", "--version", CURRENT, "--json"])
        .env("CTX_UPDATE_BASE_URL", server.url()));
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));

    let doc: serde_json::Value = serde_json::from_str(&stdout_of(&out)).unwrap();
    assert_eq!(doc["command"], "self_update");
    assert_eq!(doc["data"]["old_version"], CURRENT);
    assert_eq!(doc["data"]["new_version"], CURRENT);
    assert_eq!(doc["data"]["outcome"], "up_to_date");
}

// ============================================================================
// (2) corrupted checksum aborts, binary untouched
// ============================================================================

#[cfg(unix)]
#[test]
fn test_self_update_checksum_mismatch_aborts_with_exit_2() {
    let server = MockServer::start();
    serve_release(&server, "v9.9.9", b"FAKE-CTX-BINARY evil\n", true);

    let temp = tempfile::tempdir().unwrap();
    let bin = copy_binary(temp.path());
    let before = sha256(&fs::read(&bin).unwrap());

    let out = run(ctx_command(&bin, temp.path())
        .arg("self-update")
        .env("CTX_UPDATE_BASE_URL", server.url()));
    assert_eq!(out.status.code(), Some(2), "stdout: {}", stdout_of(&out));
    assert!(
        stderr_of(&out).contains("checksum"),
        "stderr: {}",
        stderr_of(&out)
    );

    // Byte-identical to before the attempt.
    assert_eq!(sha256(&fs::read(&bin).unwrap()), before);
}

// ============================================================================
// (3) passive notice: once, cached for 24h
// ============================================================================

#[test]
fn test_passive_notice_prints_once_and_caches_for_24h() {
    let server = MockServer::start();
    serve_release(&server, "v9.9.9", b"unused\n", false);

    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_ctx"));
    let notice = format!("ctx 9.9.9 available (you have {CURRENT}) — run 'ctx self-update'");

    // A cheap command that flows through run() and exits 0.
    let mut first = ctx_command(&bin, temp.path());
    first
        .args(["harness", "compat", "--require", "0.1"])
        .env("CTX_UPDATE_BASE_URL", server.url())
        .env("CTX_CACHE_DIR", &cache)
        .env("CTX_UPDATE_FORCE_TTY", "1");
    let out = run(&mut first);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));

    let stderr = stderr_of(&out);
    assert_eq!(
        stderr.matches(&notice).count(),
        1,
        "expected exactly one notice, stderr: {stderr}"
    );
    assert!(
        !stdout_of(&out).contains("available"),
        "notice must never reach stdout"
    );
    assert!(
        cache.join("last-update-check").exists(),
        "timestamp cache file written"
    );
    let hits_after_first = server.hits();
    assert!(hits_after_first >= 1, "first run must query the mock");

    // Second invocation within 24h: no notice, and NO network call at all.
    let mut second = ctx_command(&bin, temp.path());
    second
        .args(["harness", "compat", "--require", "0.1"])
        .env("CTX_UPDATE_BASE_URL", server.url())
        .env("CTX_CACHE_DIR", &cache)
        .env("CTX_UPDATE_FORCE_TTY", "1");
    let out = run(&mut second);
    assert_eq!(out.status.code(), Some(0));
    assert!(!stderr_of(&out).contains("available"), "no repeat notice");
    assert_eq!(
        server.hits(),
        hits_after_first,
        "cached check must not hit the network"
    );
}

// ============================================================================
// (4) suppression: zero network calls
// ============================================================================

#[test]
fn test_suppression_means_zero_network_calls() {
    let server = MockServer::start();
    serve_release(&server, "v9.9.9", b"unused\n", false);

    let temp = tempfile::tempdir().unwrap();
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_ctx"));
    let compat = ["harness", "compat", "--require", "0.1"];

    // (a) CTX_NO_UPDATE_CHECK=1
    let out = run(ctx_command(&bin, temp.path())
        .args(compat)
        .env("CTX_UPDATE_BASE_URL", server.url())
        .env("CTX_CACHE_DIR", temp.path().join("cache-a"))
        .env("CTX_UPDATE_FORCE_TTY", "1")
        .env("CTX_NO_UPDATE_CHECK", "1"));
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        server.hits(),
        0,
        "CTX_NO_UPDATE_CHECK must prevent the call"
    );

    // (b) --json active
    let out = run(ctx_command(&bin, temp.path())
        .args(compat)
        .arg("--json")
        .env("CTX_UPDATE_BASE_URL", server.url())
        .env("CTX_CACHE_DIR", temp.path().join("cache-b"))
        .env("CTX_UPDATE_FORCE_TTY", "1"));
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(server.hits(), 0, "--json must prevent the call");

    // (c) Claude Code hook environment
    let out = run(ctx_command(&bin, temp.path())
        .args(compat)
        .env("CTX_UPDATE_BASE_URL", server.url())
        .env("CTX_CACHE_DIR", temp.path().join("cache-c"))
        .env("CTX_UPDATE_FORCE_TTY", "1")
        .env("CLAUDE_PROJECT_DIR", "x"));
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(server.hits(), 0, "hook env vars must prevent the call");

    // (d) stderr not a TTY (no CTX_UPDATE_FORCE_TTY): also suppressed.
    let out = run(ctx_command(&bin, temp.path())
        .args(compat)
        .env("CTX_UPDATE_BASE_URL", server.url())
        .env("CTX_CACHE_DIR", temp.path().join("cache-d")));
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(server.hits(), 0, "non-TTY stderr must prevent the call");
}

// ============================================================================
// (5) --version and --version --check
// ============================================================================

#[test]
fn test_version_flag_output_is_unchanged() {
    // Snapshot of what clap's auto --version printed before it was replaced
    // by our custom flag: exactly "ctx <version>\n".
    let temp = tempfile::tempdir().unwrap();
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_ctx"));

    for flag in ["--version", "-V"] {
        let out = run(ctx_command(&bin, temp.path()).arg(flag));
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(stdout_of(&out), format!("ctx {CURRENT}\n"), "flag: {flag}");
    }

    // --check requires --version.
    let out = run(ctx_command(&bin, temp.path()).arg("--check"));
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn test_version_check_reports_update_and_bypasses_cache() {
    let server = MockServer::start();
    serve_release(&server, "v9.9.9", b"unused\n", false);

    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_ctx"));

    // Pre-populate a fresh passive-check stamp: --version --check must
    // ignore it (exempt from the 24h cache).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("last-update-check"), format!("{now}\n")).unwrap();

    let mut hits_before = 0;
    for _ in 0..2 {
        let out = run(ctx_command(&bin, temp.path())
            .args(["--version", "--check"])
            .env("CTX_UPDATE_BASE_URL", server.url())
            .env("CTX_CACHE_DIR", &cache));
        assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
        assert_eq!(
            stdout_of(&out),
            format!("ctx 9.9.9 available (you have {CURRENT}) — run 'ctx self-update'\n")
        );
        let hits = server.hits();
        assert!(
            hits > hits_before,
            "--version --check must hit the network every time"
        );
        hits_before = hits;
    }
}

#[test]
fn test_version_check_up_to_date_and_json() {
    let server = MockServer::start();
    serve_release(&server, &format!("v{CURRENT}"), b"unused\n", false);

    let temp = tempfile::tempdir().unwrap();
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_ctx"));

    let out = run(ctx_command(&bin, temp.path())
        .args(["--version", "--check"])
        .env("CTX_UPDATE_BASE_URL", server.url()));
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
    assert_eq!(stdout_of(&out), format!("ctx {CURRENT} is up to date\n"));

    let out = run(ctx_command(&bin, temp.path())
        .args(["--version", "--check", "--json"])
        .env("CTX_UPDATE_BASE_URL", server.url()));
    assert_eq!(out.status.code(), Some(0));
    let doc: serde_json::Value = serde_json::from_str(&stdout_of(&out)).unwrap();
    assert_eq!(doc["command"], "version.check");
    assert_eq!(doc["data"]["current_version"], CURRENT);
    assert_eq!(doc["data"]["latest_version"], CURRENT);
    assert_eq!(doc["data"]["update_available"], false);
}
