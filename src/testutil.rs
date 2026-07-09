//! Test utilities shared between the library and binary test suites.
//!
//! This module is always compiled (not `#[cfg(test)]`) so that integration
//! tests in the binary crate can use it, but it is `#[doc(hidden)]` and not
//! part of the supported public API.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A scratch git repository for tests.
///
/// All git operations run with the repository root as the working directory,
/// so tests never need to change the process-wide current directory.
pub struct GitRepo {
    pub root: PathBuf,
}

impl GitRepo {
    /// Initialize a fresh git repository in `dir` with a local test identity.
    pub fn init(dir: &Path) -> GitRepo {
        let repo = GitRepo {
            root: dir.to_path_buf(),
        };
        std::fs::create_dir_all(dir).expect("failed to create repo dir");
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["config", "user.name", "Test User"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    /// Write a file (creating parent directories) relative to the repo root.
    pub fn write(&self, rel: &str, content: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent dirs");
        }
        std::fs::write(&path, content).expect("failed to write file");
    }

    /// Stage everything and commit with the given message.
    pub fn commit_all(&self, msg: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--no-gpg-sign", "-m", msg]);
    }

    /// Write a single file and commit it.
    pub fn commit_file(&self, rel: &str, content: &str, msg: &str) {
        self.write(rel, content);
        self.commit_all(msg);
    }

    /// Create and switch to a new branch.
    pub fn branch(&self, name: &str) {
        self.git(&["checkout", "-q", "-b", name]);
    }

    /// Run a git command in the repo root, panicking on failure.
    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .expect("failed to spawn git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
