//! Scenario table and the per-scenario execution protocol.

use std::path::{Path, PathBuf};

use ctx::fixture::{self, FixtureSpec};

use crate::runner::{self, RunResult};

/// The v1.* SQL smoke query (matches the published examples).
pub const SQL_QUERY: &str =
    "SELECT kind, count(*) FROM v1.symbols GROUP BY kind ORDER BY 2 DESC LIMIT 10";

/// Minimal-but-real rules file for `check_against_head`: one layer pair with
/// a forbidden edge (the fixture's zipf call graph guarantees findings, which
/// exit 1 — a success for timing purposes) plus a limit rule so metric
/// evaluation is exercised too.
const RULES_TOML: &str = r#"version = 1

[layers]
core = ["src/m00/**"]
api = ["src/m01/**"]

[[rules.forbidden]]
from = "api"
to = "core"
reason = "perf fixture rule"

[[rules.limit]]
metric = "complexity"
scope = "symbol"
max = 100
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureKind {
    Repo2k,
    Repo150k,
}

/// What to do before each run (warmup and timed alike).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerRun {
    Nothing,
    /// Delete `.ctx/` so the next `ctx index` is cold.
    DeleteCtx,
    /// Rewrite `n` files with `fixture::apply_change_set`, round = run index,
    /// so every run performs identical fresh work.
    ApplyChangeSet(usize),
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: &'static str,
    pub fixture: FixtureKind,
    /// Run `ctx index` once during preparation.
    pub prepare_index: bool,
    /// Run `ctx map --budget 2000` once during preparation to warm the
    /// PageRank cache (reused while the index is unchanged).
    pub prepare_warm_map: bool,
    /// Write `.ctx/rules.toml` during preparation (for `ctx check`).
    pub prepare_rules: bool,
    pub per_run: PerRun,
    pub args: &'static [&'static str],
    /// Absolute latency budget in ms; `None` = informational (no budget gate).
    pub budget_ms: Option<f64>,
    /// Included in the max-RSS gate (< 300 MB * scale)?
    pub rss_gated: bool,
    /// Only run in `--suite full`.
    pub full_only: bool,
}

/// The full scenario table (budgets from perf/README.md).
pub fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            // Cold 2k indexing is not in the published budget table;
            // reported for information and gated only against the baseline.
            name: "index_cold_2k",
            fixture: FixtureKind::Repo2k,
            prepare_index: false,
            prepare_warm_map: false,
            prepare_rules: false,
            per_run: PerRun::DeleteCtx,
            args: &["index"],
            budget_ms: None,
            rss_gated: false,
            full_only: false,
        },
        Scenario {
            name: "index_incremental_1",
            fixture: FixtureKind::Repo2k,
            prepare_index: true,
            prepare_warm_map: false,
            prepare_rules: false,
            per_run: PerRun::ApplyChangeSet(1),
            args: &["index"],
            budget_ms: Some(300.0),
            rss_gated: true,
            full_only: false,
        },
        Scenario {
            name: "score_3_changed",
            fixture: FixtureKind::Repo2k,
            prepare_index: true,
            prepare_warm_map: true,
            prepare_rules: false,
            per_run: PerRun::ApplyChangeSet(3),
            args: &["score", "--against", "HEAD"],
            budget_ms: Some(2000.0),
            rss_gated: true,
            full_only: false,
        },
        Scenario {
            name: "check_against_head",
            fixture: FixtureKind::Repo2k,
            prepare_index: true,
            prepare_warm_map: true,
            prepare_rules: true,
            per_run: PerRun::ApplyChangeSet(3),
            args: &["check", "--against", "HEAD"],
            budget_ms: Some(1000.0),
            rss_gated: true,
            full_only: false,
        },
        Scenario {
            name: "map_cached",
            fixture: FixtureKind::Repo2k,
            prepare_index: true,
            prepare_warm_map: true,
            prepare_rules: false,
            per_run: PerRun::Nothing,
            args: &["map", "--budget", "2000"],
            budget_ms: Some(500.0),
            rss_gated: true,
            full_only: false,
        },
        Scenario {
            name: "sql_v1_query",
            fixture: FixtureKind::Repo2k,
            prepare_index: true,
            prepare_warm_map: false,
            prepare_rules: false,
            per_run: PerRun::Nothing,
            args: &["sql", SQL_QUERY],
            budget_ms: Some(500.0),
            rss_gated: false,
            full_only: false,
        },
        Scenario {
            name: "index_cold_150k",
            fixture: FixtureKind::Repo150k,
            prepare_index: false,
            prepare_warm_map: false,
            prepare_rules: false,
            per_run: PerRun::DeleteCtx,
            args: &["index"],
            budget_ms: Some(60_000.0),
            rss_gated: false,
            full_only: true,
        },
    ]
}

impl Scenario {
    /// Fixture spec for this scenario. `--smoke` substitutes the tiny spec
    /// everywhere so the harness's own E2E test finishes in seconds.
    pub fn spec(&self, smoke: bool) -> FixtureSpec {
        if smoke {
            return FixtureSpec::tiny();
        }
        match self.fixture {
            FixtureKind::Repo2k => FixtureSpec::repo_2k(),
            FixtureKind::Repo150k => FixtureSpec::repo_150k_loc(),
        }
    }

    /// Discarded warmup runs before timing starts.
    pub fn warmups(&self, smoke: bool) -> u32 {
        if smoke || self.fixture == FixtureKind::Repo150k {
            1
        } else {
            2
        }
    }

    /// Timed runs (median is taken over these).
    pub fn timed_runs(&self, smoke: bool) -> u32 {
        if smoke {
            2
        } else if self.fixture == FixtureKind::Repo150k {
            3
        } else {
            5
        }
    }
}

/// Result of executing one scenario's full protocol.
#[derive(Debug)]
pub enum ScenarioOutcome {
    Measured {
        /// Wall-clock ms per timed run, in run order.
        runs_ms: Vec<f64>,
        /// Max `ru_maxrss` (KB) across the timed runs.
        max_rss_kb: u64,
    },
    /// Scenario cannot run with this ctx binary (e.g. built without the
    /// duckdb feature); reported but never failed.
    Skipped(String),
    /// The command errored (exit >= 2 or signal): a scenario failure.
    Failed(String),
}

/// Execute a scenario: generate its fixture under `work_dir`, run the
/// prepare-once steps, then warmups and timed runs with per-run preparation.
pub fn run_scenario(
    scenario: &Scenario,
    ctx_bin: &Path,
    work_dir: &Path,
    smoke: bool,
) -> Result<ScenarioOutcome, String> {
    let spec = scenario.spec(smoke);
    let repo = work_dir.join(scenario.name);
    eprintln!(
        "[{}] generating fixture ({} files)...",
        scenario.name, spec.files
    );
    fixture::generate(&spec, &repo).map_err(|e| format!("fixture generation failed: {e}"))?;

    // Prepare-once steps (untimed).
    if scenario.prepare_index {
        prepare_step(scenario.name, ctx_bin, &["index"], &repo)?;
    }
    if scenario.prepare_warm_map {
        prepare_step(scenario.name, ctx_bin, &["map", "--budget", "2000"], &repo)?;
    }
    if scenario.prepare_rules {
        std::fs::create_dir_all(repo.join(".ctx"))
            .map_err(|e| format!("cannot create .ctx: {e}"))?;
        std::fs::write(repo.join(".ctx").join("rules.toml"), RULES_TOML)
            .map_err(|e| format!("cannot write rules.toml: {e}"))?;
    }

    // NOTE: for score/check we deliberately do NOT re-index between runs.
    // Both commands refresh the index internally, and the published budgets
    // include that refresh — this matches how the Claude Code hook path
    // invokes them (dirty tree, possibly stale index).
    let warmups = scenario.warmups(smoke);
    let timed = scenario.timed_runs(smoke);
    let mut runs_ms = Vec::with_capacity(timed as usize);
    let mut max_rss_kb = 0u64;
    // Change-set rounds run through warmups and timed runs on one counter so
    // every run rewrites a fresh, deterministic file selection.
    let mut round: u32 = 0;

    for i in 0..(warmups + timed) {
        prepare_run(scenario, &spec, &repo, &mut round)?;
        let result = runner::run_once(ctx_bin, scenario.args, &repo)
            .map_err(|e| format!("failed to spawn {}: {e}", ctx_bin.display()))?;

        if let Some(reason) = skip_reason(&result) {
            eprintln!("[{}] skipped: {reason}", scenario.name);
            return Ok(ScenarioOutcome::Skipped(reason));
        }
        if !result.is_success() {
            return Ok(ScenarioOutcome::Failed(format!(
                "ctx {} exited with code {}: {}",
                scenario.args.join(" "),
                result.exit_code,
                snippet(&result.stderr)
            )));
        }

        let is_warmup = i < warmups;
        eprintln!(
            "[{}] {} {:.1} ms (rss {} MB)",
            scenario.name,
            if is_warmup { "warmup" } else { "run   " },
            result.wall_ms,
            result.max_rss_kb / 1024
        );
        if !is_warmup {
            runs_ms.push(result.wall_ms);
            max_rss_kb = max_rss_kb.max(result.max_rss_kb);
        }
    }

    Ok(ScenarioOutcome::Measured {
        runs_ms,
        max_rss_kb,
    })
}

/// Per-run preparation so every run does identical fresh work.
fn prepare_run(
    scenario: &Scenario,
    spec: &FixtureSpec,
    repo: &Path,
    round: &mut u32,
) -> Result<(), String> {
    match scenario.per_run {
        PerRun::Nothing => Ok(()),
        PerRun::DeleteCtx => {
            let ctx_dir = repo.join(".ctx");
            if ctx_dir.exists() {
                std::fs::remove_dir_all(&ctx_dir)
                    .map_err(|e| format!("cannot delete .ctx: {e}"))?;
            }
            Ok(())
        }
        PerRun::ApplyChangeSet(n) => {
            // Smoke fixtures are tiny; never ask for more files than exist.
            let n = n.min(spec.files);
            let _changed: Vec<PathBuf> = fixture::apply_change_set(spec, repo, n, *round)
                .map_err(|e| format!("apply_change_set failed: {e}"))?;
            *round += 1;
            Ok(())
        }
    }
}

/// Run an untimed preparation command; exit 0/1 are both fine.
fn prepare_step(name: &str, ctx_bin: &Path, args: &[&str], repo: &Path) -> Result<(), String> {
    eprintln!("[{name}] prepare: ctx {}", args.join(" "));
    let result = runner::run_once(ctx_bin, args, repo)
        .map_err(|e| format!("failed to spawn {}: {e}", ctx_bin.display()))?;
    if !result.is_success() {
        return Err(format!(
            "prepare step 'ctx {}' exited with code {}: {}",
            args.join(" "),
            result.exit_code,
            snippet(&result.stderr)
        ));
    }
    Ok(())
}

/// A ctx binary built without the duckdb feature exits 2 with a distinctive
/// message on `ctx sql`; treat that as a skip, not a failure, so local runs
/// of a stub binary don't hard-fail.
fn skip_reason(result: &RunResult) -> Option<String> {
    if result.exit_code == 2 && result.stderr.contains("requires the duckdb feature") {
        Some("ctx binary built without the duckdb feature".to_string())
    } else {
        None
    }
}

fn snippet(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.len() > 400 {
        format!("{}...", &trimmed[..400])
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_published_budgets() {
        let scenarios = all();
        let get = |name: &str| {
            scenarios
                .iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("missing scenario {name}"))
        };
        assert_eq!(get("index_cold_2k").budget_ms, None);
        assert_eq!(get("index_incremental_1").budget_ms, Some(300.0));
        assert_eq!(get("score_3_changed").budget_ms, Some(2000.0));
        assert_eq!(get("check_against_head").budget_ms, Some(1000.0));
        assert_eq!(get("map_cached").budget_ms, Some(500.0));
        assert_eq!(get("sql_v1_query").budget_ms, Some(500.0));
        let cold = get("index_cold_150k");
        assert_eq!(cold.budget_ms, Some(60_000.0));
        assert!(cold.full_only);
        // RSS gate covers exactly score/check/map/index_incremental.
        let gated: Vec<&str> = scenarios
            .iter()
            .filter(|s| s.rss_gated)
            .map(|s| s.name)
            .collect();
        assert_eq!(
            gated,
            [
                "index_incremental_1",
                "score_3_changed",
                "check_against_head",
                "map_cached"
            ]
        );
    }

    #[test]
    fn run_counts_follow_protocol() {
        let scenarios = all();
        for s in &scenarios {
            if s.fixture == FixtureKind::Repo150k {
                assert_eq!(
                    (s.warmups(false), s.timed_runs(false)),
                    (1, 3),
                    "{}",
                    s.name
                );
            } else {
                assert_eq!(
                    (s.warmups(false), s.timed_runs(false)),
                    (2, 5),
                    "{}",
                    s.name
                );
            }
            assert_eq!(s.timed_runs(true), 2, "smoke uses N=2 for {}", s.name);
            assert_eq!(s.spec(true).files, FixtureSpec::tiny().files);
        }
    }

    #[test]
    fn rules_toml_parses_as_toml() {
        // Cheap structural sanity check without depending on the toml crate:
        // the ctx side validates for real; here we pin the key sections.
        assert!(RULES_TOML.contains("[layers]"));
        assert!(RULES_TOML.contains("[[rules.forbidden]]"));
        assert!(RULES_TOML.contains("[[rules.limit]]"));
    }
}
