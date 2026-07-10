//! `ctx score` -- quality scorecard CLI.
//!
//! Thin wrapper around the [`ctx::score`] engine: computes the scorecard for
//! the working tree against a git reference, prints it (human or `--json`),
//! and maps `--fail-on` conditions to the exit outcome.

use std::path::Path;

use ctx::error::Result;
use ctx::exit::Outcome;
use ctx::gatelog;
use ctx::score::{self, FailCondition, Metrics, ScoreReport};

/// Run `ctx score` in the current directory.
pub fn run_score(against: &str, fail_on: Option<&str>, json: bool) -> Result<Outcome> {
    let root = std::env::current_dir()?;
    run_score_in(&root, against, fail_on, json)
}

/// Dir-explicit implementation (used directly by tests).
fn run_score_in(
    root: &Path,
    against: &str,
    fail_on: Option<&str>,
    json_mode: bool,
) -> Result<Outcome> {
    // Parse --fail-on before doing any work so malformed expressions fail
    // fast with an operational error (exit 2).
    let conditions = match fail_on {
        Some(expr) => score::parse_fail_on(expr)?,
        None => Vec::new(),
    };

    let report = score::compute_score(root, against)?;
    eprintln!(
        "note: index refreshed ({} files reindexed)",
        report.files_reindexed
    );

    let failed = score::failed_conditions(&conditions, &report.metrics);

    // Opt-in gate log (CTX_GATE_LOG): append one record per evaluation.
    // Best-effort -- an IO failure warns once on stderr and never changes
    // the command's exit outcome.
    if let Some(log_path) = gatelog::gate_log_target(root) {
        let record = gatelog::GateRecord {
            schema_version: gatelog::GATE_LOG_SCHEMA_VERSION,
            ts: gatelog::now_rfc3339(),
            ctx_version: env!("CARGO_PKG_VERSION").to_string(),
            source: "score".to_string(),
            against: against.to_string(),
            fail_on: fail_on.map(str::to_string),
            metrics: metrics_value(&report.metrics),
            failed_conditions: failed.iter().map(|c| c.to_string()).collect(),
            outcome: if failed.is_empty() { "pass" } else { "fail" }.to_string(),
            blocking: gatelog::blocking_enabled(),
            session_id: gatelog::session_id(),
        };
        if let Err(err) = gatelog::append(&log_path, &record) {
            eprintln!(
                "warning: could not append gate log {}: {}",
                log_path.display(),
                err
            );
        }
    }

    if json_mode {
        ctx::json::emit("score", score_data(&report, &failed))?;
    } else {
        print_human(&report, &failed);
    }

    if failed.is_empty() {
        Ok(Outcome::Clean)
    } else {
        eprintln!("failed conditions:");
        for condition in &failed {
            eprintln!(
                "  {} (actual {})",
                condition,
                report.metrics.get(&condition.metric).unwrap_or(0)
            );
        }
        Ok(Outcome::Findings)
    }
}

// ============================================================================
// Output
// ============================================================================

/// The seven-key `metrics` object shared by the `--json` payload and the
/// gate log (see docs/json-output.md, `score`).
fn metrics_value(m: &Metrics) -> serde_json::Value {
    serde_json::json!({
        "complexity_delta": m.complexity_delta,
        "fan_out_delta": m.fan_out_delta,
        "new_duplication": m.new_duplication,
        "check_violations": m.check_violations,
        "symbols_added": m.symbols_added,
        "symbols_removed": m.symbols_removed,
        "files_changed": m.files_changed,
    })
}

/// The `data` payload for `--json` mode (see docs/json-output.md, `score`).
fn score_data(report: &ScoreReport, failed: &[FailCondition]) -> serde_json::Value {
    let m = &report.metrics;
    let per_file: Vec<serde_json::Value> = report
        .per_file
        .iter()
        .map(|f| {
            serde_json::json!({
                "path": f.path,
                "complexity_baseline": f.complexity_baseline,
                "complexity_current": f.complexity_current,
                "fan_out_baseline": f.fan_out_baseline,
                "fan_out_current": f.fan_out_current,
                "symbols_added": f.symbols_added,
                "symbols_removed": f.symbols_removed,
            })
        })
        .collect();

    serde_json::json!({
        "against": report.against,
        "files_changed": m.files_changed,
        "metrics": metrics_value(m),
        "check_violations_note": report.check_violations_note,
        "per_file": per_file,
        "failed_conditions": failed.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "notes": report.notes,
    })
}

/// The ▲ / ▼ / = marker for a delta.
fn marker(delta: i64) -> &'static str {
    match delta.cmp(&0) {
        std::cmp::Ordering::Greater => "▲",
        std::cmp::Ordering::Less => "▼",
        std::cmp::Ordering::Equal => "=",
    }
}

fn print_human(report: &ScoreReport, failed: &[FailCondition]) {
    let m = &report.metrics;

    println!(
        "Score vs {} ({} file{} changed)",
        report.against,
        m.files_changed,
        if m.files_changed == 1 { "" } else { "s" }
    );
    println!();

    // Delta metrics: baseline -> current plus a signed delta.
    let delta_rows = [
        (
            "complexity_delta",
            report.complexity_baseline,
            report.complexity_current,
            m.complexity_delta,
        ),
        (
            "fan_out_delta",
            report.fan_out_baseline,
            report.fan_out_current,
            m.fan_out_delta,
        ),
    ];
    for (name, baseline, current, delta) in delta_rows {
        println!(
            "  {:<18} {:>6} → {:<6} {} {:+}",
            name,
            baseline,
            current,
            marker(delta),
            delta
        );
    }

    // Count metrics: a single value; anything above zero is an increase.
    let count_rows = [
        ("new_duplication", m.new_duplication, None),
        (
            "check_violations",
            m.check_violations,
            report.check_violations_note.as_deref(),
        ),
        ("symbols_added", m.symbols_added, None),
        ("symbols_removed", m.symbols_removed, None),
    ];
    for (name, value, note) in count_rows {
        let note = note.map(|n| format!("  ({})", n)).unwrap_or_default();
        println!(
            "  {:<18} {:>6}          {}{}",
            name,
            value,
            marker(value),
            note
        );
    }

    if !report.notes.is_empty() {
        println!();
        println!("Notes:");
        for note in &report.notes {
            println!("  - {}", note);
        }
    }

    if !failed.is_empty() {
        println!();
        println!("Failed conditions (exit 1):");
        for condition in failed {
            println!(
                "  - {} (actual {})",
                condition,
                report.metrics.get(&condition.metric).unwrap_or(0)
            );
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ctx::index::Indexer;
    use ctx::testutil::GitRepo;
    use ctx::walker::WalkerConfig;
    use tempfile::TempDir;

    const BIG_FN: &str = r#"
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

    const BIG_FN_COPY: &str = r#"
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

    fn index_once(root: &std::path::Path) {
        let mut indexer = Indexer::with_config(root, false, WalkerConfig::default()).unwrap();
        indexer.index().unwrap();
    }

    #[test]
    fn test_run_score_fail_on_outcome_mapping() {
        let temp = TempDir::new().unwrap();
        let repo = GitRepo::init(temp.path());
        repo.write("src/a.rs", BIG_FN);
        repo.write("src/b.rs", "pub fn tiny() -> i64 { 1 }\n");
        repo.commit_all("v1");
        index_once(&repo.root);

        // Introduce duplication: the gate trips (exit 1).
        repo.write(
            "src/b.rs",
            &format!("pub fn tiny() -> i64 {{ 1 }}\n{}", BIG_FN_COPY),
        );
        let outcome = run_score_in(&repo.root, "HEAD", Some("new_duplication>0"), false).unwrap();
        assert_eq!(outcome, Outcome::Findings);

        // Same run without --fail-on is informational (exit 0).
        let outcome = run_score_in(&repo.root, "HEAD", None, false).unwrap();
        assert_eq!(outcome, Outcome::Clean);

        // Deduplicate: the gate passes again.
        repo.write("src/b.rs", "pub fn tiny() -> i64 { 1 }\n");
        let outcome = run_score_in(&repo.root, "HEAD", Some("new_duplication>0"), false).unwrap();
        assert_eq!(outcome, Outcome::Clean);

        // Malformed --fail-on is an operational error before any work.
        let err = run_score_in(&repo.root, "HEAD", Some("nope>1"), false).unwrap_err();
        assert!(err.to_string().contains("nope"), "err: {}", err);
    }

    #[test]
    fn test_score_json_payload_shape() {
        let temp = TempDir::new().unwrap();
        let repo = GitRepo::init(temp.path());
        repo.commit_file("src/a.rs", "pub fn a() -> i64 { 1 }\n", "v1");
        index_once(&repo.root);

        repo.write(
            "src/a.rs",
            "pub fn a() -> i64 { b() }\npub fn b() -> i64 { 2 }\n",
        );
        let report = ctx::score::compute_score(&repo.root, "HEAD").unwrap();
        let conditions = ctx::score::parse_fail_on("symbols_added>0").unwrap();
        let failed = ctx::score::failed_conditions(&conditions, &report.metrics);
        let data = score_data(&report, &failed);

        assert_eq!(data["against"], "HEAD");
        assert_eq!(data["files_changed"], 1);

        // Flat metrics object with exactly the seven documented keys.
        let metrics = data["metrics"].as_object().unwrap();
        let mut keys: Vec<&str> = metrics.keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        let mut expected = ctx::score::Metrics::NAMES.to_vec();
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert!(metrics["symbols_added"].as_i64().unwrap() >= 1);
        assert!(metrics["fan_out_delta"].as_i64().unwrap() >= 1);

        // Per-file entries carry both sides.
        let per_file = data["per_file"].as_array().unwrap();
        assert_eq!(per_file.len(), 1);
        let entry = per_file[0].as_object().unwrap();
        for key in [
            "path",
            "complexity_baseline",
            "complexity_current",
            "fan_out_baseline",
            "fan_out_current",
            "symbols_added",
            "symbols_removed",
        ] {
            assert!(entry.contains_key(key), "missing per_file key {}", key);
        }
        assert_eq!(entry["path"], "src/a.rs");

        // Failed conditions and notes are arrays of strings.
        assert_eq!(data["failed_conditions"][0], "symbols_added > 0");
        assert!(data["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == ctx::score::FAN_IN_NOTE));
        // No rules file in this fixture.
        assert_eq!(data["check_violations_note"], ctx::score::NO_RULES_NOTE);
        assert_eq!(data["metrics"]["check_violations"], 0);

        // The full envelope wraps the payload under "data".
        let envelope = ctx::json::envelope("score", data);
        assert_eq!(envelope["command"], "score");
        assert!(envelope["data"]["metrics"].is_object());
    }

    #[test]
    fn test_marker() {
        assert_eq!(marker(3), "▲");
        assert_eq!(marker(-2), "▼");
        assert_eq!(marker(0), "=");
    }
}
