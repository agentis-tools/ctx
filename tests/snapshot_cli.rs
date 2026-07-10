//! Integration tests for the `ctx snapshot` subcommand.
//!
//! These run the real `ctx` binary against real git repositories and assert
//! on the Parquet partition layout, the JSON envelopes, and backfill's
//! worktree hygiene. Assertions use substring matching and exit-code checks
//! (like tests/sql.rs); Parquet *content* readability is covered by the unit
//! tests in `src/snapshot.rs`.

// The whole suite drives snapshot capture, which requires the duckdb feature
// (Windows CI runs --no-default-features, where `ctx snapshot` exits 2 —
// covered by tests/snapshot_stub.rs).
#![cfg(feature = "duckdb")]

use std::path::Path;

use assert_cmd::Command;
use ctx::testutil::{git_stdout, GitRepo};
use predicates::prelude::*;
use tempfile::TempDir;

const V1: &str = r#"
pub fn alpha() -> i64 {
    beta()
}

pub fn beta() -> i64 {
    1
}
"#;

const V2: &str = r#"
pub fn alpha() -> i64 {
    beta() + gamma()
}

pub fn beta() -> i64 {
    1
}

pub fn gamma() -> i64 {
    2
}
"#;

/// A repo with three dated commits and a built index.
fn snapshot_fixture() -> (TempDir, GitRepo) {
    let temp = TempDir::new().expect("create temp dir");
    let repo = GitRepo::init(temp.path());
    repo.write("src/a.rs", V1);
    repo.commit_all_with_date("one", "2024-01-01T12:00:00 +0000");
    repo.write("src/a.rs", V2);
    repo.commit_all_with_date("two", "2024-02-01T12:00:00 +0000");
    repo.write("src/b.rs", "pub fn delta() -> i64 { 4 }\n");
    repo.commit_all_with_date("three", "2024-03-01T12:00:00 +0000");

    ctx(&repo.root).arg("index").assert().success();
    (temp, repo)
}

/// A `ctx` command rooted in `dir`.
fn ctx(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("ctx").unwrap();
    cmd.current_dir(dir);
    cmd
}

/// The snapshot partition directory for a sha.
fn partition(dir: &Path, sha: &str) -> std::path::PathBuf {
    dir.join(".ctx")
        .join("snapshots")
        .join(format!("sha={}", sha))
}

const PARQUET_FILES: [&str; 4] = [
    "symbols.parquet",
    "files.parquet",
    "dup_pairs.parquet",
    "meta.parquet",
];

// ---------------------------------------------------------------------------
// Capture: partition layout, JSON envelope, skip/--force semantics.
// ---------------------------------------------------------------------------

#[test]
fn capture_creates_partition_with_all_parquet_files() {
    let (_temp, repo) = snapshot_fixture();
    let head = git_stdout(&repo.root, &["rev-parse", "HEAD"]);

    ctx(&repo.root)
        .args(["snapshot", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"command\": \"snapshot.capture\"",
        ))
        .stdout(predicate::str::contains(&head))
        .stdout(predicate::str::contains("\"skipped_existing\": false"))
        // The fixture indexes real symbols/files; counts must be non-zero.
        .stdout(predicate::str::contains("\"symbols\": 0").not())
        .stdout(predicate::str::contains("\"files\": 0").not());

    let part = partition(&repo.root, &head);
    assert!(part.is_dir(), "missing partition {}", part.display());
    for name in PARQUET_FILES {
        assert!(
            part.join(name).is_file(),
            "missing {} in {}",
            name,
            part.display()
        );
    }
    // No staging leftovers.
    assert!(!part.with_extension("tmp").exists());
}

#[test]
fn second_capture_skips_and_force_rewrites() {
    let (_temp, repo) = snapshot_fixture();
    let head = git_stdout(&repo.root, &["rev-parse", "HEAD"]);

    ctx(&repo.root).arg("snapshot").assert().success();

    // Second run: the partition exists, so nothing is rewritten.
    ctx(&repo.root)
        .args(["snapshot", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"skipped_existing\": true"));

    // --force rewrites the partition in place.
    ctx(&repo.root)
        .args(["snapshot", "--force", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"skipped_existing\": false"))
        .stdout(predicate::str::contains("\"symbols\": 0").not());

    let part = partition(&repo.root, &head);
    for name in PARQUET_FILES {
        assert!(part.join(name).is_file(), "missing {} after --force", name);
    }
}

#[test]
fn outside_a_git_repo_is_an_error() {
    let temp = TempDir::new().unwrap();

    // Capture and backfill both need a repository; neither may leave
    // partitions behind on failure.
    ctx(temp.path())
        .arg("snapshot")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("git"));
    ctx(temp.path())
        .args(["snapshot", "backfill", "--since", "HEAD"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("git"));

    let snapshots = temp.path().join(".ctx").join("snapshots");
    assert!(
        !snapshots.exists(),
        "a failed snapshot must not create partitions"
    );
}

// ---------------------------------------------------------------------------
// Backfill: one partition per covered commit, worktree hygiene, and the
// working tree is left untouched.
// ---------------------------------------------------------------------------

#[test]
fn backfill_covers_all_commits_and_cleans_up_worktrees() {
    let (_temp, repo) = snapshot_fixture();
    let head = git_stdout(&repo.root, &["rev-parse", "HEAD"]);
    let first = git_stdout(&repo.root, &["rev-list", "--max-parents=0", "HEAD"]);
    let all: Vec<String> = git_stdout(&repo.root, &["rev-list", "--first-parent", "HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(all.len(), 3, "fixture has three commits");

    // A dirty working-tree file must survive the backfill untouched.
    let dirty = repo.root.join("src").join("dirty.rs");
    std::fs::write(&dirty, "pub fn uncommitted() {}\n").unwrap();

    ctx(&repo.root)
        .args(["snapshot", "backfill", "--since", &first, "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"command\": \"snapshot.backfill\"",
        ))
        .stdout(predicate::str::contains("\"captured\": 3"))
        .stdout(predicate::str::contains(&head));

    // One partition per covered commit (--since is inclusive), each complete.
    for sha in &all {
        let part = partition(&repo.root, sha);
        assert!(part.is_dir(), "missing partition for {}", sha);
        for name in PARQUET_FILES {
            assert!(part.join(name).is_file(), "missing {} for {}", name, sha);
        }
    }

    // Only the main worktree remains.
    let worktrees = git_stdout(&repo.root, &["worktree", "list"]);
    assert_eq!(
        worktrees.lines().count(),
        1,
        "backfill must remove its temporary worktrees:\n{}",
        worktrees
    );

    // The dirty file is still there, byte for byte.
    let content = std::fs::read_to_string(&dirty).unwrap();
    assert_eq!(content, "pub fn uncommitted() {}\n");
}

#[test]
fn backfill_skips_existing_partitions() {
    let (_temp, repo) = snapshot_fixture();
    let first = git_stdout(&repo.root, &["rev-list", "--max-parents=0", "HEAD"]);

    // Snapshot HEAD first; backfill must then only capture the other two.
    ctx(&repo.root).arg("snapshot").assert().success();
    ctx(&repo.root)
        .args(["snapshot", "backfill", "--since", &first, "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"captured\": 2"))
        .stdout(predicate::str::contains("\"skipped_existing\": 1"));
}

#[test]
fn backfill_every_samples_but_keeps_newest() {
    let (_temp, repo) = snapshot_fixture();
    let head = git_stdout(&repo.root, &["rev-parse", "HEAD"]);
    let first = git_stdout(&repo.root, &["rev-list", "--max-parents=0", "HEAD"]);

    // 3 commits, --every 2 -> oldest and newest (indices 0 and 2).
    ctx(&repo.root)
        .args(["snapshot", "backfill", "--since", &first, "--every", "2"])
        .assert()
        .success();

    assert!(partition(&repo.root, &head).is_dir(), "newest always kept");
    assert!(partition(&repo.root, &first).is_dir(), "oldest sampled");
    let snapshots = repo.root.join(".ctx").join("snapshots");
    let partitions = std::fs::read_dir(&snapshots)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("sha="))
        .count();
    assert_eq!(partitions, 2, "--every 2 over 3 commits samples 2");
}
