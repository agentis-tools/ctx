//! Gate evaluation: compare scenario medians against the committed baseline
//! and the absolute latency budgets, with a runner-class scale factor.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A scenario fails against the baseline when its median exceeds
/// `baseline * REGRESSION_FACTOR`.
pub const REGRESSION_FACTOR: f64 = 1.20;

/// Max resident set (MB) allowed for the RSS-gated scenarios, before scaling.
pub const RSS_BUDGET_MB: f64 = 300.0;

// ============================================================================
// Baseline file model (perf/baselines/<runner>.json)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Baseline {
    pub meta: BaselineMeta,
    pub scenarios: BTreeMap<String, BaselineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BaselineMeta {
    /// Cargo profile the ctx binary was built with (always "perf").
    pub profile: String,
    /// `ctx::fixture::FIXTURE_FORMAT_VERSION` at capture time; a mismatch
    /// makes the baseline incomparable and it is ignored with a warning.
    pub fixture_format_version: u32,
    pub budget_scale_at_capture: f64,
    pub commit: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct BaselineEntry {
    pub median_ms: f64,
    pub max_rss_kb: u64,
}

// ============================================================================
// Gate decision
// ============================================================================

/// Outcome of gating one scenario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    Pass,
    /// Passed, but with caveats (e.g. no baseline entry to compare against).
    Warn(Vec<String>),
    Fail(Vec<String>),
}

impl Gate {
    pub fn label(&self) -> &'static str {
        match self {
            Gate::Pass => "pass",
            Gate::Warn(_) => "warn",
            Gate::Fail(_) => "fail",
        }
    }

    pub fn notes(&self) -> &[String] {
        match self {
            Gate::Pass => &[],
            Gate::Warn(notes) | Gate::Fail(notes) => notes,
        }
    }
}

/// Evaluate one scenario:
///
/// - baseline regression: `median_ms > baseline.median_ms * 1.20`
///   (only when a baseline entry exists; a missing entry is a warn-pass);
/// - absolute budget: `median_ms > budget_ms * scale`
///   (skipped for budget-less, informational scenarios);
/// - memory: `max_rss_kb > 300 MB * scale` for RSS-gated scenarios.
pub fn evaluate(
    median_ms: f64,
    max_rss_kb: u64,
    budget_ms: Option<f64>,
    rss_gated: bool,
    baseline: Option<&BaselineEntry>,
    scale: f64,
) -> Gate {
    let mut failures = Vec::new();
    let mut warnings = Vec::new();

    match baseline {
        Some(entry) => {
            let limit = entry.median_ms * REGRESSION_FACTOR;
            if median_ms > limit {
                failures.push(format!(
                    "regression: median {median_ms:.1} ms > baseline {:.1} ms * {REGRESSION_FACTOR} = {limit:.1} ms",
                    entry.median_ms
                ));
            }
        }
        None => warnings.push("no baseline entry; regression gate skipped".to_string()),
    }

    if let Some(budget) = budget_ms {
        let limit = budget * scale;
        if median_ms > limit {
            failures.push(format!(
                "budget: median {median_ms:.1} ms > {budget:.0} ms * scale {scale} = {limit:.1} ms"
            ));
        }
    }

    if rss_gated {
        let rss_mb = max_rss_kb as f64 / 1024.0;
        let limit = RSS_BUDGET_MB * scale;
        if rss_mb > limit {
            failures.push(format!(
                "rss: {rss_mb:.1} MB > {RSS_BUDGET_MB:.0} MB * scale {scale} = {limit:.1} MB"
            ));
        }
    }

    if !failures.is_empty() {
        Gate::Fail(failures)
    } else if !warnings.is_empty() {
        Gate::Warn(warnings)
    } else {
        Gate::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_fail(gate: &Gate) -> bool {
        matches!(gate, Gate::Fail(_))
    }

    fn baseline(median_ms: f64) -> BaselineEntry {
        BaselineEntry {
            median_ms,
            max_rss_kb: 100 * 1024,
        }
    }

    #[test]
    fn passes_within_baseline_and_budget() {
        let gate = evaluate(
            100.0,
            50 * 1024,
            Some(300.0),
            true,
            Some(&baseline(95.0)),
            1.0,
        );
        assert_eq!(gate, Gate::Pass);
    }

    #[test]
    fn fails_on_baseline_regression() {
        // 121 > 100 * 1.20
        let gate = evaluate(121.0, 0, Some(300.0), false, Some(&baseline(100.0)), 1.0);
        assert!(is_fail(&gate));
        assert!(gate.notes()[0].contains("regression"));
        // Exactly at the limit passes.
        let gate = evaluate(120.0, 0, Some(300.0), false, Some(&baseline(100.0)), 1.0);
        assert_eq!(gate, Gate::Pass);
    }

    #[test]
    fn fails_on_budget_overrun() {
        let gate = evaluate(301.0, 0, Some(300.0), false, Some(&baseline(300.0)), 1.0);
        assert!(is_fail(&gate));
        assert!(gate.notes()[0].contains("budget"));
    }

    #[test]
    fn missing_baseline_is_warn_pass() {
        let gate = evaluate(100.0, 0, Some(300.0), false, None, 1.0);
        assert_eq!(gate.label(), "warn");
        assert!(!is_fail(&gate));
        assert!(gate.notes()[0].contains("no baseline"));
    }

    #[test]
    fn budget_less_scenario_only_gates_on_baseline() {
        // No budget, no baseline, huge median: warn-pass, never budget-fail.
        let gate = evaluate(1_000_000.0, 0, None, false, None, 1.0);
        assert_eq!(gate.label(), "warn");
    }

    #[test]
    fn fails_on_rss_when_gated() {
        let rss_kb = 301 * 1024; // 301 MB
        let gate = evaluate(1.0, rss_kb, None, true, Some(&baseline(100.0)), 1.0);
        assert!(is_fail(&gate));
        assert!(gate.notes()[0].contains("rss"));
        // Same RSS is fine when the scenario is not RSS-gated.
        let gate = evaluate(1.0, rss_kb, None, false, Some(&baseline(100.0)), 1.0);
        assert_eq!(gate, Gate::Pass);
    }

    #[test]
    fn scale_applies_to_budget_and_rss() {
        // 400 <= 300 * 1.5, and 400 MB <= 300 MB * 1.5.
        let gate = evaluate(
            400.0,
            400 * 1024,
            Some(300.0),
            true,
            Some(&baseline(400.0)),
            1.5,
        );
        assert_eq!(gate, Gate::Pass);
        // ...but the regression factor is NOT scaled.
        let gate = evaluate(400.0, 0, Some(300.0), false, Some(&baseline(300.0)), 1.5);
        assert!(is_fail(&gate), "regression gate ignores budget scale");
    }

    #[test]
    fn baseline_json_round_trip() {
        let mut scenarios = BTreeMap::new();
        scenarios.insert(
            "map_cached".to_string(),
            BaselineEntry {
                median_ms: 123.4,
                max_rss_kb: 56789,
            },
        );
        let baseline = Baseline {
            meta: BaselineMeta {
                profile: "perf".to_string(),
                fixture_format_version: 1,
                budget_scale_at_capture: 1.5,
                commit: "abc123".to_string(),
            },
            scenarios,
        };
        let json = serde_json::to_string_pretty(&baseline).unwrap();
        let parsed: Baseline = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, baseline);
    }
}
