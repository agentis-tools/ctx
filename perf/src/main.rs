//! perf-harness: operational performance harness for the ctx binary.
//!
//! Spawns a prebuilt ctx binary (env `CTX_PERF_BIN`) against deterministic
//! synthetic fixtures, times each scenario, and gates medians against the
//! committed baseline and the absolute latency budgets. See perf/README.md.
//!
//! Exit codes: 0 = all gates pass (or report-only mode), 1 = at least one
//! scenario failed a gate or errored, 2 = harness operational error.

mod compare;
mod report;
mod runner;
mod scenarios;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use compare::{Baseline, BaselineEntry, BaselineMeta};
use report::{Results, ResultsMeta, ScenarioResult};
use scenarios::{Scenario, ScenarioOutcome};

const USAGE: &str = "\
perf-harness: latency-budget harness for the ctx binary

USAGE:
    perf-harness [OPTIONS]

OPTIONS:
    --suite pr|full       Scenario suite (default: pr; full adds index_cold_150k)
    --baseline <path>     Baseline JSON to compare against (and target of --write-baseline)
    --write-baseline      Overwrite the baseline file from this run (never in CI)
    --json-out <path>     Write full results JSON to <path>
    --smoke               Tiny fixtures, N=2, report-only: the harness's own E2E test

ENVIRONMENT:
    CTX_PERF_BIN            Path to the ctx binary to measure (required)
    CTX_PERF_BUDGET_SCALE   Budget/RSS scale factor (default 1.0; 1.5 on hosted CI)
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Suite {
    Pr,
    Full,
}

#[derive(Debug)]
struct Args {
    suite: Suite,
    baseline: Option<PathBuf>,
    write_baseline: bool,
    json_out: Option<PathBuf>,
    smoke: bool,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args {
        suite: Suite::Pr,
        baseline: None,
        write_baseline: false,
        json_out: None,
        smoke: false,
    };
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--suite" => {
                args.suite = match it.next().map(String::as_str) {
                    Some("pr") => Suite::Pr,
                    Some("full") => Suite::Full,
                    other => return Err(format!("--suite expects 'pr' or 'full', got {other:?}")),
                };
            }
            "--baseline" => {
                let path = it.next().ok_or("--baseline expects a path")?;
                args.baseline = Some(PathBuf::from(path));
            }
            "--write-baseline" => args.write_baseline = true,
            "--json-out" => {
                let path = it.next().ok_or("--json-out expects a path")?;
                args.json_out = Some(PathBuf::from(path));
            }
            "--smoke" => args.smoke = true,
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if args.write_baseline && args.baseline.is_none() {
        return Err("--write-baseline requires --baseline <path>".to_string());
    }
    Ok(args)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(args) => args,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("error: {msg}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(any_failed) => {
            if any_failed {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &Args) -> Result<bool, String> {
    let ctx_bin = resolve_ctx_bin()?;
    let scale = budget_scale()?;
    let baseline = load_baseline(args)?;

    let selected: Vec<Scenario> = scenarios::all()
        .into_iter()
        .filter(|s| args.suite == Suite::Full || !s.full_only)
        .collect();

    let work_dir = std::env::temp_dir().join(format!("ctx-perf-{}", std::process::id()));
    std::fs::create_dir_all(&work_dir).map_err(|e| format!("cannot create work dir: {e}"))?;

    let mut results = BTreeMap::new();
    let mut any_failed = false;

    for scenario in &selected {
        let outcome = scenarios::run_scenario(scenario, &ctx_bin, &work_dir, args.smoke)?;
        let baseline_entry = baseline
            .as_ref()
            .and_then(|b| b.scenarios.get(scenario.name))
            .copied();
        let result = evaluate_outcome(scenario, outcome, baseline_entry, scale, args.smoke);
        if result.status == "fail" {
            any_failed = true;
        }
        results.insert(scenario.name.to_string(), result);
    }

    // Best-effort cleanup: the 150k fixture is sizable.
    let _ = std::fs::remove_dir_all(&work_dir);

    let results = Results {
        meta: ResultsMeta {
            profile: "perf".to_string(),
            fixture_format_version: ctx::fixture::FIXTURE_FORMAT_VERSION,
            budget_scale: scale,
            commit: detect_commit(),
            os: std::env::consts::OS.to_string(),
            suite: match args.suite {
                Suite::Pr => "pr".to_string(),
                Suite::Full => "full".to_string(),
            },
            smoke: args.smoke,
        },
        scenarios: results,
    };

    let table = report::markdown_table(&results);
    println!("{table}");
    append_step_summary(&table);

    if let Some(path) = &args.json_out {
        let json =
            serde_json::to_string_pretty(&results).map_err(|e| format!("results JSON: {e}"))?;
        std::fs::write(path, json + "\n")
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        eprintln!("results written to {}", path.display());
    }

    if args.write_baseline {
        if args.smoke {
            return Err("refusing to write a baseline from a --smoke run".to_string());
        }
        let path = args.baseline.as_ref().expect("checked in parse_args");
        write_baseline(path, &results, scale)?;
        eprintln!("baseline written to {}", path.display());
    }

    Ok(any_failed)
}

/// Fold a scenario outcome and its gate decision into a report row.
fn evaluate_outcome(
    scenario: &Scenario,
    outcome: ScenarioOutcome,
    baseline: Option<BaselineEntry>,
    scale: f64,
    smoke: bool,
) -> ScenarioResult {
    match outcome {
        ScenarioOutcome::Skipped(reason) => ScenarioResult {
            status: "skip".to_string(),
            median_ms: None,
            min_ms: None,
            runs_ms: vec![],
            max_rss_kb: None,
            budget_ms: scenario.budget_ms,
            baseline_median_ms: None,
            notes: vec![reason],
        },
        ScenarioOutcome::Failed(reason) => ScenarioResult {
            status: "fail".to_string(),
            median_ms: None,
            min_ms: None,
            runs_ms: vec![],
            max_rss_kb: None,
            budget_ms: scenario.budget_ms,
            baseline_median_ms: None,
            notes: vec![reason],
        },
        ScenarioOutcome::Measured {
            runs_ms,
            max_rss_kb,
        } => {
            let median_ms = report::median(&runs_ms);
            let min_ms = runs_ms.iter().cloned().fold(f64::INFINITY, f64::min);
            if smoke {
                // Smoke runs report only; budgets and baselines are not
                // meaningful against the tiny fixture.
                return ScenarioResult {
                    status: "info".to_string(),
                    median_ms: Some(median_ms),
                    min_ms: Some(min_ms),
                    runs_ms,
                    max_rss_kb: Some(max_rss_kb),
                    budget_ms: None,
                    baseline_median_ms: None,
                    notes: vec!["smoke: report only".to_string()],
                };
            }
            let gate = compare::evaluate(
                median_ms,
                max_rss_kb,
                scenario.budget_ms,
                scenario.rss_gated,
                baseline.as_ref(),
                scale,
            );
            ScenarioResult {
                status: gate.label().to_string(),
                median_ms: Some(median_ms),
                min_ms: Some(min_ms),
                runs_ms,
                max_rss_kb: Some(max_rss_kb),
                budget_ms: scenario.budget_ms,
                baseline_median_ms: baseline.map(|b| b.median_ms),
                notes: gate.notes().to_vec(),
            }
        }
    }
}

fn resolve_ctx_bin() -> Result<PathBuf, String> {
    let raw = std::env::var("CTX_PERF_BIN").map_err(|_| {
        "CTX_PERF_BIN is not set. Build the binary first:\n  \
         cargo build --profile perf --all-features\n\
         then point CTX_PERF_BIN at target/perf/ctx (absolute path)."
            .to_string()
    })?;
    let path = PathBuf::from(&raw);
    if !path.is_file() {
        return Err(format!(
            "CTX_PERF_BIN points at '{raw}', which is not an existing file"
        ));
    }
    Ok(path)
}

fn budget_scale() -> Result<f64, String> {
    match std::env::var("CTX_PERF_BUDGET_SCALE") {
        Err(_) => Ok(1.0),
        Ok(raw) => {
            let scale: f64 = raw
                .parse()
                .map_err(|_| format!("CTX_PERF_BUDGET_SCALE='{raw}' is not a number"))?;
            if !scale.is_finite() || scale <= 0.0 {
                return Err(format!("CTX_PERF_BUDGET_SCALE must be positive, got {raw}"));
            }
            Ok(scale)
        }
    }
}

/// Load the baseline if a path was given and the file exists. A missing file
/// or a fixture-format mismatch degrades to "no baseline" with a warning
/// (every scenario then warn-passes its regression gate).
fn load_baseline(args: &Args) -> Result<Option<Baseline>, String> {
    let Some(path) = &args.baseline else {
        return Ok(None);
    };
    if !path.exists() {
        eprintln!(
            "warning: baseline file {} does not exist; regression gates are warn-pass \
             (see perf/baselines/README.md for the capture procedure)",
            path.display()
        );
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read baseline {}: {e}", path.display()))?;
    let baseline: Baseline = serde_json::from_str(&text)
        .map_err(|e| format!("baseline {} is not valid: {e}", path.display()))?;
    if baseline.meta.fixture_format_version != ctx::fixture::FIXTURE_FORMAT_VERSION {
        eprintln!(
            "warning: baseline fixture_format_version {} != current {}; ignoring baseline \
             (recapture it per perf/baselines/README.md)",
            baseline.meta.fixture_format_version,
            ctx::fixture::FIXTURE_FORMAT_VERSION
        );
        return Ok(None);
    }
    Ok(Some(baseline))
}

fn write_baseline(path: &Path, results: &Results, scale: f64) -> Result<(), String> {
    let mut scenarios = BTreeMap::new();
    for (name, r) in &results.scenarios {
        if let (Some(median_ms), Some(max_rss_kb)) = (r.median_ms, r.max_rss_kb) {
            scenarios.insert(
                name.clone(),
                BaselineEntry {
                    median_ms,
                    max_rss_kb,
                },
            );
        }
    }
    let baseline = Baseline {
        meta: BaselineMeta {
            profile: "perf".to_string(),
            fixture_format_version: ctx::fixture::FIXTURE_FORMAT_VERSION,
            budget_scale_at_capture: scale,
            commit: detect_commit(),
        },
        scenarios,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let json =
        serde_json::to_string_pretty(&baseline).map_err(|e| format!("baseline JSON: {e}"))?;
    std::fs::write(path, json + "\n").map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn detect_commit() -> String {
    if let Ok(sha) = std::env::var("GITHUB_SHA") {
        if !sha.is_empty() {
            return sha;
        }
    }
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Append the report to the GitHub Actions job summary when available.
fn append_step_summary(table: &str) {
    let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") else {
        return;
    };
    let opened = std::fs::OpenOptions::new().append(true).open(&path);
    if let Ok(mut file) = opened {
        let _ = writeln!(file, "## ctx perf harness\n\n{table}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        parse_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parse_defaults() {
        let args = parse(&[]).unwrap();
        assert_eq!(args.suite, Suite::Pr);
        assert!(args.baseline.is_none());
        assert!(!args.write_baseline);
        assert!(args.json_out.is_none());
        assert!(!args.smoke);
    }

    #[test]
    fn parse_full_invocation() {
        let args = parse(&[
            "--suite",
            "full",
            "--baseline",
            "perf/baselines/ubuntu-latest.json",
            "--json-out",
            "out.json",
            "--smoke",
        ])
        .unwrap();
        assert_eq!(args.suite, Suite::Full);
        assert_eq!(
            args.baseline.as_deref(),
            Some(Path::new("perf/baselines/ubuntu-latest.json"))
        );
        assert_eq!(args.json_out.as_deref(), Some(Path::new("out.json")));
        assert!(args.smoke);
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(parse(&["--suite", "nightly"]).is_err());
        assert!(parse(&["--frobnicate"]).is_err());
        assert!(
            parse(&["--write-baseline"]).is_err(),
            "--write-baseline requires --baseline"
        );
    }
}
