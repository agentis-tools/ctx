//! Code analysis commands.
//!
//! Handles complexity analysis and code audits. Near-duplicate detection
//! lives in [`crate::commands::duplicates`].

use std::env;

use ctx::analytics;
use ctx::audit::{run_audit as do_audit, AuditConfig};
use ctx::error::Result;
use ctx::index;
use ctx::utils::{truncate_path, truncate_str};

/// Analyze code complexity and flag high fan-out functions.
pub fn run_complexity(threshold: i64, warnings_only: bool, output: &str) -> Result<()> {
    let root = env::current_dir()?;
    let analytics = analytics::Analytics::open(&root)?;

    let results = analytics.complexity_analysis(threshold)?;

    if results.is_empty() {
        println!("No functions found.");
        return Ok(());
    }

    // Filter to only warnings if requested
    let results: Vec<_> = if warnings_only {
        results
            .into_iter()
            .filter(|r| r.fan_out >= threshold)
            .collect()
    } else {
        results
    };

    if output == "json" {
        let json_results: Vec<_> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "file": r.file_path,
                    "line": r.line,
                    "fan_out": r.fan_out,
                    "fan_in": r.fan_in,
                    "complexity_score": r.complexity_score,
                    "severity": r.severity,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_results)?);
    } else {
        println!("Code Complexity Analysis (threshold: {})", threshold);
        println!("{}", "=".repeat(90));
        println!(
            "{:<35} {:>8} {:>8} {:>8} {:<10} FILE",
            "FUNCTION", "FAN-OUT", "FAN-IN", "SCORE", "SEVERITY"
        );
        println!("{}", "-".repeat(90));

        for result in &results {
            let name = truncate_str(&result.name, 33);
            let file = truncate_path(&result.file_path, 20);

            let severity_marker = match result.severity.as_str() {
                "critical" => "🔴 CRITICAL",
                "high" => "🟠 HIGH",
                "medium" => "🟡 MEDIUM",
                _ => "🟢 LOW",
            };

            println!(
                "{:<35} {:>8} {:>8} {:>8} {:<10} {}:{}",
                name,
                result.fan_out,
                result.fan_in,
                result.complexity_score,
                severity_marker,
                file,
                result.line
            );
        }

        // Summary
        let critical = results.iter().filter(|r| r.severity == "critical").count();
        let high = results.iter().filter(|r| r.severity == "high").count();

        println!("{}", "-".repeat(90));
        println!("Total: {} functions analyzed", results.len());
        if critical > 0 || high > 0 {
            println!(
                "⚠️  {} critical, {} high complexity functions need attention",
                critical, high
            );
        }
    }

    Ok(())
}

/// Run code quality audit.
pub fn run_audit(
    format: &str,
    min_score: Option<f32>,
    categories: Option<String>,
    incremental: bool,
) -> Result<()> {
    let root = env::current_dir()?;

    // Open database
    let db = index::open_database(&root).map_err(|_| {
        "No index found. Run 'ctx index' first to build the code intelligence database."
    })?;

    // Open analytics (optional, provides complexity analysis)
    let analytics = analytics::Analytics::open(&root).ok();

    // Parse categories if provided
    let category_list = categories
        .as_ref()
        .map(|c| c.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    // Build config
    let config = AuditConfig {
        categories: category_list,
        path: root.clone(),
        incremental,
        min_score,
    };

    if incremental {
        eprintln!("Running incremental code quality audit (changed files only)...\n");
    } else {
        eprintln!("Running code quality audit...\n");
    }

    // Run audit
    let report = do_audit(&db, analytics.as_ref(), &config)?;

    // Output in requested format
    match format {
        "json" => {
            let json = report.format_json()?;
            println!("{}", json);
        }
        "markdown" | "md" => {
            println!("{}", report.format_markdown());
        }
        _ => {
            // Default: text
            println!("{}", report.format_text());
        }
    }

    // Exit with non-zero if below threshold
    if !report.passed {
        eprintln!(
            "\nAudit failed: score {:.1} below threshold {:.1}",
            report.overall_score,
            report.threshold.unwrap_or(0.0)
        );
        std::process::exit(1);
    }

    Ok(())
}
