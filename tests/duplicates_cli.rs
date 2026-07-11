//! End-to-end CLI tests for `ctx duplicates` and its indexing hook.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// A function with > 50 normalized tokens.
const DUPE_A: &str = r#"
pub fn process_orders(items: &[i64]) -> i64 {
    let mut total = 0;
    for item in items {
        if *item > 10 {
            total += *item * 2;
        } else {
            total += *item + 1;
        }
    }
    println!("processed the batch: {}", total);
    total
}
"#;

/// A structural copy of `DUPE_A` with renamed identifiers and different
/// string/number literals.
const DUPE_B: &str = r#"
pub fn sum_invoices(entries: &[i64]) -> i64 {
    let mut acc = 0;
    for entry in entries {
        if *entry > 99 {
            acc += *entry * 7;
        } else {
            acc += *entry + 3;
        }
    }
    println!("done with invoices: {}", acc);
    acc
}
"#;

/// A structurally unrelated function, also > 50 tokens.
const UNRELATED: &str = r#"
pub fn render_table(headers: &[String], widths: &[usize]) -> String {
    let mut out = String::new();
    for (header, width) in headers.iter().zip(widths.iter()) {
        out.push('|');
        out.push_str(header);
        while out.len() < *width {
            out.push(' ');
        }
    }
    out.push('\n');
    for width in widths {
        out.push_str(&"-".repeat(*width));
        out.push('+');
    }
    out
}
"#;

/// A Solidity function with > 50 normalized tokens.
const SOL_A: &str = r#"
pragma solidity ^0.8.0;

contract Ledger {
    function processOrders(uint256[] memory items) public pure returns (uint256) {
        uint256 total = 0;
        for (uint256 i = 0; i < items.length; i++) {
            if (items[i] > 10) {
                total += items[i] * 2;
            } else {
                total += items[i] + 1;
            }
        }
        return total;
    }
}
"#;

/// A structural copy of `SOL_A` with renamed identifiers and different
/// number literals.
const SOL_B: &str = r#"
pragma solidity ^0.8.0;

contract Register {
    function sumInvoices(uint256[] memory entries) public pure returns (uint256) {
        uint256 acc = 0;
        for (uint256 j = 0; j < entries.length; j++) {
            if (entries[j] > 99) {
                acc += entries[j] * 7;
            } else {
                acc += entries[j] + 3;
            }
        }
        return acc;
    }
}
"#;

fn ctx(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run ctx binary")
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn test_index_verbose_logs_fingerprints_incrementally() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "src/a.rs", DUPE_A);
    write(root, "src/b.rs", DUPE_B);

    // First index fingerprints every file.
    let out = ctx(root, &["index", "--verbose"]);
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Fingerprinted 1 functions in src/a.rs"),
        "stderr: {}",
        stderr
    );
    assert!(stderr.contains("Fingerprinted 1 functions in src/b.rs"));

    // No changes: nothing is re-fingerprinted.
    let out = ctx(root, &["index", "--verbose"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("Fingerprinted"), "stderr: {}", stderr);

    // Editing one file re-fingerprints only that file's symbols.
    write(
        root,
        "src/b.rs",
        &DUPE_B.replace("sum_invoices", "sum_invoices_v2"),
    );
    let out = ctx(root, &["index", "--verbose"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Fingerprinted 1 functions in src/b.rs"),
        "stderr: {}",
        stderr
    );
    assert!(!stderr.contains("in src/a.rs"), "stderr: {}", stderr);
}

#[test]
fn test_duplicates_json_output_and_exit_codes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "src/a.rs", DUPE_A);
    write(root, "src/b.rs", DUPE_B);
    write(root, "src/c.rs", UNRELATED);
    assert!(ctx(root, &["index"]).status.success());

    // JSON mode: one envelope on stdout, the renamed copy is detected.
    let out = ctx(root, &["duplicates", "--json"]);
    assert_eq!(out.status.code(), Some(0), "informational run exits 0");
    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is a single JSON document");
    assert_eq!(doc["command"], "duplicates");
    assert_eq!(doc["data"]["threshold"], 0.85);
    assert_eq!(doc["data"]["min_tokens"], 50);
    assert_eq!(doc["data"]["skipped_languages"], serde_json::json!([]));

    let pairs = doc["data"]["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 1, "pairs: {}", doc["data"]["pairs"]);
    let names = [
        pairs[0]["a"]["name"].as_str().unwrap(),
        pairs[0]["b"]["name"].as_str().unwrap(),
    ];
    assert!(names.contains(&"process_orders"));
    assert!(names.contains(&"sum_invoices"));
    assert!(pairs[0]["similarity"].as_f64().unwrap() >= 0.85);
    assert!(pairs[0]["token_count_a"].as_i64().unwrap() >= 50);

    // --fail-on-found turns findings into exit code 1.
    let out = ctx(root, &["duplicates", "--fail-on-found"]);
    assert_eq!(out.status.code(), Some(1));

    // Without the flag the same findings exit 0.
    let out = ctx(root, &["duplicates"]);
    assert_eq!(out.status.code(), Some(0));

    // Remove the duplicate; --fail-on-found is Clean again.
    fs::remove_file(root.join("src/b.rs")).unwrap();
    assert!(ctx(root, &["index"]).status.success());
    let out = ctx(root, &["duplicates", "--fail-on-found"]);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn test_duplicates_detects_solidity_near_duplicates() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "contracts/Ledger.sol", SOL_A);
    write(root, "contracts/Register.sol", SOL_B);
    assert!(ctx(root, &["index"]).status.success());

    let out = ctx(root, &["duplicates", "--json"]);
    assert_eq!(out.status.code(), Some(0), "informational run exits 0");
    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is a single JSON document");

    // Solidity is fingerprinted now, so nothing is skipped.
    assert_eq!(doc["data"]["skipped_languages"], serde_json::json!([]));

    // The two structurally identical .sol functions are detected as a pair.
    let pairs = doc["data"]["pairs"].as_array().unwrap();
    let found = pairs.iter().any(|p| {
        let names = [
            p["a"]["name"].as_str().unwrap_or(""),
            p["b"]["name"].as_str().unwrap_or(""),
        ];
        names.contains(&"processOrders") && names.contains(&"sumInvoices")
    });
    assert!(
        found,
        "expected the two .sol functions to pair: {}",
        doc["data"]["pairs"]
    );
}

#[test]
fn test_help_documents_new_semantics_and_old_flags_are_gone() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let out = ctx(root, &["duplicates", "--help"]);
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);

    // New threshold semantics are documented.
    assert!(help.contains("Jaccard"), "help: {}", help);
    assert!(help.contains("0.0-1.0"), "help: {}", help);
    assert!(help.contains("--threshold"));
    assert!(help.contains("--min-tokens"));
    assert!(help.contains("--against"));
    assert!(help.contains("--fail-on-found"));

    // The old line-based flags are fully gone...
    assert!(!help.contains("--similarity"), "help: {}", help);
    assert!(!help.contains("--min-lines"), "help: {}", help);

    // ...and rejected as unknown arguments (clap usage error, exit 2).
    let out = ctx(root, &["duplicates", "--similarity", "80"]);
    assert_eq!(out.status.code(), Some(2));
    let out = ctx(root, &["duplicates", "--min-lines", "5"]);
    assert_eq!(out.status.code(), Some(2));

    // No hidden `dupes` alias: it is not a recognized subcommand (clap
    // treats it as a context file pattern) and never reaches the detector.
    let out = ctx(root, &["help", "dupes"]);
    assert_ne!(out.status.code(), Some(0));
    let out = ctx(root, &["--help"]);
    let top_help = String::from_utf8_lossy(&out.stdout);
    assert!(!top_help.contains("dupes,"), "help: {}", top_help);
    let out = ctx(root, &["dupes"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("Near-duplicate"), "stdout: {}", stdout);
}

#[test]
fn test_threshold_validation_and_clamping() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "src/a.rs", DUPE_A);
    assert!(ctx(root, &["index"]).status.success());

    // Out-of-range threshold is an operational error (exit 2).
    let out = ctx(root, &["duplicates", "--threshold", "1.5"]);
    assert_eq!(out.status.code(), Some(2));

    // Below 0.5 the threshold is clamped with a warning on stderr.
    let out = ctx(root, &["duplicates", "--threshold", "0.2"]);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("clamping to 0.5"), "stderr: {}", stderr);
}
