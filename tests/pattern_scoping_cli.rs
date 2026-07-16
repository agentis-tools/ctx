//! Offline end-to-end coverage for positional pattern scoping on commands
//! whose help advertises the shared `[PATTERNS]...` arguments.

use std::path::Path;
use std::process::{Command, Output};

use ctx::testutil::GitRepo;

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
fn similar_keyword_scopes_literal_directory_file_glob_and_or_patterns() {
    let temp = tempfile::tempdir().unwrap();
    let repo = GitRepo::init(temp.path());
    repo.write(
        "src/scoped.rs",
        "/// reusable checksum helper\nfn reusable_checksum_helper() {}\n",
    );
    repo.write(
        "docs/guide.rs",
        "/// reusable checksum helper guide\nfn reusable_checksum_helper_guide() {}\n",
    );
    repo.write(
        "tests/outside.rs",
        "/// reusable checksum helper test\nfn reusable_checksum_helper_test() {}\n",
    );

    assert_success(&ctx(&repo.root, &["index"]));

    for patterns in [vec!["src/"], vec!["src/**/*.rs"], vec!["src/scoped.rs"]] {
        let mut args = vec!["similar", "reusable checksum helper", "--keyword", "--json"];
        args.extend(patterns);
        let output = ctx(&repo.root, &args);
        assert_success(&output);
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let paths: Vec<_> = json["data"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|hit| hit["symbol"]["file"].as_str().unwrap())
            .collect();
        assert_eq!(paths, vec!["src/scoped.rs"], "args: {args:?}");
    }

    let output = ctx(
        &repo.root,
        &[
            "similar",
            "reusable checksum helper",
            "--keyword",
            "--json",
            "src/",
            "docs/guide.rs",
        ],
    );
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let paths: std::collections::BTreeSet<_> = json["data"]["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["symbol"]["file"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["docs/guide.rs", "src/scoped.rs"].into());

    let output = ctx(
        &repo.root,
        &["similar", "reusable checksum helper", "--keyword", "--json"],
    );
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["data"]["results"].as_array().unwrap().len(), 3);
}

#[test]
fn diff_scopes_changes_deletions_and_both_sides_of_renames() {
    let temp = tempfile::tempdir().unwrap();
    let repo = GitRepo::init(temp.path());
    repo.write("src/changed.rs", "fn changed() {}\n");
    repo.write("src/deleted.rs", "fn deleted() {}\n");
    repo.write("src/renamed.rs", "fn renamed() {}\n");
    repo.write("docs/outside.md", "before\n");
    repo.write("docs/caller.rs", "fn outside_caller() { changed(); }\n");
    repo.commit_all("base");
    assert_success(&ctx(&repo.root, &["index"]));

    repo.write(
        "src/changed.rs",
        "fn changed() { println!(\"changed\"); }\n",
    );
    std::fs::remove_file(repo.root.join("src/deleted.rs")).unwrap();
    std::fs::create_dir_all(repo.root.join("docs")).unwrap();
    let rename = Command::new("git")
        .args(["mv", "src/renamed.rs", "docs/renamed.rs"])
        .current_dir(&repo.root)
        .output()
        .unwrap();
    assert_success(&rename);
    repo.write("docs/outside.md", "after\n");

    let output = ctx(
        &repo.root,
        &["diff", "HEAD", "--changes-only", "--summary", "src/"],
    );
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stdout.contains("src/changed.rs"), "{stdout}");
    assert!(stdout.contains("docs/renamed.rs"), "{stdout}");
    assert!(!stdout.contains("docs/outside.md"), "{stdout}");
    assert!(stderr.contains("src/deleted.rs"), "{stderr}");
    assert!(stderr.contains("docs/renamed.rs"), "{stderr}");
    assert!(!stderr.contains("docs/outside.md"), "{stderr}");

    #[cfg(feature = "duckdb")]
    {
        let output = ctx(&repo.root, &["diff", "HEAD", "--summary", "--no-tree", "."]);
        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("docs/caller.rs"),
            "unfiltered graph expansion should include the indexed caller: {stdout}"
        );

        let output = ctx(
            &repo.root,
            &["diff", "HEAD", "--summary", "--no-tree", "src/"],
        );
        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("docs/caller.rs"),
            "scoped graph expansion escaped src/: {stdout}"
        );
    }
}

/// An empty scope is a legitimate answer to a narrower question, so it reports
/// nothing and warns rather than failing. A revision with no changes at all is
/// still an operational error, as it was before patterns were honored.
#[test]
fn diff_scope_matching_no_changes_warns_and_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    let repo = GitRepo::init(temp.path());
    repo.commit_file("src/a.rs", "fn a() {}\n", "init");
    repo.commit_file("docs/b.md", "docs\n", "add docs");
    repo.commit_file("src/a.rs", "fn a() { let _ = 1; }\n", "change src only");
    assert_success(&ctx(&repo.root, &["index"]));

    // Scope that matches changed files: succeeds and reports them.
    let hit = ctx(
        &repo.root,
        &["diff", "HEAD^", "--summary", "--no-tree", "src/"],
    );
    assert_success(&hit);

    // Scope that matches nothing: exit 0, warns, and reports no changed file.
    let miss = ctx(
        &repo.root,
        &["diff", "HEAD^", "--summary", "--no-tree", "docs/"],
    );
    assert_eq!(
        miss.status.code(),
        Some(0),
        "an empty scope must not be an operational error: {}",
        String::from_utf8_lossy(&miss.stderr)
    );
    let stderr = String::from_utf8_lossy(&miss.stderr);
    assert!(
        stderr.contains("no changed files match the requested scope"),
        "an empty scope must warn: {stderr}"
    );
    assert!(
        !String::from_utf8_lossy(&miss.stdout).contains("src/a.rs"),
        "an empty scope must not fall back to the unscoped result"
    );

    // No changes at all remains an operational error (pre-existing contract).
    let none = ctx(&repo.root, &["diff", "HEAD", "--summary", "--no-tree"]);
    assert_eq!(
        none.status.code(),
        Some(2),
        "a revision with no changes must stay an operational error"
    );
}
