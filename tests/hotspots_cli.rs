//! End-to-end tests for `ctx hotspots`, driving the compiled binary against
//! real git repositories that are indexed with `ctx index`.

use std::path::Path;
use std::process::{Command, Output};

use ctx::testutil::GitRepo;

/// Run the ctx binary with `args` in `dir`.
fn ctx_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run ctx binary")
}

/// Run `ctx index` in `dir`, panicking on failure.
fn index_in(dir: &Path) {
    let out = ctx_in(dir, &["index"]);
    assert!(
        out.status.success(),
        "ctx index failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Parse the single JSON envelope a `--json` command printed to stdout.
fn parse_envelope(out: &Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is not valid JSON")
}

/// A function with several resolved calls (high complexity when indexed).
const COMPLEX_BODY: &str = r#"
fn h1() {}
fn h2() {}
fn h3() {}

fn busy_main() {
    h1();
    h2();
    h3();
    h1();
    h2();
}
"#;

/// Fixture: three indexed Rust files with different churn/complexity mixes.
///
/// - `hot.rs`: complex AND committed 3 times
/// - `complex_only.rs`: complex, committed once
/// - `churn_only.rs`: trivial, committed 3 times
fn ranking_fixture(dir: &Path) -> GitRepo {
    let repo = GitRepo::init(dir);
    repo.write("hot.rs", COMPLEX_BODY);
    repo.write(
        "complex_only.rs",
        &COMPLEX_BODY.replace("busy_main", "other_main"),
    );
    repo.write("churn_only.rs", "fn tiny() {}\n");
    repo.commit_all("initial");

    for i in 0..2 {
        repo.write("hot.rs", &format!("{}// rev {}\n", COMPLEX_BODY, i));
        repo.write("churn_only.rs", &format!("fn tiny() {{}}\n// rev {}\n", i));
        repo.commit_all(&format!("touch hot and churn_only {}", i));
    }

    index_in(&repo.root);
    repo
}

#[test]
fn test_hot_file_outranks_single_dimension_files() {
    let dir = tempfile::tempdir().unwrap();
    let repo = ranking_fixture(dir.path());

    let out = ctx_in(
        &repo.root,
        &[
            "hotspots",
            "--json",
            "--since",
            "20 years ago",
            "--min-churn",
            "1",
        ],
    );
    let envelope = parse_envelope(&out);
    assert_eq!(envelope["command"], "hotspots");

    let data = &envelope["data"];
    assert_eq!(data["by"], "file");
    assert_eq!(data["min_churn"], 1);
    assert_eq!(data["since"], "20 years ago");
    assert!(data["against"].is_null());

    let entries = data["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3, "all three indexed files are analyzed");

    // hot.rs is both complex and churned -> ranks first with score 1.0.
    assert_eq!(entries[0]["file"], "hot.rs");
    assert_eq!(entries[0]["commits"], 3);
    assert_eq!(entries[0]["score"], 1.0);
    let hot_score = entries[0]["score"].as_f64().unwrap();
    for entry in &entries[1..] {
        let score = entry["score"].as_f64().unwrap();
        assert!(score < hot_score, "hot.rs must outrank {}", entry["file"]);
        assert!((0.0..=1.0).contains(&score));
    }

    // Payload shape: snake_case keys, top-3 symbols max.
    let keys: Vec<&str> = entries[0]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    assert_eq!(
        keys,
        vec![
            "commits",
            "complexity",
            "fan_out",
            "file",
            "score",
            "symbols"
        ]
    );
    let symbols = entries[0]["symbols"].as_array().unwrap();
    assert!(!symbols.is_empty());
    assert!(
        symbols.len() <= 3,
        "at most 3 symbols, got {}",
        symbols.len()
    );
    // hot.rs has 4 functions, so the cap is actually exercised.
    assert_eq!(symbols.len(), 3);
    assert_eq!(symbols[0]["symbol"]["name"], "busy_main");
    assert_eq!(symbols[0]["symbol"]["file"], "hot.rs");
    assert!(symbols[0]["complexity"].as_i64().unwrap() > 0);
}

#[test]
fn test_by_symbol_ranks_symbols_with_file_churn() {
    let dir = tempfile::tempdir().unwrap();
    let repo = ranking_fixture(dir.path());

    let out = ctx_in(
        &repo.root,
        &[
            "hotspots",
            "--json",
            "--by",
            "symbol",
            "--since",
            "20 years ago",
            "--min-churn",
            "1",
        ],
    );
    let envelope = parse_envelope(&out);
    let data = &envelope["data"];
    assert_eq!(data["by"], "symbol");

    let entries = data["entries"].as_array().unwrap();
    assert!(!entries.is_empty());
    // The complex, frequently-committed function ranks first, carrying its
    // file's churn (the documented v1 approximation).
    assert_eq!(entries[0]["symbol"]["name"], "busy_main");
    assert_eq!(entries[0]["file"], "hot.rs");
    assert_eq!(entries[0]["commits"], 3);
    let keys: Vec<&str> = entries[0]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    assert_eq!(
        keys,
        vec![
            "commits",
            "complexity",
            "fan_out",
            "file",
            "score",
            "symbol"
        ]
    );
}

#[test]
fn test_since_window_changes_churn_counts() {
    let dir = tempfile::tempdir().unwrap();
    let repo = GitRepo::init(dir.path());

    // Two old commits (well outside any recent window) and one fresh commit.
    repo.write("a.rs", COMPLEX_BODY);
    repo.commit_all_with_date("old one", "2020-01-01T12:00:00 +0000");
    repo.write("a.rs", &format!("{}// old rev\n", COMPLEX_BODY));
    repo.commit_all_with_date("old two", "2020-01-02T12:00:00 +0000");
    repo.write("a.rs", &format!("{}// new rev\n", COMPLEX_BODY));
    repo.commit_all("recent");

    index_in(&repo.root);

    let commits_for = |since: &str| -> i64 {
        let out = ctx_in(
            &repo.root,
            &["hotspots", "--json", "--since", since, "--min-churn", "1"],
        );
        let envelope = parse_envelope(&out);
        let entries = envelope["data"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["file"], "a.rs");
        entries[0]["commits"].as_i64().unwrap()
    };

    assert_eq!(commits_for("20 years ago"), 3);
    assert_eq!(commits_for("2 weeks ago"), 1);
}

#[test]
fn test_not_a_git_repo_exits_2_with_empty_stdout() {
    let dir = tempfile::tempdir().unwrap();

    let out = ctx_in(dir.path(), &["hotspots"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        out.stdout.is_empty(),
        "stdout must be empty, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Not a git repository"),
        "stderr should explain the failure, got: {}",
        stderr
    );

    // Same contract in JSON mode: nothing on stdout, error on stderr.
    let out = ctx_in(dir.path(), &["hotspots", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn test_min_churn_excludes_low_churn_files_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let repo = ranking_fixture(dir.path());

    let out = ctx_in(
        &repo.root,
        &[
            "hotspots",
            "--json",
            "--since",
            "20 years ago",
            "--min-churn",
            "2",
        ],
    );
    let envelope = parse_envelope(&out);
    let entries = envelope["data"]["entries"].as_array().unwrap();

    // complex_only.rs was committed once and must be filtered out.
    let files: Vec<&str> = entries
        .iter()
        .map(|e| e["file"].as_str().unwrap())
        .collect();
    assert_eq!(files.len(), 2);
    assert!(files.contains(&"hot.rs"));
    assert!(files.contains(&"churn_only.rs"));
}
