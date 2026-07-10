//! `ctx snapshot` on a build without the duckdb feature: the CLI surface
//! still parses (help output stays stable on Windows), but running it is an
//! operational error (exit 2) pointing at the missing feature.
#![cfg(not(feature = "duckdb"))]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn snapshot_requires_the_duckdb_feature() {
    let temp = TempDir::new().unwrap();
    Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .arg("snapshot")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("duckdb"));
}

#[test]
fn snapshot_help_is_available_without_the_feature() {
    let temp = TempDir::new().unwrap();
    Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .args(["snapshot", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("backfill"));
}
