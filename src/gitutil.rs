//! Shared git helpers for quality commands.
//!
//! All helpers shell out to `git` (matching the style of [`crate::diff`]) and
//! operate on the process working directory. Paths returned by these helpers
//! are relative to the working directory (the repo prefix is stripped), so
//! they line up with index-relative paths in `.ctx/codebase.sqlite`.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Command, Output};

use crate::error::{CtxError, Result};

/// Check if the current directory is inside a git repository.
pub fn is_git_repo() -> bool {
    is_git_repo_in(Path::new("."))
}

/// Get the path of the current directory relative to the repository root.
///
/// Returns `""` at the repo root, or a prefix like `"sub/dir/"` (with a
/// trailing slash) inside a subdirectory.
pub fn repo_prefix() -> Result<String> {
    repo_prefix_in(Path::new("."))
}

/// Get the set of files changed relative to `reference`.
///
/// The result is the union of:
/// - `git diff --name-only <reference>...HEAD` (committed changes since the
///   merge base with `reference`)
/// - `git diff --name-only HEAD` (uncommitted working-tree changes)
/// - `git ls-files --others --exclude-standard` (untracked files)
///
/// Paths are relative to the current directory; files outside it are dropped.
pub fn changed_files_against(reference: &str) -> Result<HashSet<String>> {
    changed_files_against_in(Path::new("."), reference)
}

/// Count how many commits touched each file since `since` (a `git log --since`
/// date spec, e.g. `"6 months ago"` or `"2025-01-01"`).
///
/// Paths are relative to the current directory; only files under it are
/// counted.
pub fn churn_since(since: &str) -> Result<HashMap<String, u32>> {
    churn_since_in(Path::new("."), since)
}

/// Get the contents of `path` (relative to the current directory) at
/// `reference`, or `None` if the file does not exist at that revision.
pub fn show_file(reference: &str, path: &str) -> Result<Option<String>> {
    show_file_in(Path::new("."), reference, path)
}

// ============================================================================
// Directory-explicit implementations (used directly by tests)
// ============================================================================

/// Dir-explicit variant of [`is_git_repo`], for commands and tests that
/// operate on an explicit project root instead of the process cwd.
pub fn is_git_repo_in(dir: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo_prefix_in(dir: &Path) -> Result<String> {
    let output = run_git(dir, &["rev-parse", "--show-prefix"])?;
    let stdout = stdout_or_err(output, None)?;
    Ok(stdout.trim_end_matches(['\n', '\r']).to_string())
}

/// Dir-explicit variant of [`changed_files_against`], for commands and tests
/// that operate on an explicit project root instead of the process cwd.
pub fn changed_files_against_in(dir: &Path, reference: &str) -> Result<HashSet<String>> {
    if !is_git_repo_in(dir) {
        return Err(CtxError::NotGitRepo);
    }

    let prefix = repo_prefix_in(dir)?;
    let mut files = HashSet::new();

    // Committed changes since the merge base with the reference.
    let range = format!("{}...HEAD", reference);
    let output = run_git(dir, &["diff", "--name-only", &range])?;
    let committed = stdout_or_err(output, Some(reference))?;
    collect_paths(&committed, &prefix, &mut files);

    // Uncommitted (staged + unstaged) changes.
    let output = run_git(dir, &["diff", "--name-only", "HEAD"])?;
    let uncommitted = stdout_or_err(output, None)?;
    collect_paths(&uncommitted, &prefix, &mut files);

    // Untracked files (--full-name makes paths repo-root-relative like diff).
    let output = run_git(
        dir,
        &["ls-files", "--others", "--exclude-standard", "--full-name"],
    )?;
    let untracked = stdout_or_err(output, None)?;
    collect_paths(&untracked, &prefix, &mut files);

    Ok(files)
}

fn churn_since_in(dir: &Path, since: &str) -> Result<HashMap<String, u32>> {
    if !is_git_repo_in(dir) {
        return Err(CtxError::NotGitRepo);
    }

    let prefix = repo_prefix_in(dir)?;
    let since_arg = format!("--since={}", since);
    let output = run_git(
        dir,
        &[
            "log",
            &since_arg,
            "--format=",
            "--name-only",
            "--no-renames",
            "--",
            ".",
        ],
    )?;
    let log = stdout_or_err(output, None)?;

    let mut churn = HashMap::new();
    for (path, count) in parse_name_only_log(&log) {
        if let Some(local) = strip_repo_prefix(&path, &prefix) {
            *churn.entry(local).or_insert(0) += count;
        }
    }
    Ok(churn)
}

/// Dir-explicit variant of [`show_file`], for commands and tests that
/// operate on an explicit project root instead of the process cwd.
pub fn show_file_in(dir: &Path, reference: &str, path: &str) -> Result<Option<String>> {
    if !is_git_repo_in(dir) {
        return Err(CtxError::NotGitRepo);
    }

    // "REF:./path" makes git resolve the path relative to the current
    // directory, matching index-relative paths.
    let spec = format!("{}:./{}", reference, path);
    let output = run_git(dir, &["show", &spec])?;

    if output.status.success() {
        return Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("does not exist") || stderr.contains("exists on disk, but not in") {
        return Ok(None);
    }
    // Reuse the shared error mapping for bad revisions and other failures.
    stdout_or_err(output, Some(reference)).map(Some)
}

// ============================================================================
// Helpers
// ============================================================================

/// Run a git command in `dir`, returning the raw output.
fn run_git(dir: &Path, args: &[&str]) -> Result<Output> {
    Ok(Command::new("git").args(args).current_dir(dir).output()?)
}

/// Convert a git `Output` into its stdout, mapping failures to `CtxError`.
///
/// If `revision` is given, revision-related failures map to
/// [`CtxError::InvalidRevision`] with that revision.
fn stdout_or_err(output: Output, revision: Option<&str>) -> Result<String> {
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("not a git repository") {
        return Err(CtxError::NotGitRepo);
    }
    if let Some(rev) = revision {
        if stderr.contains("unknown revision")
            || stderr.contains("bad revision")
            || stderr.contains("invalid object name")
            || stderr.contains("bad object")
        {
            return Err(CtxError::InvalidRevision(rev.to_string()));
        }
    }
    Err(CtxError::git(stderr.trim().to_string()))
}

/// Parse `git log --format= --name-only` output into per-path commit counts.
///
/// Blank lines (commit separators) are skipped; each non-blank line is a path
/// that was touched by one commit.
fn parse_name_only_log(s: &str) -> HashMap<String, u32> {
    let mut counts = HashMap::new();
    for line in s.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        *counts.entry(line.to_string()).or_insert(0) += 1;
    }
    counts
}

/// Strip the repo prefix from a repo-root-relative path.
///
/// Returns `None` for paths outside the prefix (i.e. outside the current
/// directory).
fn strip_repo_prefix(path: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        Some(path.to_string())
    } else {
        path.strip_prefix(prefix).map(|p| p.to_string())
    }
}

/// Add each non-blank, prefix-local path in `raw` to `files`.
fn collect_paths(raw: &str, prefix: &str, files: &mut HashSet<String>) {
    for line in raw.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        if let Some(local) = strip_repo_prefix(line, prefix) {
            files.insert(local);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::GitRepo;

    #[test]
    fn test_parse_name_only_log() {
        let log = "src/a.rs\nsrc/b.rs\n\nsrc/a.rs\n\n\nsrc/c.rs\n";
        let counts = parse_name_only_log(log);
        assert_eq!(counts.len(), 3);
        assert_eq!(counts.get("src/a.rs"), Some(&2));
        assert_eq!(counts.get("src/b.rs"), Some(&1));
        assert_eq!(counts.get("src/c.rs"), Some(&1));
    }

    #[test]
    fn test_parse_name_only_log_empty() {
        assert!(parse_name_only_log("").is_empty());
        assert!(parse_name_only_log("\n\n\n").is_empty());
    }

    #[test]
    fn test_strip_repo_prefix() {
        assert_eq!(
            strip_repo_prefix("src/a.rs", ""),
            Some("src/a.rs".to_string())
        );
        assert_eq!(
            strip_repo_prefix("sub/src/a.rs", "sub/"),
            Some("src/a.rs".to_string())
        );
        assert_eq!(strip_repo_prefix("other/a.rs", "sub/"), None);
    }

    #[test]
    fn test_is_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_git_repo_in(dir.path()));

        let repo = GitRepo::init(dir.path());
        assert!(is_git_repo_in(&repo.root));
    }

    #[test]
    fn test_changed_files_against() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path());
        repo.commit_file("src/a.rs", "fn a() {}", "initial");

        repo.branch("feature");
        repo.commit_file("src/b.rs", "fn b() {}", "add b");

        // Uncommitted modification + untracked file.
        repo.write("src/a.rs", "fn a() { /* changed */ }");
        repo.write("src/c.rs", "fn c() {}");

        let changed = changed_files_against_in(&repo.root, "main").unwrap();
        assert_eq!(changed.len(), 3);
        assert!(changed.contains("src/a.rs"));
        assert!(changed.contains("src/b.rs"));
        assert!(changed.contains("src/c.rs"));
    }

    #[test]
    fn test_changed_files_strips_prefix_in_subdir() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path());
        repo.write("top.rs", "fn top() {}");
        repo.write("sub/x.rs", "fn x() {}");
        repo.commit_all("initial");

        repo.branch("feature");
        repo.write("top.rs", "fn top() { /* changed */ }");
        repo.write("sub/x.rs", "fn x() { /* changed */ }");
        repo.commit_all("change both");

        let subdir = repo.root.join("sub");
        let changed = changed_files_against_in(&subdir, "main").unwrap();
        // Only files under sub/, with the prefix stripped.
        assert_eq!(changed.len(), 1);
        assert!(changed.contains("x.rs"));
    }

    #[test]
    fn test_changed_files_bad_reference() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path());
        repo.commit_file("a.rs", "fn a() {}", "initial");

        let err = changed_files_against_in(&repo.root, "no-such-ref").unwrap_err();
        assert!(
            matches!(err, CtxError::InvalidRevision(ref r) if r == "no-such-ref"),
            "expected InvalidRevision, got: {}",
            err
        );
    }

    #[test]
    fn test_changed_files_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let err = changed_files_against_in(dir.path(), "main").unwrap_err();
        assert!(matches!(err, CtxError::NotGitRepo));
    }

    #[test]
    fn test_churn_since() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path());
        repo.commit_file("src/a.rs", "v1", "one");
        repo.commit_file("src/a.rs", "v2", "two");
        repo.commit_file("src/b.rs", "v1", "three");

        let churn = churn_since_in(&repo.root, "2000-01-01").unwrap();
        assert_eq!(churn.get("src/a.rs"), Some(&2));
        assert_eq!(churn.get("src/b.rs"), Some(&1));
    }

    #[test]
    fn test_churn_since_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let err = churn_since_in(dir.path(), "1 week ago").unwrap_err();
        assert!(matches!(err, CtxError::NotGitRepo));
    }

    #[test]
    fn test_show_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path());
        repo.commit_file("src/a.rs", "fn a() {}", "initial");

        let content = show_file_in(&repo.root, "HEAD", "src/a.rs").unwrap();
        assert_eq!(content.as_deref(), Some("fn a() {}"));

        // Missing file at the revision -> None.
        let missing = show_file_in(&repo.root, "HEAD", "src/nope.rs").unwrap();
        assert!(missing.is_none());

        // Bad revision -> error.
        let err = show_file_in(&repo.root, "no-such-ref", "src/a.rs").unwrap_err();
        assert!(matches!(err, CtxError::InvalidRevision(_)));
    }
}
