//! `ctx score` engine: a quality scorecard for the changes between a git
//! reference and the current working tree.
//!
//! The pipeline is:
//!
//! 1. Refresh the index **incrementally** (never a full reindex).
//! 2. Collect the changed-file set relative to the reference.
//! 3. For every changed file, compute the same metrics on two sides:
//!    - **current**: from the index, restricted to the file (per-file
//!      queries only -- no global scans).
//!    - **baseline**: by parsing the file's content at the reference
//!      **in memory** (no database writes).
//! 4. Diff the two sides into deltas, detect newly introduced duplication,
//!    and run the architecture-rules check.
//!
//! ## Approximation
//!
//! Per-function complexity is `2 * fan_out + same_file_fan_in`. The baseline
//! side is parsed in isolation, so cross-file callers are unknowable there;
//! fan-in is therefore approximated as *same-file* callers on **both** sides
//! so the delta compares like with like. This is surfaced as a note in both
//! human and JSON output.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

use crate::check;
use crate::db::{Database, ParseResult, Symbol, SymbolKind};
use crate::error::{CtxError, Result};
use crate::fingerprint;
use crate::gitutil;
use crate::index::Indexer;
use crate::parser::{CodeParser, Language};
use crate::rules;
use crate::walker::WalkerConfig;

/// Jaccard threshold used for the `new_duplication` metric.
pub const DUP_THRESHOLD: f64 = 0.85;

/// Minimum normalized token count for the `new_duplication` metric.
pub const DUP_MIN_TOKENS: i64 = 50;

/// Note attached whenever complexity deltas are reported.
pub const FAN_IN_NOTE: &str = "fan_in approximated as same-file callers for baseline comparability";

/// Note attached when `.ctx/rules.toml` does not exist.
pub const NO_RULES_NOTE: &str = "no rules file";

// ============================================================================
// Metrics
// ============================================================================

/// The flat metric set of a score run. Field names are the exact metric
/// names accepted by `--fail-on` and emitted under `data.metrics` in JSON.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Metrics {
    pub complexity_delta: i64,
    pub fan_out_delta: i64,
    pub new_duplication: i64,
    pub check_violations: i64,
    pub symbols_added: i64,
    pub symbols_removed: i64,
    pub files_changed: i64,
}

impl Metrics {
    /// All valid metric names, in scorecard order.
    pub const NAMES: [&'static str; 7] = [
        "complexity_delta",
        "fan_out_delta",
        "new_duplication",
        "check_violations",
        "symbols_added",
        "symbols_removed",
        "files_changed",
    ];

    /// Look up a metric by its `--fail-on` / JSON name.
    pub fn get(&self, name: &str) -> Option<i64> {
        match name {
            "complexity_delta" => Some(self.complexity_delta),
            "fan_out_delta" => Some(self.fan_out_delta),
            "new_duplication" => Some(self.new_duplication),
            "check_violations" => Some(self.check_violations),
            "symbols_added" => Some(self.symbols_added),
            "symbols_removed" => Some(self.symbols_removed),
            "files_changed" => Some(self.files_changed),
            _ => None,
        }
    }
}

/// Per-changed-file breakdown (both sides of the delta metrics).
#[derive(Debug, Clone)]
pub struct FileScore {
    pub path: String,
    pub complexity_baseline: i64,
    pub complexity_current: i64,
    pub fan_out_baseline: i64,
    pub fan_out_current: i64,
    pub symbols_added: i64,
    pub symbols_removed: i64,
}

/// The complete result of a score run.
#[derive(Debug)]
pub struct ScoreReport {
    /// The git reference the working tree was compared against.
    pub against: String,
    /// How many files the internal incremental index refresh reindexed.
    pub files_reindexed: usize,
    /// Summed baseline complexity over changed files.
    pub complexity_baseline: i64,
    /// Summed current complexity over changed files.
    pub complexity_current: i64,
    /// Summed baseline fan-out over changed files.
    pub fan_out_baseline: i64,
    /// Summed current fan-out over changed files.
    pub fan_out_current: i64,
    pub metrics: Metrics,
    pub per_file: Vec<FileScore>,
    /// Set when `check_violations` is 0 because no rules file exists.
    pub check_violations_note: Option<String>,
    /// Caveats surfaced in both human and JSON output.
    pub notes: Vec<String>,
}

// ============================================================================
// Engine
// ============================================================================

/// Compute the score of the working tree (plus commits since the merge base)
/// against `reference`.
///
/// `root` is the project root (also the directory git commands run in).
pub fn compute_score(root: &Path, reference: &str) -> Result<ScoreReport> {
    if !gitutil::is_git_repo_in(root) {
        return Err(CtxError::NotGitRepo);
    }

    // Incremental index refresh: only changed files are re-parsed; the
    // existing database is never cleared.
    let mut indexer = Indexer::with_config(root, false, WalkerConfig::default())?;
    let index_result = indexer.index()?;
    let db = indexer.db;

    let indexed: HashSet<String> = db.get_indexed_files()?.into_iter().collect();

    // Changed files, filtered to supported source files that are either
    // indexed (exist) or gone from disk (deleted). Supported files that
    // exist but are excluded from the index (ignore patterns) are skipped.
    let mut changed: Vec<String> = gitutil::changed_files_against_in(root, reference)?
        .into_iter()
        .filter(|f| CodeParser::is_supported_static(Path::new(f)))
        .filter(|f| indexed.contains(f) || !root.join(f).exists())
        .collect();
    changed.sort();
    let changed_set: HashSet<String> = changed.iter().cloned().collect();

    // Per-file two-sided metrics.
    let mut parser = CodeParser::new();
    let mut per_file = Vec::with_capacity(changed.len());
    for path in &changed {
        per_file.push(score_file(
            root,
            &db,
            &mut parser,
            reference,
            path,
            &indexed,
        )?);
    }

    // Newly introduced near-duplicate pairs.
    let new_duplication = new_duplication(root, &db, &mut parser, reference, &changed_set)?;

    // Architecture rules, scoped to the same reference.
    let rules_path = root.join(rules::DEFAULT_RULES_PATH);
    let (check_violations, check_violations_note) = if rules_path.exists() {
        let context = check::load_context(root, None)?;
        let violations = check::collect_violations(root, &context, Some(reference))?;
        (violations.len() as i64, None)
    } else {
        (0, Some(NO_RULES_NOTE.to_string()))
    };

    // Aggregate.
    let sum = |f: fn(&FileScore) -> i64| per_file.iter().map(f).sum::<i64>();
    let complexity_baseline = sum(|f| f.complexity_baseline);
    let complexity_current = sum(|f| f.complexity_current);
    let fan_out_baseline = sum(|f| f.fan_out_baseline);
    let fan_out_current = sum(|f| f.fan_out_current);

    let metrics = Metrics {
        complexity_delta: complexity_current - complexity_baseline,
        fan_out_delta: fan_out_current - fan_out_baseline,
        new_duplication,
        check_violations,
        symbols_added: sum(|f| f.symbols_added),
        symbols_removed: sum(|f| f.symbols_removed),
        files_changed: changed.len() as i64,
    };

    let mut notes = vec![FAN_IN_NOTE.to_string()];
    if let Some(ref note) = check_violations_note {
        notes.push(format!(
            "check_violations: {} ({})",
            note,
            rules::DEFAULT_RULES_PATH
        ));
    }

    Ok(ScoreReport {
        against: reference.to_string(),
        files_reindexed: index_result.files_indexed,
        complexity_baseline,
        complexity_current,
        fan_out_baseline,
        fan_out_current,
        metrics,
        per_file,
        check_violations_note,
        notes,
    })
}

/// One side (baseline or current) of a changed file's metrics.
#[derive(Debug, Default)]
struct Side {
    complexity: i64,
    fan_out: i64,
    /// `(parent_name, symbol_name)` keys of every symbol in the file.
    keys: HashSet<(Option<String>, String)>,
}

/// Compute both sides of one changed file.
fn score_file(
    root: &Path,
    db: &Database,
    parser: &mut CodeParser,
    reference: &str,
    path: &str,
    indexed: &HashSet<String>,
) -> Result<FileScore> {
    // Current side: from the index, restricted to this file. A file that is
    // no longer indexed (deleted) contributes an empty side.
    let current = if indexed.contains(path) {
        let symbols = db.get_file_symbols(path)?;
        let call_edges = db.file_call_edges(path)?;
        side_metrics(&symbols, &call_edges)
    } else {
        Side::default()
    };

    // Baseline side: parse the content at the reference in memory. A file
    // missing at the reference (added) contributes an empty side.
    let baseline = match gitutil::show_file_in(root, reference, path)? {
        Some(source) => match parser.parse(Path::new(path), &source) {
            Some(parse) => parse_side_metrics(&parse),
            None => Side::default(),
        },
        None => Side::default(),
    };

    Ok(FileScore {
        path: path.to_string(),
        complexity_baseline: baseline.complexity,
        complexity_current: current.complexity,
        fan_out_baseline: baseline.fan_out,
        fan_out_current: current.fan_out,
        symbols_added: current.keys.difference(&baseline.keys).count() as i64,
        symbols_removed: baseline.keys.difference(&current.keys).count() as i64,
    })
}

/// Last `::` segment of a symbol's parent id (the parent's simple name).
fn parent_name(parent_id: Option<&str>) -> Option<String> {
    parent_id.map(|p| p.rsplit("::").next().unwrap_or(p).to_string())
}

/// The `(parent_name, name)` key used to match symbols across sides.
/// Never match by symbol id: ids embed line numbers, which shift.
fn symbol_key(symbol: &Symbol) -> (Option<String>, String) {
    (
        parent_name(symbol.parent_id.as_deref()),
        symbol.name.clone(),
    )
}

/// Metrics for one side, from a symbol list plus the `calls` edges sourced
/// in the file as `(source_symbol_id, target_name)` pairs. Used identically
/// for both sides so deltas are honest.
fn side_metrics(symbols: &[Symbol], call_edges: &[(String, String)]) -> Side {
    let mut fan_out_by_source: HashMap<&str, i64> = HashMap::new();
    let mut calls_by_target_name: HashMap<&str, i64> = HashMap::new();
    for (source_id, target_name) in call_edges {
        *fan_out_by_source.entry(source_id.as_str()).or_insert(0) += 1;
        *calls_by_target_name
            .entry(target_name.as_str())
            .or_insert(0) += 1;
    }

    let mut side = Side {
        fan_out: call_edges.len() as i64,
        ..Side::default()
    };
    for symbol in symbols {
        side.keys.insert(symbol_key(symbol));
        if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
            continue;
        }
        let fan_out = fan_out_by_source
            .get(symbol.id.as_str())
            .copied()
            .unwrap_or(0);
        let same_file_fan_in = calls_by_target_name
            .get(symbol.name.as_str())
            .copied()
            .unwrap_or(0);
        side.complexity += 2 * fan_out + same_file_fan_in;
    }
    side
}

/// [`side_metrics`] over an in-memory parse result (baseline side).
fn parse_side_metrics(parse: &ParseResult) -> Side {
    let call_edges: Vec<(String, String)> = parse
        .edges
        .iter()
        .filter(|e| matches!(e.kind, crate::db::EdgeKind::Calls))
        .map(|e| (e.source_id.clone(), e.target_name.clone()))
        .collect();
    side_metrics(&parse.symbols, &call_edges)
}

// ============================================================================
// New duplication
// ============================================================================

/// Count verified near-duplicate pairs (threshold [`DUP_THRESHOLD`],
/// min-tokens [`DUP_MIN_TOKENS`], at least one endpoint in a changed file)
/// that did **not** exist at the baseline.
///
/// A pair existed at the baseline iff both endpoints map to baseline
/// counterparts -- matched by `(file, parent, name)`, never by symbol id --
/// and the baseline exact Jaccard similarity is still at or above the
/// threshold. Any unmatched endpoint makes the pair new.
fn new_duplication(
    root: &Path,
    db: &Database,
    parser: &mut CodeParser,
    reference: &str,
    changed: &HashSet<String>,
) -> Result<i64> {
    let pairs =
        fingerprint::find_near_duplicates(db, DUP_THRESHOLD, DUP_MIN_TOKENS, Some(changed))?;
    if pairs.is_empty() {
        return Ok(0);
    }

    // Baseline shingle sets for every function/method in the changed files,
    // keyed by (parent, name). Duplicate keys (rare: cfg variants, overloads)
    // keep all candidates.
    type KeyedShingles = HashMap<(Option<String>, String), Vec<HashSet<u64>>>;
    let mut baseline: HashMap<String, KeyedShingles> = HashMap::new();
    for path in changed {
        let mut keyed = KeyedShingles::new();
        if let Some(source) = gitutil::show_file_in(root, reference, path)? {
            let lang = Language::from_path(Path::new(path));
            let tokens = fingerprint::tokenize(lang, &source);
            let parse = parser.parse(Path::new(path), &source);
            if let (Some(tokens), Some(parse)) = (tokens, parse) {
                for symbol in &parse.symbols {
                    if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
                        continue;
                    }
                    let symbol_tokens: Vec<fingerprint::Tok> = tokens
                        .iter()
                        .filter(|t| t.line >= symbol.line_start && t.line <= symbol.line_end)
                        .cloned()
                        .collect();
                    keyed
                        .entry(symbol_key(symbol))
                        .or_default()
                        .push(fingerprint::shingle_set(&symbol_tokens));
                }
            }
        }
        baseline.insert(path.clone(), keyed);
    }

    // Baseline shingle sets for one current endpoint: from the baseline
    // parse for changed files, from the stored (unchanged) snippet otherwise.
    let baseline_sets = |symbol: &Symbol| -> Option<Vec<HashSet<u64>>> {
        if changed.contains(&symbol.file_path) {
            baseline
                .get(&symbol.file_path)
                .and_then(|keyed| keyed.get(&symbol_key(symbol)))
                .cloned()
        } else {
            fingerprint::symbol_shingles(symbol).map(|s| vec![s])
        }
    };

    let mut new_pairs = 0;
    for pair in &pairs {
        let existed = match (baseline_sets(&pair.a), baseline_sets(&pair.b)) {
            (Some(a_sets), Some(b_sets)) => a_sets.iter().any(|sa| {
                b_sets
                    .iter()
                    .any(|sb| fingerprint::jaccard(sa, sb) >= DUP_THRESHOLD)
            }),
            _ => false, // an unmatched endpoint means the pair is new
        };
        if !existed {
            new_pairs += 1;
        }
    }
    Ok(new_pairs)
}

// ============================================================================
// --fail-on conditions
// ============================================================================

/// Comparison operator in a `--fail-on` condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Ge,
    Le,
    Gt,
    Lt,
}

impl Op {
    fn as_str(self) -> &'static str {
        match self {
            Op::Ge => ">=",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Lt => "<",
        }
    }

    fn holds(self, lhs: i64, rhs: i64) -> bool {
        match self {
            Op::Ge => lhs >= rhs,
            Op::Le => lhs <= rhs,
            Op::Gt => lhs > rhs,
            Op::Lt => lhs < rhs,
        }
    }
}

/// One parsed `--fail-on` condition (`metric OP value`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailCondition {
    pub metric: String,
    pub op: Op,
    pub value: i64,
}

impl FailCondition {
    /// Whether this condition is satisfied (i.e. the gate fails) for the
    /// given metrics.
    pub fn holds(&self, metrics: &Metrics) -> bool {
        metrics
            .get(&self.metric)
            .map(|actual| self.op.holds(actual, self.value))
            .unwrap_or(false)
    }
}

impl fmt::Display for FailCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.metric, self.op.as_str(), self.value)
    }
}

/// Parse a comma-separated `--fail-on` expression like
/// `"new_duplication>0, complexity_delta >= 10"`.
///
/// Metric names are the JSON metric keys exactly ([`Metrics::NAMES`]);
/// operators are `>=`, `<=`, `>`, `<`. Unknown metrics or malformed
/// conditions are errors (exit code 2).
pub fn parse_fail_on(expr: &str) -> Result<Vec<FailCondition>> {
    let mut conditions = Vec::new();
    for part in expr.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(CtxError::Other(format!(
                "invalid --fail-on expression {:?}: empty condition",
                expr
            )));
        }
        // Two-character operators must be tried first.
        let (op, idx, len) = if let Some(i) = part.find(">=") {
            (Op::Ge, i, 2)
        } else if let Some(i) = part.find("<=") {
            (Op::Le, i, 2)
        } else if let Some(i) = part.find('>') {
            (Op::Gt, i, 1)
        } else if let Some(i) = part.find('<') {
            (Op::Lt, i, 1)
        } else {
            return Err(CtxError::Other(format!(
                "invalid --fail-on condition {:?}: expected `metric OP value` with OP one of >=, <=, >, <",
                part
            )));
        };

        let metric = part[..idx].trim();
        if !Metrics::NAMES.contains(&metric) {
            return Err(CtxError::Other(format!(
                "unknown --fail-on metric {:?} (valid metrics: {})",
                metric,
                Metrics::NAMES.join(", ")
            )));
        }

        let value_str = part[idx + len..].trim();
        let value: i64 = value_str.parse().map_err(|_| {
            CtxError::Other(format!(
                "invalid --fail-on condition {:?}: {:?} is not an integer",
                part, value_str
            ))
        })?;

        conditions.push(FailCondition {
            metric: metric.to_string(),
            op,
            value,
        });
    }
    Ok(conditions)
}

/// The subset of `conditions` satisfied by `metrics` (each one fails the gate).
pub fn failed_conditions(conditions: &[FailCondition], metrics: &Metrics) -> Vec<FailCondition> {
    conditions
        .iter()
        .filter(|c| c.holds(metrics))
        .cloned()
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::GitRepo;
    use crate::walker::WalkerConfig;
    use std::fs;
    use tempfile::TempDir;

    // ---------- --fail-on parser ----------

    #[test]
    fn test_parse_fail_on_all_operators() {
        let conds = parse_fail_on(
            "complexity_delta>=10,new_duplication>0,fan_out_delta<=5,symbols_removed<3",
        )
        .unwrap();
        assert_eq!(conds.len(), 4);
        assert_eq!(conds[0].metric, "complexity_delta");
        assert_eq!(conds[0].op, Op::Ge);
        assert_eq!(conds[0].value, 10);
        assert_eq!(conds[1].op, Op::Gt);
        assert_eq!(conds[1].value, 0);
        assert_eq!(conds[2].op, Op::Le);
        assert_eq!(conds[2].value, 5);
        assert_eq!(conds[3].op, Op::Lt);
        assert_eq!(conds[3].value, 3);
    }

    #[test]
    fn test_parse_fail_on_whitespace_tolerance() {
        let conds = parse_fail_on("  new_duplication  >  0  ,  check_violations >= 1 ").unwrap();
        assert_eq!(conds.len(), 2);
        assert_eq!(conds[0].metric, "new_duplication");
        assert_eq!(conds[0].op, Op::Gt);
        assert_eq!(conds[1].metric, "check_violations");
        assert_eq!(conds[1].op, Op::Ge);
        assert_eq!(conds[0].to_string(), "new_duplication > 0");
    }

    #[test]
    fn test_parse_fail_on_negative_values() {
        let conds = parse_fail_on("complexity_delta>=-5").unwrap();
        assert_eq!(conds[0].value, -5);
    }

    #[test]
    fn test_parse_fail_on_unknown_metric_is_error() {
        let err = parse_fail_on("bogus_metric>0").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bogus_metric"), "msg: {}", msg);
        assert!(msg.contains("complexity_delta"), "msg: {}", msg); // lists valid names
    }

    #[test]
    fn test_parse_fail_on_malformed_is_error() {
        for bad in [
            "new_duplication",
            "new_duplication>",
            ">0",
            "new_duplication>x",
            "",
            " , ",
        ] {
            assert!(parse_fail_on(bad).is_err(), "expected error for {:?}", bad);
        }
        // One bad condition poisons the whole expression.
        assert!(parse_fail_on("new_duplication>0,nope").is_err());
    }

    #[test]
    fn test_failed_conditions_evaluation() {
        let metrics = Metrics {
            complexity_delta: 7,
            new_duplication: 1,
            ..Metrics::default()
        };
        let conds =
            parse_fail_on("complexity_delta>=10,new_duplication>0,files_changed<1").unwrap();
        let failed = failed_conditions(&conds, &metrics);
        assert_eq!(failed.len(), 2);
        assert_eq!(failed[0].to_string(), "new_duplication > 0");
        assert_eq!(failed[1].to_string(), "files_changed < 1");
    }

    // ---------- end-to-end scoring ----------

    const V1: &str = r#"
pub fn alpha() -> i64 {
    beta()
}

pub fn beta() -> i64 {
    1
}
"#;

    /// V1 plus a new function and an extra call to it inside alpha().
    /// (Nested `fn` items are not extracted as symbols by the Rust parser,
    /// so the new function is top-level.)
    const V2: &str = r#"
pub fn alpha() -> i64 {
    beta() + gamma()
}

pub fn beta() -> i64 {
    1
}

pub fn gamma() -> i64 {
    2
}
"#;

    /// A function with > 50 normalized tokens (fingerprintable).
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

    /// A structural copy of [`BIG_FN`] with renamed identifiers and changed
    /// literals (still a near-duplicate under normalization).
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

    fn setup_repo() -> (TempDir, GitRepo) {
        let temp = TempDir::new().unwrap();
        let repo = GitRepo::init(temp.path());
        (temp, repo)
    }

    /// Build the initial index (part of test setup, not of what's measured).
    fn index_once(root: &Path) {
        let mut indexer = Indexer::with_config(root, false, WalkerConfig::default()).unwrap();
        indexer.index().unwrap();
    }

    #[test]
    fn test_score_added_symbols_and_calls() {
        let (_temp, repo) = setup_repo();
        repo.commit_file("src/a.rs", V1, "v1");
        index_once(&repo.root);

        // Uncommitted change: new function + extra call.
        repo.write("src/a.rs", V2);
        let report = compute_score(&repo.root, "HEAD").unwrap();

        assert_eq!(report.metrics.files_changed, 1);
        assert!(
            report.metrics.complexity_delta > 0,
            "complexity_delta: {}",
            report.metrics.complexity_delta
        );
        assert!(
            report.metrics.fan_out_delta >= 1,
            "fan_out_delta: {}",
            report.metrics.fan_out_delta
        );
        assert!(
            report.metrics.symbols_added >= 1,
            "symbols_added: {}",
            report.metrics.symbols_added
        );
        assert_eq!(report.metrics.symbols_removed, 0);
        assert_eq!(report.metrics.new_duplication, 0);
        // No rules file: value 0 plus a note.
        assert_eq!(report.metrics.check_violations, 0);
        assert_eq!(report.check_violations_note.as_deref(), Some(NO_RULES_NOTE));
        assert!(report.notes.iter().any(|n| n == FAN_IN_NOTE));

        // Per-file entry carries both sides.
        assert_eq!(report.per_file.len(), 1);
        let file = &report.per_file[0];
        assert_eq!(file.path, "src/a.rs");
        assert!(file.complexity_current > file.complexity_baseline);
        assert!(file.fan_out_current > file.fan_out_baseline);
    }

    #[test]
    fn test_score_reverted_change_has_zero_deltas() {
        let (_temp, repo) = setup_repo();
        repo.commit_file("src/a.rs", V1, "v1");
        index_once(&repo.root);

        // A change that touches the file but not its structure: the file is
        // still reported as changed, but every delta must be exactly zero.
        repo.write("src/a.rs", &format!("{}\n// a trailing comment\n", V1));
        let report = compute_score(&repo.root, "HEAD").unwrap();

        assert_eq!(report.metrics.files_changed, 1);
        assert_eq!(report.metrics.complexity_delta, 0);
        assert_eq!(report.metrics.fan_out_delta, 0);
        assert_eq!(report.metrics.symbols_added, 0);
        assert_eq!(report.metrics.symbols_removed, 0);
        assert_eq!(report.metrics.new_duplication, 0);

        // Full revert: nothing is changed at all.
        repo.write("src/a.rs", V1);
        let report = compute_score(&repo.root, "HEAD").unwrap();
        assert_eq!(report.metrics.files_changed, 0);
        assert_eq!(report.metrics.complexity_delta, 0);
        assert_eq!(report.metrics.fan_out_delta, 0);
        assert_eq!(report.metrics.symbols_added, 0);
        assert_eq!(report.metrics.symbols_removed, 0);
    }

    #[test]
    fn test_score_new_duplication_detected_and_cleared() {
        let (_temp, repo) = setup_repo();
        repo.write("src/a.rs", BIG_FN);
        repo.write("src/b.rs", UNRELATED);
        repo.commit_all("v1");
        index_once(&repo.root);

        // Copy-paste (with renames) the big function into a changed file.
        repo.write("src/b.rs", &format!("{}\n{}", UNRELATED, BIG_FN_COPY));
        let report = compute_score(&repo.root, "HEAD").unwrap();
        assert!(
            report.metrics.new_duplication >= 1,
            "new_duplication: {}",
            report.metrics.new_duplication
        );
        assert!(report.metrics.symbols_added >= 1);

        // --fail-on gate trips on the duplication.
        let conds = parse_fail_on("new_duplication>0").unwrap();
        assert_eq!(failed_conditions(&conds, &report.metrics).len(), 1);

        // Deduplicate: back to clean.
        repo.write("src/b.rs", UNRELATED);
        let report = compute_score(&repo.root, "HEAD").unwrap();
        assert_eq!(report.metrics.new_duplication, 0);
        assert!(failed_conditions(&conds, &report.metrics).is_empty());
    }

    #[test]
    fn test_score_preexisting_duplication_is_not_new() {
        let (_temp, repo) = setup_repo();
        // The duplicate pair already exists at the baseline.
        repo.write("src/a.rs", BIG_FN);
        repo.write("src/b.rs", BIG_FN_COPY);
        repo.commit_all("v1 with existing duplication");
        index_once(&repo.root);

        // Touch one endpoint's file without changing the duplicate.
        repo.write("src/b.rs", &format!("{}\n// touched\n", BIG_FN_COPY));
        let report = compute_score(&repo.root, "HEAD").unwrap();
        assert_eq!(
            report.metrics.new_duplication, 0,
            "pre-existing pair must not count as new"
        );
    }

    #[test]
    fn test_score_added_and_deleted_files() {
        let (_temp, repo) = setup_repo();
        repo.write("src/a.rs", V1);
        repo.write("src/b.rs", UNRELATED);
        repo.commit_all("v1");
        index_once(&repo.root);

        // Add a new untracked file: empty baseline side.
        repo.write("src/c.rs", BIG_FN);
        // Delete a tracked file: empty current side.
        fs::remove_file(repo.root.join("src/b.rs")).unwrap();

        let report = compute_score(&repo.root, "HEAD").unwrap();
        assert_eq!(report.metrics.files_changed, 2);
        assert!(report.metrics.symbols_added >= 1, "added file symbols");
        assert!(report.metrics.symbols_removed >= 1, "deleted file symbols");

        let added = report
            .per_file
            .iter()
            .find(|f| f.path == "src/c.rs")
            .unwrap();
        assert_eq!(added.complexity_baseline, 0);
        assert_eq!(added.fan_out_baseline, 0);
        assert_eq!(added.symbols_removed, 0);

        let deleted = report
            .per_file
            .iter()
            .find(|f| f.path == "src/b.rs")
            .unwrap();
        assert_eq!(deleted.complexity_current, 0);
        assert_eq!(deleted.fan_out_current, 0);
        assert_eq!(deleted.symbols_added, 0);
        assert!(deleted.symbols_removed >= 1);
    }

    #[test]
    fn test_score_errors() {
        // Not a git repository -> operational error.
        let temp = TempDir::new().unwrap();
        let err = compute_score(temp.path(), "HEAD").unwrap_err();
        assert!(matches!(err, CtxError::NotGitRepo));

        // Bad reference -> operational error.
        let (_temp2, repo) = setup_repo();
        repo.commit_file("src/a.rs", V1, "v1");
        let err = compute_score(&repo.root, "no-such-ref").unwrap_err();
        assert!(matches!(err, CtxError::InvalidRevision(_)), "err: {}", err);

        // Present but invalid rules file -> operational error (not swallowed).
        repo.write(".ctx/rules.toml", "[layers\nbroken = [");
        let err = compute_score(&repo.root, "HEAD").unwrap_err();
        assert!(
            err.to_string().contains("invalid rules file"),
            "err: {}",
            err
        );
    }

    /// Performance guard: scoring 3 changed files in a ~2000-file repo must
    /// finish in under 2 seconds (per-file queries, fingerprints loaded
    /// once, no O(n^2) work).
    ///
    /// Run manually with:
    /// `cargo test --release test_score_benchmark_2000_files -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark; run manually with --ignored"]
    fn test_score_benchmark_2000_files() {
        let (_temp, repo) = setup_repo();

        // ~2000 small, distinct functions (< 50 tokens: excluded from the
        // duplication scan by min-tokens, as typical glue code would be).
        for i in 0..2000 {
            repo.write(
                &format!("src/gen/f{:04}.rs", i),
                &format!(
                    "pub fn func_{i}(x: i64) -> i64 {{\n    let y = x * {i} + 1;\n    y - {i}\n}}\n",
                    i = i
                ),
            );
        }
        repo.write("src/big.rs", BIG_FN);
        repo.commit_all("v1");
        index_once(&repo.root);

        // Change 3 files.
        for i in [7, 900, 1999] {
            repo.write(
                &format!("src/gen/f{:04}.rs", i),
                &format!(
                    "pub fn func_{i}(x: i64) -> i64 {{\n    let y = x * {i} + 2;\n    y + {i}\n}}\n",
                    i = i
                ),
            );
        }

        let start = std::time::Instant::now();
        let report = compute_score(&repo.root, "HEAD").unwrap();
        let elapsed = start.elapsed();

        assert_eq!(report.metrics.files_changed, 3);
        eprintln!(
            "score over 2001-file repo with 3 changed files: {:?}",
            elapsed
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "ctx score took {:?} (budget: 2s)",
            elapsed
        );
    }
}
