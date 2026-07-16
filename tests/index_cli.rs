//! End-to-end CLI tests for `ctx index` scoping via positional patterns
//! and `-p/--pattern` (see Linear AGE-13: positional paths were silently
//! ignored and the whole repository was indexed).

use std::path::Path;
use std::process::{Command, Output};

use ctx::testutil::GitRepo;
use tempfile::TempDir;

fn ctx(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run ctx binary")
}

/// A repo with Rust files in two top-level directories, so scoping to one
/// of them is observable in the index contents.
fn fixture() -> TempDir {
    let temp = TempDir::new().expect("create temp dir");
    let repo = GitRepo::init(temp.path());
    repo.write("src/a.rs", "pub fn alpha() {}\n");
    repo.write("src/nested/b.rs", "pub fn beta() {}\n");
    repo.write("lib/c.rs", "pub fn gamma() {}\n");
    temp
}

/// Index with the given arguments, then return the `ctx query files` listing.
fn index_and_list(dir: &Path, index_args: &[&str]) -> (Output, String) {
    let index_out = ctx(dir, index_args);
    assert!(
        index_out.status.success(),
        "`ctx {}` failed: {}",
        index_args.join(" "),
        String::from_utf8_lossy(&index_out.stderr)
    );
    let list = ctx(dir, &["query", "files"]);
    assert!(list.status.success());
    (index_out, String::from_utf8_lossy(&list.stdout).to_string())
}

#[test]
fn positional_directory_scopes_index() {
    let temp = fixture();
    let (index_out, files) = index_and_list(temp.path(), &["index", "src"]);

    assert!(files.contains("src/a.rs"), "files: {}", files);
    assert!(files.contains("src/nested/b.rs"), "files: {}", files);
    assert!(!files.contains("lib/c.rs"), "files: {}", files);

    // The banner echoes the effective scope before parsing starts.
    let stderr = String::from_utf8_lossy(&index_out.stderr);
    assert!(stderr.contains("scoped to: src"), "stderr: {}", stderr);
}

#[test]
fn positional_directory_with_trailing_slash_scopes_index() {
    let temp = fixture();
    let (_, files) = index_and_list(temp.path(), &["index", "src/"]);

    assert!(files.contains("src/a.rs"), "files: {}", files);
    assert!(!files.contains("lib/c.rs"), "files: {}", files);
}

#[test]
fn positional_glob_scopes_index() {
    let temp = fixture();
    let (_, files) = index_and_list(temp.path(), &["index", "src/**"]);

    assert!(files.contains("src/a.rs"), "files: {}", files);
    assert!(files.contains("src/nested/b.rs"), "files: {}", files);
    assert!(!files.contains("lib/c.rs"), "files: {}", files);
}

#[test]
fn pattern_flag_literal_directory_scopes_index() {
    let temp = fixture();
    let (_, files) = index_and_list(temp.path(), &["index", "-p", "src"]);

    assert!(files.contains("src/a.rs"), "files: {}", files);
    assert!(files.contains("src/nested/b.rs"), "files: {}", files);
    assert!(!files.contains("lib/c.rs"), "files: {}", files);
}

#[test]
fn positional_and_flag_patterns_combine() {
    let temp = fixture();
    let (_, files) = index_and_list(temp.path(), &["index", "lib", "-p", "src/**"]);

    assert!(files.contains("src/a.rs"), "files: {}", files);
    assert!(files.contains("src/nested/b.rs"), "files: {}", files);
    assert!(files.contains("lib/c.rs"), "files: {}", files);
}

#[test]
fn no_patterns_index_everything() {
    let temp = fixture();
    let (index_out, files) = index_and_list(temp.path(), &["index"]);

    assert!(files.contains("src/a.rs"), "files: {}", files);
    assert!(files.contains("lib/c.rs"), "files: {}", files);

    let stderr = String::from_utf8_lossy(&index_out.stderr);
    assert!(!stderr.contains("scoped to"), "stderr: {}", stderr);
}

#[test]
fn explicit_dot_is_unscoped() {
    let temp = fixture();
    let (index_out, files) = index_and_list(temp.path(), &["index", "."]);

    assert!(files.contains("src/a.rs"), "files: {}", files);
    assert!(files.contains("lib/c.rs"), "files: {}", files);

    let stderr = String::from_utf8_lossy(&index_out.stderr);
    assert!(!stderr.contains("scoped to"), "stderr: {}", stderr);
}

#[test]
fn zero_match_pattern_errors_on_fresh_index() {
    let temp = fixture();
    let out = ctx(temp.path(), &["index", "nomatch/**"]);
    assert!(!out.status.success());

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("matched no files"), "stderr: {}", stderr);
}

#[test]
fn zero_match_pattern_does_not_wipe_existing_index() {
    let temp = fixture();
    let (_, files) = index_and_list(temp.path(), &["index"]);
    assert!(files.contains("src/a.rs"), "files: {}", files);

    // A typo'd scope must refuse to update rather than treating every
    // previously indexed file as deleted.
    let out = ctx(temp.path(), &["index", "nomatch/**"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("refusing to update the index"),
        "stderr: {}",
        stderr
    );

    let list = ctx(temp.path(), &["query", "files"]);
    assert!(list.status.success());
    let files = String::from_utf8_lossy(&list.stdout);
    assert!(files.contains("src/a.rs"), "files: {}", files);
    assert!(files.contains("lib/c.rs"), "files: {}", files);
}

#[test]
fn default_context_command_does_not_warn_in_empty_repo() {
    // The default `ctx` command always passes the `.` positional default;
    // an empty repository must not trigger the scoped zero-match warning.
    let temp = TempDir::new().expect("create temp dir");
    GitRepo::init(temp.path());

    let out = ctx(temp.path(), &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("include patterns matched no files"),
        "stderr: {}",
        stderr
    );
}
