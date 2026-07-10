//! End-to-end tests for the deterministic synthetic-repo generator
//! (`ctx::fixture`): byte-level determinism, indexability of the output by
//! the real `ctx` binary, synthesized history, and change-set application.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ctx::fixture::{apply_change_set, generate, FixtureSpec};
use ctx::index::open_database;
use ctx::testutil::git_stdout;

/// Recursive snapshot of a tree as sorted (relative path, bytes) pairs,
/// skipping `.git`. Two identical snapshots mean byte-identical trees.
fn tree_snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).expect("read_dir failed") {
            let path = entry.expect("dir entry failed").path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if path.is_dir() {
                collect(root, &path, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("path under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read(&path).expect("read file failed")));
            }
        }
    }
    let mut files = Vec::new();
    collect(root, root, &mut files);
    files.sort();
    files
}

#[test]
fn same_spec_is_byte_identical_including_commit_shas() {
    let spec = FixtureSpec::tiny();
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    generate(&spec, dir_a.path()).unwrap();
    generate(&spec, dir_b.path()).unwrap();

    let snap_a = tree_snapshot(dir_a.path());
    assert!(!snap_a.is_empty(), "generator produced no files");
    assert_eq!(
        snap_a,
        tree_snapshot(dir_b.path()),
        "same spec must produce byte-identical trees"
    );

    // Identical trees, messages, identity, and dates => identical SHAs.
    assert_eq!(
        git_stdout(dir_a.path(), &["rev-parse", "HEAD"]),
        git_stdout(dir_b.path(), &["rev-parse", "HEAD"]),
        "same spec must produce identical commit SHAs"
    );

    // A different seed must produce a different tree.
    let mut other = spec.clone();
    other.seed ^= 1;
    let dir_c = tempfile::tempdir().unwrap();
    generate(&other, dir_c.path()).unwrap();
    assert_ne!(snap_a, tree_snapshot(dir_c.path()));

    // Refusing to clobber: the root must be empty or nonexistent.
    assert!(generate(&spec, dir_a.path()).is_err());
}

#[test]
fn generated_repo_indexes_with_symbols_and_cross_file_call_edges() {
    let spec = FixtureSpec::tiny();
    let dir = tempfile::tempdir().unwrap();
    generate(&spec, dir.path()).unwrap();

    let out: Output = Command::new(env!("CARGO_BIN_EXE_ctx"))
        .arg("index")
        .current_dir(dir.path())
        .output()
        .expect("failed to run ctx binary");
    assert!(
        out.status.success(),
        "ctx index failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let db = open_database(dir.path()).unwrap();

    // Every file contributes several symbols (consts, struct, method, fns).
    let symbols = db.count_symbols().unwrap();
    assert!(
        symbols > spec.files as i64,
        "expected more symbols than files, got {symbols}"
    );

    // Bare-identifier calls must materialize as resolved cross-file edges.
    let cross = db.get_cross_file_edges().unwrap();
    assert!(
        !cross.is_empty(),
        "expected resolved cross-file edges in the index"
    );
    assert!(
        cross
            .iter()
            .any(|e| e.kind == "calls" && e.source.file_path != e.target.file_path),
        "expected at least one cross-file 'calls' edge"
    );

    // Complexity must vary across generated functions.
    let metrics = db.symbol_metrics().unwrap();
    let complexities: Vec<i64> = metrics
        .iter()
        .filter(|m| m.kind == "function")
        .map(|m| m.complexity)
        .collect();
    assert!(!complexities.is_empty());
    let min = complexities.iter().min().unwrap();
    let max = complexities.iter().max().unwrap();
    assert!(
        max > min,
        "complexity should vary across symbols (min {min}, max {max})"
    );
}

#[test]
fn history_has_initial_plus_configured_churn_commits() {
    let spec = FixtureSpec::tiny();
    let dir = tempfile::tempdir().unwrap();
    generate(&spec, dir.path()).unwrap();

    let log = git_stdout(dir.path(), &["log", "--oneline"]);
    assert_eq!(
        log.lines().count(),
        1 + spec.history_commits,
        "expected 1 initial + {} churn commits:\n{log}",
        spec.history_commits
    );

    // generate() must leave the working tree clean.
    assert_eq!(git_stdout(dir.path(), &["status", "--porcelain"]), "");
}

#[test]
fn apply_change_set_rewrites_exactly_the_selected_files() {
    let spec = FixtureSpec::tiny();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    generate(&spec, root).unwrap();

    let changed = apply_change_set(&spec, root, 3, 0).unwrap();
    assert_eq!(changed.len(), 3);
    for path in &changed {
        assert!(path.exists(), "missing changed file {}", path.display());
    }

    // git sees exactly the returned paths as modified (and nothing staged).
    let mut dirty: Vec<String> = git_stdout(root, &["status", "--porcelain"])
        .lines()
        .map(|line| {
            let (status, rel) = line.split_at(3);
            assert_eq!(status, " M ", "unexpected status line: {line}");
            rel.to_string()
        })
        .collect();
    dirty.sort();
    let mut expected: Vec<String> = changed
        .iter()
        .map(|p| {
            p.strip_prefix(root)
                .expect("changed path under root")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    expected.sort();
    assert_eq!(dirty, expected);

    // Same (spec, n, round) => same selection and identical bytes.
    let before: Vec<Vec<u8>> = changed.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let again = apply_change_set(&spec, root, 3, 0).unwrap();
    assert_eq!(changed, again);
    let after: Vec<Vec<u8>> = again.iter().map(|p| std::fs::read(p).unwrap()).collect();
    assert_eq!(before, after);

    // Different round => different content for the same files. Rewriting all
    // files makes both rounds cover the same set, so contents are comparable.
    let all_round0: Vec<PathBuf> = apply_change_set(&spec, root, spec.files, 0).unwrap();
    assert_eq!(all_round0.len(), spec.files);
    let round0: Vec<Vec<u8>> = all_round0
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    let all_round1 = apply_change_set(&spec, root, spec.files, 1).unwrap();
    assert_eq!(all_round0, all_round1, "full selections cover the same set");
    for (path, old) in all_round0.iter().zip(&round0) {
        assert_ne!(
            &std::fs::read(path).unwrap(),
            old,
            "round 1 must rewrite {} with different content",
            path.display()
        );
    }

    // Requesting more files than the spec has is an error.
    assert!(apply_change_set(&spec, root, spec.files + 1, 0).is_err());
}

/// Perf guard for the 2,000-file preset. Ignored by default to keep the
/// suite fast; run with `cargo test --test fixture_gen -- --ignored`.
#[test]
#[ignore = "slow perf smoke test; run explicitly with --ignored"]
fn repo_2k_generates_in_under_five_seconds() {
    let spec = FixtureSpec::repo_2k();
    let dir = tempfile::tempdir().unwrap();
    let start = std::time::Instant::now();
    generate(&spec, dir.path()).unwrap();
    let elapsed = start.elapsed();
    println!("repo_2k generated in {elapsed:?}");
    assert!(
        elapsed.as_secs() < 5,
        "repo_2k generation took {elapsed:?}, budget is 5s"
    );
}
