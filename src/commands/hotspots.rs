//! `ctx hotspots` — rank files (or symbols) by combined git churn and complexity.
//!
//! A hotspot is code that is both structurally complex and frequently changed:
//! the intersection where refactoring effort pays off most. The score is
//! `normalized_churn * normalized_complexity`, min-max normalized to `[0, 1]`
//! over the analyzed set (indexed files with churn >= `--min-churn`).
//!
//! This is an informational command: it always exits 0 on success and 2 on
//! operational errors (not a git repo, missing index, bad `--against` ref).
//!
//! Known v1 approximations:
//! - With `--by symbol`, a symbol's churn is approximated by its file's
//!   commit count (per-symbol git history is not tracked yet).
//! - Churn is collected with `git log --no-renames`, so renaming a file
//!   resets its churn count.

use std::collections::{HashMap, HashSet};
use std::env;

use clap::ValueEnum;

use ctx::db::{FileComplexity, SymbolMetrics};
use ctx::error::Result;
use ctx::gitutil;
use ctx::index;
use ctx::json::{self, SymbolRef};
use ctx::utils::{truncate_path, truncate_str};

/// What each ranked row represents.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HotspotBy {
    /// One row per indexed file.
    #[default]
    File,
    /// One row per function/method (churn approximated by the file's churn).
    Symbol,
}

/// A ranked per-file hotspot.
#[derive(Debug, Clone)]
pub struct HotspotEntry {
    pub file: String,
    pub commits: u32,
    pub complexity: i64,
    pub fan_out: i64,
    pub score: f64,
}

/// A ranked per-symbol hotspot (churn approximated by the file's churn).
#[derive(Debug, Clone)]
pub struct SymbolHotspotEntry {
    pub metrics: SymbolMetrics,
    pub commits: u32,
    pub score: f64,
}

/// Min-max normalize `value` into `[0, 1]` over `[min, max]`.
///
/// Degenerate case: when `max == min` every value normalizes to `1.0`.
fn normalize(value: f64, min: f64, max: f64) -> f64 {
    if max <= min {
        1.0
    } else {
        (value - min) / (max - min)
    }
}

/// Min and max of an iterator of f64 values (assumed non-empty and finite).
fn min_max(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v), hi.max(v))
    })
}

/// Score and rank per-file hotspots.
///
/// The analyzed set is the intersection of:
/// - files present in the index (i.e. in `complexity`; churn-only files that
///   are not indexed are never reported),
/// - files with churn >= `min_churn`,
/// - `restrict_to` when given (e.g. files changed against a ref).
///
/// `score = normalized_churn * normalized_complexity`, min-max normalized
/// over the analyzed set (if `max == min`, all values normalize to `1.0`).
///
/// Ordering is deterministic: score desc, raw churn desc, complexity desc,
/// then file path asc. The result is truncated to `limit` entries.
pub fn score_hotspots(
    churn: &HashMap<String, u32>,
    complexity: &[FileComplexity],
    min_churn: u32,
    limit: usize,
    restrict_to: Option<&HashSet<String>>,
) -> Vec<HotspotEntry> {
    let candidates: Vec<(&FileComplexity, u32)> = complexity
        .iter()
        .filter(|fc| restrict_to.is_none_or(|set| set.contains(&fc.file_path)))
        .map(|fc| (fc, churn.get(&fc.file_path).copied().unwrap_or(0)))
        .filter(|(_, commits)| *commits >= min_churn)
        .collect();

    if candidates.is_empty() {
        return Vec::new();
    }

    let (churn_min, churn_max) = min_max(candidates.iter().map(|(_, c)| *c as f64));
    let (cx_min, cx_max) = min_max(candidates.iter().map(|(fc, _)| fc.complexity as f64));

    let mut entries: Vec<HotspotEntry> = candidates
        .into_iter()
        .map(|(fc, commits)| HotspotEntry {
            file: fc.file_path.clone(),
            commits,
            complexity: fc.complexity,
            fan_out: fc.fan_out,
            score: normalize(commits as f64, churn_min, churn_max)
                * normalize(fc.complexity as f64, cx_min, cx_max),
        })
        .collect();

    entries.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.commits.cmp(&a.commits))
            .then(b.complexity.cmp(&a.complexity))
            .then(a.file.cmp(&b.file))
    });
    entries.truncate(limit);
    entries
}

/// Score and rank per-symbol hotspots.
///
/// Same scoring model as [`score_hotspots`], but each row is a function or
/// method and its churn is approximated by its file's commit count (v1
/// limitation: per-symbol history is not tracked).
///
/// Ordering: score desc, raw churn desc, complexity desc, file path asc,
/// then symbol id asc as the final tiebreak.
pub fn score_symbol_hotspots(
    churn: &HashMap<String, u32>,
    metrics: &[SymbolMetrics],
    min_churn: u32,
    limit: usize,
    restrict_to: Option<&HashSet<String>>,
) -> Vec<SymbolHotspotEntry> {
    let candidates: Vec<(&SymbolMetrics, u32)> = metrics
        .iter()
        .filter(|m| restrict_to.is_none_or(|set| set.contains(&m.file_path)))
        .map(|m| (m, churn.get(&m.file_path).copied().unwrap_or(0)))
        .filter(|(_, commits)| *commits >= min_churn)
        .collect();

    if candidates.is_empty() {
        return Vec::new();
    }

    let (churn_min, churn_max) = min_max(candidates.iter().map(|(_, c)| *c as f64));
    let (cx_min, cx_max) = min_max(candidates.iter().map(|(m, _)| m.complexity as f64));

    let mut entries: Vec<SymbolHotspotEntry> = candidates
        .into_iter()
        .map(|(m, commits)| SymbolHotspotEntry {
            metrics: m.clone(),
            commits,
            score: normalize(commits as f64, churn_min, churn_max)
                * normalize(m.complexity as f64, cx_min, cx_max),
        })
        .collect();

    entries.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.commits.cmp(&a.commits))
            .then(b.metrics.complexity.cmp(&a.metrics.complexity))
            .then(a.metrics.file_path.cmp(&b.metrics.file_path))
            .then(a.metrics.id.cmp(&b.metrics.id))
    });
    entries.truncate(limit);
    entries
}

/// The most complex symbols in `file` (up to `limit`), ties broken by name.
fn top_symbols_for_file<'a>(
    metrics: &'a [SymbolMetrics],
    file: &str,
    limit: usize,
) -> Vec<&'a SymbolMetrics> {
    let mut in_file: Vec<&SymbolMetrics> = metrics.iter().filter(|m| m.file_path == file).collect();
    in_file.sort_by(|a, b| b.complexity.cmp(&a.complexity).then(a.name.cmp(&b.name)));
    in_file.truncate(limit);
    in_file
}

/// Build a [`SymbolRef`] from symbol metrics.
fn symbol_ref(m: &SymbolMetrics) -> SymbolRef {
    SymbolRef {
        name: m.name.clone(),
        qualified_name: m.qualified_name.clone(),
        kind: m.kind.clone(),
        file: m.file_path.clone(),
        line_start: m.line_start,
        line_end: m.line_end,
    }
}

/// Build the `--json` payload for `--by file` output.
fn file_payload(
    since: &str,
    min_churn: u32,
    against: Option<&str>,
    entries: &[HotspotEntry],
    metrics: &[SymbolMetrics],
) -> serde_json::Value {
    let json_entries: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            let symbols: Vec<serde_json::Value> = top_symbols_for_file(metrics, &e.file, 3)
                .into_iter()
                .map(|m| {
                    serde_json::json!({
                        "symbol": symbol_ref(m).to_value(),
                        "complexity": m.complexity,
                    })
                })
                .collect();
            serde_json::json!({
                "file": e.file,
                "commits": e.commits,
                "complexity": e.complexity,
                "fan_out": e.fan_out,
                "score": e.score,
                "symbols": symbols,
            })
        })
        .collect();

    serde_json::json!({
        "since": since,
        "min_churn": min_churn,
        "by": "file",
        "against": against,
        "entries": json_entries,
    })
}

/// Build the `--json` payload for `--by symbol` output.
fn symbol_payload(
    since: &str,
    min_churn: u32,
    against: Option<&str>,
    entries: &[SymbolHotspotEntry],
) -> serde_json::Value {
    let json_entries: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "symbol": symbol_ref(&e.metrics).to_value(),
                "file": e.metrics.file_path,
                "commits": e.commits,
                "complexity": e.metrics.complexity,
                "fan_out": e.metrics.fan_out,
                "score": e.score,
            })
        })
        .collect();

    serde_json::json!({
        "since": since,
        "min_churn": min_churn,
        "by": "symbol",
        "against": against,
        "entries": json_entries,
    })
}

/// Run `ctx hotspots`: rank files (or symbols) by churn x complexity.
///
/// Informational command: always returns `Ok(())` (exit 0) on success;
/// operational errors (not a git repo, missing index, bad ref) map to exit 2.
pub fn run_hotspots(
    since: &str,
    limit: usize,
    by: HotspotBy,
    min_churn: u32,
    against: Option<&str>,
    json: bool,
) -> Result<()> {
    if !gitutil::is_git_repo() {
        return Err(ctx::error::CtxError::NotGitRepo);
    }

    let churn = gitutil::churn_since(since)?;
    let restrict = against.map(gitutil::changed_files_against).transpose()?;

    let root = env::current_dir()?;
    let db = index::open_database(&root)?;
    let metrics = db.symbol_metrics()?;

    match by {
        HotspotBy::File => {
            let complexity = db.file_complexity()?;
            let entries = score_hotspots(&churn, &complexity, min_churn, limit, restrict.as_ref());
            if json {
                json::emit(
                    "hotspots",
                    file_payload(since, min_churn, against, &entries, &metrics),
                )?;
            } else {
                print_file_table(since, min_churn, &entries);
            }
        }
        HotspotBy::Symbol => {
            let entries =
                score_symbol_hotspots(&churn, &metrics, min_churn, limit, restrict.as_ref());
            if json {
                json::emit(
                    "hotspots",
                    symbol_payload(since, min_churn, against, &entries),
                )?;
            } else {
                print_symbol_table(since, min_churn, &entries);
            }
        }
    }

    Ok(())
}

fn print_file_table(since: &str, min_churn: u32, entries: &[HotspotEntry]) {
    println!(
        "Code Hotspots (since \"{}\", by file, min churn {})",
        since, min_churn
    );
    if entries.is_empty() {
        println!("No hotspots found (try lowering --min-churn or widening --since).");
        return;
    }

    println!("{}", "=".repeat(92));
    println!(
        "{:>4}  {:<45} {:>8} {:>11} {:>8} {:>6}",
        "RANK", "FILE", "COMMITS", "COMPLEXITY", "FAN-OUT", "SCORE"
    );
    println!("{}", "-".repeat(92));
    for (i, e) in entries.iter().enumerate() {
        println!(
            "{:>4}  {:<45} {:>8} {:>11} {:>8} {:>6.2}",
            i + 1,
            truncate_path(&e.file, 45),
            e.commits,
            e.complexity,
            e.fan_out,
            e.score
        );
    }
}

fn print_symbol_table(since: &str, min_churn: u32, entries: &[SymbolHotspotEntry]) {
    println!(
        "Code Hotspots (since \"{}\", by symbol, min churn {})",
        since, min_churn
    );
    println!("Note: symbol churn is approximated by its file's commit count.");
    if entries.is_empty() {
        println!("No hotspots found (try lowering --min-churn or widening --since).");
        return;
    }

    println!("{}", "=".repeat(108));
    println!(
        "{:>4}  {:<30} {:<30} {:>8} {:>11} {:>8} {:>6}",
        "RANK", "SYMBOL", "FILE", "COMMITS", "COMPLEXITY", "FAN-OUT", "SCORE"
    );
    println!("{}", "-".repeat(108));
    for (i, e) in entries.iter().enumerate() {
        println!(
            "{:>4}  {:<30} {:<30} {:>8} {:>11} {:>8} {:>6.2}",
            i + 1,
            truncate_str(&e.metrics.name, 30),
            truncate_path(&e.metrics.file_path, 30),
            e.commits,
            e.metrics.complexity,
            e.metrics.fan_out,
            e.score
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fc(file: &str, complexity: i64, fan_out: i64) -> FileComplexity {
        FileComplexity {
            file_path: file.to_string(),
            complexity,
            fan_out,
            symbol_count: 1,
        }
    }

    fn sm(id: &str, name: &str, file: &str, complexity: i64) -> SymbolMetrics {
        SymbolMetrics {
            id: id.to_string(),
            name: name.to_string(),
            qualified_name: None,
            kind: "function".to_string(),
            file_path: file.to_string(),
            line_start: 1,
            line_end: 2,
            fan_in: 0,
            fan_out: 0,
            complexity,
        }
    }

    fn churn(pairs: &[(&str, u32)]) -> HashMap<String, u32> {
        pairs.iter().map(|(f, c)| (f.to_string(), *c)).collect()
    }

    #[test]
    fn test_both_high_ranks_above_single_dimension() {
        // hot.rs is both complex and churned; the others excel on one axis only.
        let churn = churn(&[("hot.rs", 10), ("churny.rs", 10), ("complex.rs", 2)]);
        let complexity = vec![
            fc("hot.rs", 100, 5),
            fc("churny.rs", 5, 1),
            fc("complex.rs", 100, 5),
        ];

        let entries = score_hotspots(&churn, &complexity, 1, 20, None);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].file, "hot.rs");
        assert!(entries[0].score > entries[1].score);
        assert!(entries[0].score > entries[2].score);
        // Raw values are reported alongside the score.
        assert_eq!(entries[0].commits, 10);
        assert_eq!(entries[0].complexity, 100);
        assert_eq!(entries[0].fan_out, 5);
    }

    #[test]
    fn test_min_churn_filters_before_normalization() {
        let churn = churn(&[("a.rs", 10), ("b.rs", 1)]);
        let complexity = vec![fc("a.rs", 10, 1), fc("b.rs", 100, 9)];

        let entries = score_hotspots(&churn, &complexity, 2, 20, None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file, "a.rs");
        // a.rs is alone in the analyzed set -> degenerate normalization -> 1.0.
        assert_eq!(entries[0].score, 1.0);
    }

    #[test]
    fn test_files_not_in_index_are_not_reported() {
        // ghost.rs has churn but no complexity row (not in the index).
        let churn = churn(&[("a.rs", 5), ("ghost.rs", 50)]);
        let complexity = vec![fc("a.rs", 10, 1)];

        let entries = score_hotspots(&churn, &complexity, 1, 20, None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file, "a.rs");
    }

    #[test]
    fn test_max_equals_min_normalizes_to_one() {
        // Same churn and same complexity everywhere -> all scores 1.0.
        let churn = churn(&[("a.rs", 3), ("b.rs", 3)]);
        let complexity = vec![fc("a.rs", 7, 1), fc("b.rs", 7, 2)];

        let entries = score_hotspots(&churn, &complexity, 1, 20, None);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.score == 1.0));
        // Equal score/churn/complexity -> path asc.
        assert_eq!(entries[0].file, "a.rs");
        assert_eq!(entries[1].file, "b.rs");
    }

    #[test]
    fn test_deterministic_tie_breaking() {
        // All min-churn files score 0 on one normalized axis -> equal scores,
        // broken by raw churn desc, then complexity desc, then path asc.
        let churn = churn(&[
            ("top.rs", 10),
            ("b.rs", 4),
            ("a.rs", 2),
            ("z.rs", 2),
            ("m.rs", 2),
        ]);
        let complexity = vec![
            fc("top.rs", 100, 1),
            fc("b.rs", 0, 0),  // complexity 0 -> score 0, churn 4
            fc("a.rs", 0, 0),  // score 0, churn 2, complexity 0
            fc("z.rs", 0, 0),  // score 0, churn 2, complexity 0 -> after a.rs
            fc("m.rs", 50, 1), // churn 2 (min) -> normalized churn 0 -> score 0, complexity 50
        ];

        let entries = score_hotspots(&churn, &complexity, 1, 20, None);
        let order: Vec<&str> = entries.iter().map(|e| e.file.as_str()).collect();
        assert_eq!(order, vec!["top.rs", "b.rs", "m.rs", "a.rs", "z.rs"]);
    }

    #[test]
    fn test_limit_truncates() {
        let churn = churn(&[("a.rs", 1), ("b.rs", 2), ("c.rs", 3)]);
        let complexity = vec![fc("a.rs", 1, 0), fc("b.rs", 2, 0), fc("c.rs", 3, 0)];

        let entries = score_hotspots(&churn, &complexity, 1, 2, None);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].file, "c.rs");
    }

    #[test]
    fn test_restrict_to_intersects_analyzed_set() {
        let churn = churn(&[("a.rs", 10), ("b.rs", 10)]);
        let complexity = vec![fc("a.rs", 10, 1), fc("b.rs", 100, 9)];
        let only_a: HashSet<String> = ["a.rs".to_string()].into_iter().collect();

        let entries = score_hotspots(&churn, &complexity, 1, 20, Some(&only_a));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file, "a.rs");
        // Restriction happens before normalization: a.rs alone -> score 1.0.
        assert_eq!(entries[0].score, 1.0);
    }

    #[test]
    fn test_empty_inputs_produce_no_entries() {
        let entries = score_hotspots(&HashMap::new(), &[], 1, 20, None);
        assert!(entries.is_empty());

        // Churn exists but nothing is indexed.
        let entries = score_hotspots(&churn(&[("a.rs", 5)]), &[], 0, 20, None);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_symbol_scoring_uses_file_churn_and_id_tiebreak() {
        let churn = churn(&[("a.rs", 5), ("b.rs", 1)]);
        let metrics = vec![
            sm("a.rs::beta", "beta", "a.rs", 10),
            sm("a.rs::alpha", "alpha", "a.rs", 10),
            sm("b.rs::gamma", "gamma", "b.rs", 100),
        ];

        let entries = score_symbol_hotspots(&churn, &metrics, 1, 20, None);
        assert_eq!(entries.len(), 3);
        // Both a.rs symbols inherit the file churn of 5.
        assert!(entries
            .iter()
            .filter(|e| e.metrics.file_path == "a.rs")
            .all(|e| e.commits == 5));
        // Equal score/churn/complexity/path for the two a.rs symbols -> id asc.
        let a_ids: Vec<&str> = entries
            .iter()
            .filter(|e| e.metrics.file_path == "a.rs")
            .map(|e| e.metrics.id.as_str())
            .collect();
        assert_eq!(a_ids, vec!["a.rs::alpha", "a.rs::beta"]);
    }

    #[test]
    fn test_symbol_min_churn_filters_by_file_churn() {
        let churn = churn(&[("a.rs", 5), ("b.rs", 1)]);
        let metrics = vec![
            sm("a.rs::f", "f", "a.rs", 10),
            sm("b.rs::g", "g", "b.rs", 100),
        ];

        let entries = score_symbol_hotspots(&churn, &metrics, 2, 20, None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].metrics.id, "a.rs::f");
    }

    #[test]
    fn test_top_symbols_caps_at_limit_with_name_tiebreak() {
        let metrics = vec![
            sm("f::d", "d", "f.rs", 5),
            sm("f::c", "c", "f.rs", 10),
            sm("f::b", "b", "f.rs", 10),
            sm("f::a", "a", "f.rs", 1),
            sm("g::x", "x", "g.rs", 99),
        ];

        let top = top_symbols_for_file(&metrics, "f.rs", 3);
        let names: Vec<&str> = top.iter().map(|m| m.name.as_str()).collect();
        // complexity desc, ties by name asc, capped at 3, other files excluded.
        assert_eq!(names, vec!["b", "c", "d"]);
    }

    #[test]
    fn test_file_payload_shape() {
        let entries = vec![HotspotEntry {
            file: "a.rs".to_string(),
            commits: 4,
            complexity: 12,
            fan_out: 3,
            score: 1.0,
        }];
        let metrics = vec![
            sm("a.rs::d", "d", "a.rs", 5),
            sm("a.rs::c", "c", "a.rs", 10),
            sm("a.rs::b", "b", "a.rs", 10),
            sm("a.rs::a", "a", "a.rs", 1),
        ];

        let payload = file_payload("6 months ago", 2, None, &entries, &metrics);
        assert_eq!(payload["since"], "6 months ago");
        assert_eq!(payload["min_churn"], 2);
        assert_eq!(payload["by"], "file");
        assert!(payload["against"].is_null());

        let entry = &payload["entries"][0];
        let keys: Vec<&str> = entry
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
        // Top 3 most complex symbols at most.
        let symbols = entry["symbols"].as_array().unwrap();
        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0]["symbol"]["name"], "b");
        assert_eq!(symbols[0]["complexity"], 10);
        // SymbolRef shape (snake_case).
        assert!(symbols[0]["symbol"]["line_start"].is_number());
    }

    #[test]
    fn test_symbol_payload_shape() {
        let entries = vec![SymbolHotspotEntry {
            metrics: sm("a.rs::f", "f", "a.rs", 10),
            commits: 4,
            score: 0.5,
        }];

        let payload = symbol_payload("1 week ago", 1, Some("main"), &entries);
        assert_eq!(payload["by"], "symbol");
        assert_eq!(payload["against"], "main");

        let entry = &payload["entries"][0];
        let keys: Vec<&str> = entry
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
        assert_eq!(entry["symbol"]["name"], "f");
        assert_eq!(entry["symbol"]["kind"], "function");
    }
}
