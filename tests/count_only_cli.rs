//! Regression coverage for the global token-counting flags on context commands.

use std::path::Path;
use std::process::{Command, Output};

#[cfg(feature = "duckdb")]
use ctx::db::Database;
use ctx::testutil::GitRepo;
#[cfg(feature = "duckdb")]
use ctx::testutil::MockServer;

fn ctx(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args(args)
        .current_dir(dir)
        .env("CTX_NO_UPDATE_CHECK", "1")
        .output()
        .expect("failed to run ctx")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "ctx failed with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn root_count_only_remains_plain_under_json() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("one.txt"), "hello world\n").unwrap();

    let output = ctx(temp.path(), &["--json", "--count-only", "one.txt"]);
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("Files: 1\n"), "{stdout}");
    assert!(stdout.contains("Tokens (cl100k_base):"), "{stdout}");
    assert!(!stdout.trim_start().starts_with('{'), "{stdout}");
}

#[test]
fn diff_count_only_uses_encoding_budget_and_output_streams() {
    let temp = tempfile::tempdir().unwrap();
    let repo = GitRepo::init(temp.path());
    repo.write("a.txt", "small\n");
    repo.write("b.txt", "before\n");
    repo.commit_all("base");
    repo.write("a.txt", "small changed\n");
    repo.write("b.txt", &"large changed content ".repeat(200));

    let budget = ctx::tokens::count_tokens_with_encoding(
        "small changed\n",
        ctx::tokens::Encoding::O200kBase,
    )
    .unwrap()
    .to_string();
    let output = ctx(
        &repo.root,
        &[
            "diff",
            "HEAD",
            "--changes-only",
            "--summary",
            "--count-only",
            "--stats",
            "--encoding",
            "o200k_base",
            "--max-tokens",
            &budget,
        ],
    );
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.starts_with("Files: 1\n"), "{stdout}");
    assert!(stdout.contains("Tokens (o200k_base):"), "{stdout}");
    assert!(!stdout.contains("small changed"), "{stdout}");
    assert!(!stdout.contains("large changed"), "{stdout}");
    assert!(
        stderr.contains("Changes in HEAD: 2 files changed"),
        "{stderr}"
    );
    assert!(stderr.contains("context 1 files"), "{stderr}");
    assert!(stderr.contains("1 omitted"), "{stderr}");
    assert!(stderr.contains("Counted in"), "{stderr}");
}

#[test]
fn smart_and_diff_reject_invalid_encoding_with_exit_two() {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        vec!["diff", "--encoding", "not-an-encoding"],
        vec!["smart", "task", "--encoding", "not-an-encoding"],
    ] {
        let output = ctx(temp.path(), &args);
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Invalid encoding 'not-an-encoding'"),
            "args: {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty(), "args: {args:?}");
    }
}

#[cfg(feature = "duckdb")]
#[test]
fn smart_count_only_suppresses_dry_run_and_explain_output() {
    let temp = tempfile::tempdir().unwrap();
    let repo = GitRepo::init(temp.path());
    repo.write(
        "src/alpha.rs",
        "pub fn alpha() { println!(\"selected file content\"); }\n",
    );
    repo.write(
        "src/beta.rs",
        "pub fn beta() { println!(\"other file content\"); }\n",
    );
    assert_success(&ctx(&repo.root, &["index"]));

    {
        let db = Database::open(&repo.root.join(".ctx/codebase.sqlite")).unwrap();
        for symbol_id in db.get_all_symbol_ids().unwrap() {
            db.store_embedding(&symbol_id, "ollama", "test", &[1.0, 0.0, 0.0, 0.0])
                .unwrap();
        }
    }

    let server = MockServer::start();
    server.add_route(
        "/api/embed",
        "application/json",
        r#"{"embeddings":[[1.0,0.0,0.0,0.0]]}"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args([
            "smart",
            "find alpha",
            "--provider",
            "ollama",
            "--count-only",
            "--dry-run",
            "--explain",
            "--stats",
            "--encoding",
            "o200k_base",
            "--max-tokens",
            "1",
        ])
        .current_dir(&repo.root)
        .env("CTX_NO_UPDATE_CHECK", "1")
        .env("OLLAMA_HOST", server.url())
        .output()
        .unwrap();
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.starts_with("Files: 1\n"), "{stdout}");
    assert!(stdout.contains("Tokens (o200k_base):"), "{stdout}");
    assert!(!stdout.contains("Would select"), "{stdout}");
    assert!(!stdout.contains("selected file content"), "{stdout}");
    assert!(!stderr.contains("Selection reasoning"), "{stderr}");
    assert!(stderr.contains("Counted in"), "{stderr}");
    assert!(server.hits() >= 2);
}
