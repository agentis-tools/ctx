//! Result aggregation, JSON output, and the markdown report table.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Median of a non-empty slice. Even lengths average the two middle values.
pub fn median(samples: &[f64]) -> f64 {
    assert!(!samples.is_empty(), "median of an empty sample set");
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

// ============================================================================
// Results model (--json-out)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Results {
    pub meta: ResultsMeta,
    pub scenarios: BTreeMap<String, ScenarioResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResultsMeta {
    pub profile: String,
    pub fixture_format_version: u32,
    pub budget_scale: f64,
    pub commit: String,
    pub os: String,
    pub suite: String,
    pub smoke: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScenarioResult {
    /// `pass`, `warn`, `fail`, `skip`, or `info` (smoke / unenforced).
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs_ms: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rss_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_median_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

// ============================================================================
// Markdown table
// ============================================================================

fn fmt_ms(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.1}"),
        None => "-".to_string(),
    }
}

fn fmt_rss_mb(value: Option<u64>) -> String {
    match value {
        Some(kb) => format!("{:.1}", kb as f64 / 1024.0),
        None => "-".to_string(),
    }
}

/// Render the report as a markdown table (scenarios in name order).
pub fn markdown_table(results: &Results) -> String {
    let mut out = String::new();
    out.push_str(
        "| scenario | median (ms) | min (ms) | budget (ms) | baseline (ms) | rss (MB) | status |\n",
    );
    out.push_str("|---|---:|---:|---:|---:|---:|---|\n");
    for (name, r) in &results.scenarios {
        let status = if r.notes.is_empty() {
            r.status.clone()
        } else {
            format!("{} ({})", r.status, r.notes.join("; "))
        };
        out.push_str(&format!(
            "| {name} | {} | {} | {} | {} | {} | {status} |\n",
            fmt_ms(r.median_ms),
            fmt_ms(r.min_ms),
            fmt_ms(r.budget_ms),
            fmt_ms(r.baseline_median_ms),
            fmt_rss_mb(r.max_rss_kb),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_odd_even_single() {
        assert_eq!(median(&[3.0]), 3.0);
        assert_eq!(median(&[5.0, 1.0, 3.0]), 3.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    fn sample_results() -> Results {
        let mut scenarios = BTreeMap::new();
        scenarios.insert(
            "map_cached".to_string(),
            ScenarioResult {
                status: "pass".to_string(),
                median_ms: Some(123.45),
                min_ms: Some(120.0),
                runs_ms: vec![120.0, 123.45, 130.0],
                max_rss_kb: Some(51200),
                budget_ms: Some(500.0),
                baseline_median_ms: Some(110.0),
                notes: vec![],
            },
        );
        scenarios.insert(
            "sql_v1_query".to_string(),
            ScenarioResult {
                status: "skip".to_string(),
                median_ms: None,
                min_ms: None,
                runs_ms: vec![],
                max_rss_kb: None,
                budget_ms: Some(500.0),
                baseline_median_ms: None,
                notes: vec!["ctx binary built without the duckdb feature".to_string()],
            },
        );
        Results {
            meta: ResultsMeta {
                profile: "perf".to_string(),
                fixture_format_version: 1,
                budget_scale: 1.0,
                commit: "abc123".to_string(),
                os: "linux".to_string(),
                suite: "pr".to_string(),
                smoke: false,
            },
            scenarios,
        }
    }

    #[test]
    fn results_json_round_trip() {
        let results = sample_results();
        let json = serde_json::to_string_pretty(&results).unwrap();
        let parsed: Results = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, results);
    }

    #[test]
    fn markdown_table_snapshot() {
        let expected = "\
| scenario | median (ms) | min (ms) | budget (ms) | baseline (ms) | rss (MB) | status |
|---|---:|---:|---:|---:|---:|---|
| map_cached | 123.5 | 120.0 | 500.0 | 110.0 | 50.0 | pass |
| sql_v1_query | - | - | 500.0 | - | - | skip (ctx binary built without the duckdb feature) |
";
        assert_eq!(markdown_table(&sample_results()), expected);
    }
}
