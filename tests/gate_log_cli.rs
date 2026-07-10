//! End-to-end CLI tests for the opt-in gate log (`CTX_GATE_LOG`).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use ctx::testutil::GitRepo;

/// A small Rust source file so `ctx index` has something to chew on.
const SOURCE: &str = r#"
pub fn compute_total(items: &[i64]) -> i64 {
    let mut total = 0;
    for item in items {
        total += *item;
    }
    total
}
"#;

/// Run `ctx` with extra environment variables set on the child only (no
/// process-wide env mutation, so tests stay parallel-safe).
fn ctx_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args(args)
        .current_dir(dir)
        // Never inherit gate settings from the test runner's environment.
        .env_remove("CTX_GATE_LOG")
        .env_remove("CTX_GATE_BLOCKING")
        .env_remove("CLAUDE_SESSION_ID")
        .envs(envs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
        .output()
        .expect("failed to run ctx binary")
}

/// A repo with one committed, indexed source file.
fn seeded_repo(dir: &Path) -> GitRepo {
    let repo = GitRepo::init(dir);
    repo.commit_file("src/lib.rs", SOURCE, "initial");
    assert!(ctx_env(&repo.root, &["index"], &[]).status.success());
    repo
}

/// Parsed lines of the gate log at `path`.
fn log_lines(path: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(path)
        .expect("gate log exists")
        .lines()
        .map(|line| serde_json::from_str(line).expect("line is valid JSON"))
        .collect()
}

#[test]
fn test_gate_log_records_fail_and_pass_and_appends() {
    let temp = tempfile::tempdir().unwrap();
    let repo = seeded_repo(temp.path());
    let log = repo.root.join(".ctx/gate-log.jsonl");

    // files_changed >= 0 always holds, so this gate trips; logging must not
    // change the exit code (still 1).
    let out = ctx_env(
        &repo.root,
        &[
            "score",
            "--against",
            "HEAD",
            "--fail-on",
            "files_changed>=0",
        ],
        &[("CTX_GATE_LOG", "1"), ("CLAUDE_SESSION_ID", "sess-123")],
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let lines = log_lines(&log);
    assert_eq!(lines.len(), 1, "lines: {lines:?}");
    let record = &lines[0];
    assert_eq!(record["schema_version"], 1);
    assert_eq!(record["ctx_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(record["source"], "score");
    assert_eq!(record["against"], "HEAD");
    assert_eq!(record["fail_on"], "files_changed>=0");
    assert_eq!(record["outcome"], "fail");
    assert_eq!(record["blocking"], false);
    assert_eq!(record["session_id"], "sess-123");
    let failed = record["failed_conditions"].as_array().unwrap();
    assert_eq!(failed.len(), 1, "failed: {failed:?}");
    assert_eq!(failed[0], "files_changed >= 0");
    // The metrics object has exactly the seven documented keys.
    let metrics = record["metrics"].as_object().unwrap();
    let mut keys: Vec<&str> = metrics.keys().map(|k| k.as_str()).collect();
    keys.sort_unstable();
    let mut expected = ctx::score::Metrics::NAMES.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(metrics["files_changed"], 0);

    // A passing gate (files_changed > 0 with a clean tree) exits 0 and
    // appends a second record.
    let out = ctx_env(
        &repo.root,
        &["score", "--against", "HEAD", "--fail-on", "files_changed>0"],
        &[("CTX_GATE_LOG", "1"), ("CTX_GATE_BLOCKING", "1")],
    );
    assert_eq!(out.status.code(), Some(0));

    let lines = log_lines(&log);
    assert_eq!(lines.len(), 2, "lines: {lines:?}");
    let record = &lines[1];
    assert_eq!(record["outcome"], "pass");
    assert_eq!(record["blocking"], true);
    assert!(record["session_id"].is_null());
    assert!(record["failed_conditions"].as_array().unwrap().is_empty());
}

#[test]
fn test_gate_log_absent_without_env() {
    let temp = tempfile::tempdir().unwrap();
    let repo = seeded_repo(temp.path());

    let out = ctx_env(
        &repo.root,
        &[
            "score",
            "--against",
            "HEAD",
            "--fail-on",
            "files_changed>=0",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(!repo.root.join(".ctx/gate-log.jsonl").exists());

    // "0" is an explicit off switch, not a path.
    let out = ctx_env(
        &repo.root,
        &["score", "--against", "HEAD"],
        &[("CTX_GATE_LOG", "0")],
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(!repo.root.join(".ctx/gate-log.jsonl").exists());
    assert!(!repo.root.join("0").exists());
}

#[test]
fn test_gate_log_custom_path() {
    let temp = tempfile::tempdir().unwrap();
    let repo = seeded_repo(temp.path());

    // A relative value is a path under the repo root.
    let out = ctx_env(
        &repo.root,
        &["score", "--against", "HEAD"],
        &[("CTX_GATE_LOG", "logs/gates.jsonl")],
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(!repo.root.join(".ctx/gate-log.jsonl").exists());
    let lines = log_lines(&repo.root.join("logs/gates.jsonl"));
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["outcome"], "pass");
    // No --fail-on: the raw expression is null.
    assert!(lines[0]["fail_on"].is_null());
}
