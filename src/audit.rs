//! Code quality audit module.
//!
//! Provides automated code quality analysis with scoring for CI integration.
//! Analyzes complexity, duplication potential, documentation coverage,
//! modularity, and naming conventions.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::analytics::Analytics;
use crate::db::models::{Symbol, Visibility};
use crate::db::Database;

/// Error type for audit operations.
#[derive(Debug)]
pub enum AuditError {
    Database(String),
    Analytics(String),
}

impl std::fmt::Display for AuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditError::Database(msg) => write!(f, "Database error: {}", msg),
            AuditError::Analytics(msg) => write!(f, "Analytics error: {}", msg),
        }
    }
}

impl std::error::Error for AuditError {}

impl From<rusqlite::Error> for AuditError {
    fn from(e: rusqlite::Error) -> Self {
        AuditError::Database(e.to_string())
    }
}

impl From<duckdb::Error> for AuditError {
    fn from(e: duckdb::Error) -> Self {
        AuditError::Analytics(e.to_string())
    }
}

/// Configuration for audit analysis.
#[derive(Debug, Clone)]
pub struct AuditConfig {
    /// Categories to analyze (empty = all)
    pub categories: Vec<String>,
    /// Path to audit
    pub path: PathBuf,
    /// Only audit changed files
    pub incremental: bool,
    /// Minimum score threshold
    pub min_score: Option<f32>,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            categories: vec![
                "complexity".to_string(),
                "duplication".to_string(),
                "coverage".to_string(),
                "modularity".to_string(),
                "naming".to_string(),
            ],
            path: PathBuf::from("."),
            incremental: false,
            min_score: None,
        }
    }
}

/// Severity level for quality issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A quality issue found during audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityIssue {
    /// Severity level
    pub severity: Severity,
    /// Category this issue belongs to
    pub category: String,
    /// File path where issue was found
    pub file: String,
    /// Line number (if applicable)
    pub line: Option<u32>,
    /// Issue description
    pub message: String,
    /// Suggested fix (if applicable)
    pub suggestion: Option<String>,
}

/// Score for a single category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryScore {
    /// Category name
    pub name: String,
    /// Score (0.0-10.0)
    pub score: f32,
    /// Number of issues found
    pub issue_count: usize,
    /// Weight for overall calculation
    pub weight: f32,
}

/// Complete quality report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityReport {
    /// Overall score (0.0-10.0)
    pub overall_score: f32,
    /// Whether the audit passed (score >= threshold)
    pub passed: bool,
    /// Score threshold (if specified)
    pub threshold: Option<f32>,
    /// Scores by category
    pub categories: Vec<CategoryScore>,
    /// Issues found
    pub issues: Vec<QualityIssue>,
    /// Total symbol count
    pub total_symbols: usize,
    /// Total function count
    pub total_functions: usize,
}

impl QualityReport {
    /// Create a new empty report.
    pub fn new() -> Self {
        Self {
            overall_score: 0.0,
            passed: true,
            threshold: None,
            categories: Vec::new(),
            issues: Vec::new(),
            total_symbols: 0,
            total_functions: 0,
        }
    }

    /// Add a category score.
    pub fn add_category(&mut self, score: CategoryScore) {
        self.categories.push(score);
    }

    /// Add an issue.
    pub fn add_issue(&mut self, issue: QualityIssue) {
        self.issues.push(issue);
    }

    /// Calculate the overall score from category scores.
    pub fn calculate_overall(&mut self) {
        let total_weight: f32 = self.categories.iter().map(|c| c.weight).sum();
        if total_weight > 0.0 {
            let weighted_sum: f32 = self.categories.iter().map(|c| c.score * c.weight).sum();
            self.overall_score = weighted_sum / total_weight;
        }

        // Check threshold
        if let Some(threshold) = self.threshold {
            self.passed = self.overall_score >= threshold;
        }
    }

    /// Get issues by severity.
    pub fn issues_by_severity(&self, severity: Severity) -> Vec<&QualityIssue> {
        self.issues
            .iter()
            .filter(|i| i.severity == severity)
            .collect()
    }

    /// Format as text output.
    pub fn format_text(&self) -> String {
        let mut output = String::new();

        output.push_str("Code Quality Audit\n");
        output.push_str("==================\n\n");

        output.push_str(&format!("Overall Score: {:.1}/10\n\n", self.overall_score));

        // Categories
        output.push_str("Categories:\n");
        for cat in &self.categories {
            output.push_str(&format!(
                "  {:12} {:.1}/10  ({} issues)\n",
                format!("{}:", capitalize(&cat.name)),
                cat.score,
                cat.issue_count
            ));
        }
        output.push('\n');

        // Critical issues
        let critical = self.issues_by_severity(Severity::Critical);
        if !critical.is_empty() {
            output.push_str(&format!("Critical Issues ({}):\n", critical.len()));
            for issue in critical.iter().take(5) {
                if let Some(line) = issue.line {
                    output.push_str(&format!(
                        "  [CRIT] {}:{} - {}\n",
                        issue.file, line, issue.message
                    ));
                } else {
                    output.push_str(&format!("  [CRIT] {} - {}\n", issue.file, issue.message));
                }
            }
            if critical.len() > 5 {
                output.push_str(&format!("  ... and {} more\n", critical.len() - 5));
            }
            output.push('\n');
        }

        // Warnings
        let warnings = self.issues_by_severity(Severity::Warning);
        if !warnings.is_empty() {
            output.push_str(&format!("Warnings ({}):\n", warnings.len()));
            for issue in warnings.iter().take(5) {
                if let Some(line) = issue.line {
                    output.push_str(&format!(
                        "  [WARN] {}:{} - {}\n",
                        issue.file, line, issue.message
                    ));
                } else {
                    output.push_str(&format!("  [WARN] {} - {}\n", issue.file, issue.message));
                }
            }
            if warnings.len() > 5 {
                output.push_str(&format!("  ... and {} more\n", warnings.len() - 5));
            }
            output.push('\n');
        }

        // Threshold result
        if let Some(threshold) = self.threshold {
            if self.passed {
                output.push_str(&format!(
                    "✓ Score {:.1} meets threshold {:.1}\n",
                    self.overall_score, threshold
                ));
            } else {
                output.push_str(&format!(
                    "✗ Score {:.1} below threshold {:.1}\n",
                    self.overall_score, threshold
                ));
            }
        }

        output
    }

    /// Format as markdown output.
    pub fn format_markdown(&self) -> String {
        let mut output = String::new();

        output.push_str("# Code Quality Audit\n\n");

        // Summary
        output.push_str(&format!(
            "**Overall Score: {:.1}/10**\n\n",
            self.overall_score
        ));

        if let Some(threshold) = self.threshold {
            if self.passed {
                output.push_str(&format!("✅ Passed (threshold: {:.1})\n\n", threshold));
            } else {
                output.push_str(&format!("❌ Failed (threshold: {:.1})\n\n", threshold));
            }
        }

        // Categories table
        output.push_str("## Categories\n\n");
        output.push_str("| Category | Score | Issues | Weight |\n");
        output.push_str("|----------|-------|--------|--------|\n");
        for cat in &self.categories {
            output.push_str(&format!(
                "| {} | {:.1}/10 | {} | {:.0}% |\n",
                capitalize(&cat.name),
                cat.score,
                cat.issue_count,
                cat.weight * 100.0
            ));
        }
        output.push('\n');

        // Issues by severity
        let critical = self.issues_by_severity(Severity::Critical);
        if !critical.is_empty() {
            output.push_str("## Critical Issues\n\n");
            for issue in &critical {
                if let Some(line) = issue.line {
                    output.push_str(&format!(
                        "- **{}:{}** - {}\n",
                        issue.file, line, issue.message
                    ));
                } else {
                    output.push_str(&format!("- **{}** - {}\n", issue.file, issue.message));
                }
                if let Some(ref suggestion) = issue.suggestion {
                    output.push_str(&format!("  - *Suggestion: {}*\n", suggestion));
                }
            }
            output.push('\n');
        }

        let warnings = self.issues_by_severity(Severity::Warning);
        if !warnings.is_empty() {
            output.push_str("## Warnings\n\n");
            for issue in warnings.iter().take(20) {
                if let Some(line) = issue.line {
                    output.push_str(&format!(
                        "- **{}:{}** - {}\n",
                        issue.file, line, issue.message
                    ));
                } else {
                    output.push_str(&format!("- **{}** - {}\n", issue.file, issue.message));
                }
            }
            if warnings.len() > 20 {
                output.push_str(&format!(
                    "\n*... and {} more warnings*\n",
                    warnings.len() - 20
                ));
            }
            output.push('\n');
        }

        // Statistics
        output.push_str("## Statistics\n\n");
        output.push_str(&format!("- Total symbols: {}\n", self.total_symbols));
        output.push_str(&format!("- Total functions: {}\n", self.total_functions));
        output.push_str(&format!(
            "- Issues: {} critical, {} warnings\n",
            critical.len(),
            warnings.len()
        ));

        output
    }

    /// Format as JSON output.
    pub fn format_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

impl Default for QualityReport {
    fn default() -> Self {
        Self::new()
    }
}

/// Category weights for overall score calculation.
const WEIGHT_COMPLEXITY: f32 = 0.25;
const WEIGHT_DUPLICATION: f32 = 0.20;
const WEIGHT_COVERAGE: f32 = 0.20;
const WEIGHT_MODULARITY: f32 = 0.20;
const WEIGHT_NAMING: f32 = 0.15;

/// Run a complete quality audit.
pub fn run_audit(
    db: &Database,
    analytics: Option<&Analytics>,
    config: &AuditConfig,
) -> Result<QualityReport, AuditError> {
    let mut report = QualityReport::new();
    report.threshold = config.min_score;

    // Get all symbols using a broad search
    // We use "%" pattern to get all symbols with a high limit
    let symbols = db.find_symbols("%", 100000)?;
    report.total_symbols = symbols.len();
    report.total_functions = symbols
        .iter()
        .filter(|s| s.kind.as_str() == "function" || s.kind.as_str() == "method")
        .count();

    let should_run = |cat: &str| -> bool {
        config.categories.is_empty() || config.categories.iter().any(|c| c == cat)
    };

    // Complexity analysis
    if should_run("complexity") {
        let (score, issues) = if let Some(a) = analytics {
            score_complexity(a, &symbols)?
        } else {
            (8.0, Vec::new()) // Default good score if no analytics
        };
        report.add_category(CategoryScore {
            name: "complexity".to_string(),
            score,
            issue_count: issues.len(),
            weight: WEIGHT_COMPLEXITY,
        });
        for issue in issues {
            report.add_issue(issue);
        }
    }

    // Duplication analysis (simplified - based on similar function names)
    if should_run("duplication") {
        let (score, issues) = score_duplication(&symbols);
        report.add_category(CategoryScore {
            name: "duplication".to_string(),
            score,
            issue_count: issues.len(),
            weight: WEIGHT_DUPLICATION,
        });
        for issue in issues {
            report.add_issue(issue);
        }
    }

    // Documentation coverage
    if should_run("coverage") {
        let (score, issues) = score_coverage(&symbols);
        report.add_category(CategoryScore {
            name: "coverage".to_string(),
            score,
            issue_count: issues.len(),
            weight: WEIGHT_COVERAGE,
        });
        for issue in issues {
            report.add_issue(issue);
        }
    }

    // Modularity analysis
    if should_run("modularity") {
        let (score, issues) = if let Some(a) = analytics {
            score_modularity(a)?
        } else {
            (8.0, Vec::new())
        };
        report.add_category(CategoryScore {
            name: "modularity".to_string(),
            score,
            issue_count: issues.len(),
            weight: WEIGHT_MODULARITY,
        });
        for issue in issues {
            report.add_issue(issue);
        }
    }

    // Naming conventions
    if should_run("naming") {
        let (score, issues) = score_naming(&symbols);
        report.add_category(CategoryScore {
            name: "naming".to_string(),
            score,
            issue_count: issues.len(),
            weight: WEIGHT_NAMING,
        });
        for issue in issues {
            report.add_issue(issue);
        }
    }

    report.calculate_overall();
    Ok(report)
}

/// Score complexity based on fan-out metrics.
fn score_complexity(
    analytics: &Analytics,
    _symbols: &[Symbol],
) -> Result<(f32, Vec<QualityIssue>), AuditError> {
    let mut issues = Vec::new();

    // Get complexity data from analytics
    let complexity_results = analytics.complexity_analysis(20)?;

    // Count high-complexity functions
    let critical_count = complexity_results
        .iter()
        .filter(|r| r.severity == "critical")
        .count();
    let high_count = complexity_results
        .iter()
        .filter(|r| r.severity == "high")
        .count();
    let medium_count = complexity_results
        .iter()
        .filter(|r| r.severity == "medium")
        .count();

    // Generate issues for high-complexity functions
    for result in complexity_results
        .iter()
        .filter(|r| r.severity == "critical" || r.severity == "high")
    {
        issues.push(QualityIssue {
            severity: if result.severity == "critical" {
                Severity::Critical
            } else {
                Severity::Warning
            },
            category: "complexity".to_string(),
            file: result.file_path.clone(),
            line: Some(result.line),
            message: format!(
                "{}: fan-out {} (threshold: 20)",
                result.name, result.fan_out
            ),
            suggestion: Some("Extract helper functions to reduce complexity".to_string()),
        });
    }

    // Calculate score
    let score = calculate_complexity_score(critical_count, high_count, medium_count);

    Ok((score, issues))
}

/// Calculate complexity score based on issue counts.
fn calculate_complexity_score(critical: usize, high: usize, medium: usize) -> f32 {
    // Score based on severity distribution
    if critical > 0 {
        // Critical issues significantly reduce score
        (4.0 - (critical as f32 * 0.5).min(3.0)).max(1.0)
    } else if high > 5 {
        5.0 - ((high - 5) as f32 * 0.2).min(2.0)
    } else if high > 0 {
        6.0 - (high as f32 * 0.2)
    } else if medium > 10 {
        7.0 - ((medium - 10) as f32 * 0.1).min(1.0)
    } else if medium > 0 {
        8.0 - (medium as f32 * 0.1)
    } else {
        10.0
    }
}

/// Score duplication by looking for similar function names.
fn score_duplication(symbols: &[Symbol]) -> (f32, Vec<QualityIssue>) {
    use std::collections::HashMap;

    let mut issues = Vec::new();
    let mut name_counts: HashMap<String, Vec<&Symbol>> = HashMap::new();

    // Group functions by simplified name (without numeric suffixes)
    for symbol in symbols
        .iter()
        .filter(|s| s.kind.as_str() == "function" || s.kind.as_str() == "method")
    {
        // Remove numeric suffixes like _1, _2, etc.
        let base_name = symbol
            .name
            .trim_end_matches(|c: char| c.is_ascii_digit() || c == '_')
            .to_string();
        if base_name.len() >= 4 {
            name_counts.entry(base_name).or_default().push(symbol);
        }
    }

    // Find potential duplicates (3+ similar names)
    let duplicates: Vec<_> = name_counts
        .iter()
        .filter(|(_, syms)| syms.len() >= 3)
        .collect();

    for (name, syms) in &duplicates {
        if syms.len() >= 5 {
            issues.push(QualityIssue {
                severity: Severity::Warning,
                category: "duplication".to_string(),
                file: syms[0].file_path.clone(),
                line: Some(syms[0].line_start),
                message: format!(
                    "Potential code duplication: {} functions with similar name '{}'",
                    syms.len(),
                    name
                ),
                suggestion: Some("Consider extracting shared logic".to_string()),
            });
        }
    }

    // Calculate score based on duplicate patterns
    let score = if duplicates.is_empty() {
        10.0
    } else if duplicates.len() <= 2 {
        8.0
    } else if duplicates.len() <= 5 {
        6.0
    } else {
        4.0
    };

    (score, issues)
}

/// Score documentation coverage for public symbols.
fn score_coverage(symbols: &[Symbol]) -> (f32, Vec<QualityIssue>) {
    let mut issues = Vec::new();

    // Only check public symbols
    let public_symbols: Vec<_> = symbols
        .iter()
        .filter(|s| s.visibility == Visibility::Public)
        .collect();

    if public_symbols.is_empty() {
        return (10.0, issues);
    }

    // Count documented symbols (have brief or docstring)
    let documented_count = public_symbols
        .iter()
        .filter(|s| s.brief.is_some() || s.docstring.is_some())
        .count();

    let coverage = documented_count as f32 / public_symbols.len() as f32;

    // Report undocumented public functions/methods (limit to avoid spam)
    for symbol in public_symbols
        .iter()
        .filter(|s| {
            s.brief.is_none()
                && s.docstring.is_none()
                && (s.kind.as_str() == "function" || s.kind.as_str() == "method")
        })
        .take(10)
    {
        issues.push(QualityIssue {
            severity: Severity::Info,
            category: "coverage".to_string(),
            file: symbol.file_path.clone(),
            line: Some(symbol.line_start),
            message: format!(
                "Undocumented public {}: {}",
                symbol.kind.as_str(),
                symbol.name
            ),
            suggestion: Some("Add documentation comment".to_string()),
        });
    }

    // Calculate score based on coverage percentage
    let score = if coverage >= 0.95 {
        10.0
    } else if coverage >= 0.80 {
        8.0
    } else if coverage >= 0.60 {
        6.0
    } else if coverage >= 0.40 {
        4.0
    } else {
        2.0
    };

    (score, issues)
}

/// Score modularity based on file dependencies.
fn score_modularity(analytics: &Analytics) -> Result<(f32, Vec<QualityIssue>), AuditError> {
    let mut issues = Vec::new();

    // Get file dependencies
    let deps = analytics.file_dependencies()?;

    // Count cross-file dependencies
    let total_deps = deps.len();
    let external_deps = deps.iter().filter(|(_, t, _)| t == "external").count();

    // High external dependency ratio might indicate poor modularity
    let external_ratio = if total_deps > 0 {
        external_deps as f32 / total_deps as f32
    } else {
        0.0
    };

    // Look for files with too many outgoing dependencies
    use std::collections::HashMap;
    let mut file_dep_counts: HashMap<&str, usize> = HashMap::new();
    for (source, _, _) in &deps {
        *file_dep_counts.entry(source).or_default() += 1;
    }

    for (file, count) in file_dep_counts.iter().filter(|(_, &c)| c > 20) {
        issues.push(QualityIssue {
            severity: Severity::Warning,
            category: "modularity".to_string(),
            file: file.to_string(),
            line: None,
            message: format!("High coupling: {} outgoing dependencies", count),
            suggestion: Some("Consider splitting into smaller modules".to_string()),
        });
    }

    // Calculate score
    let score = if external_ratio > 0.5 {
        5.0
    } else if external_ratio > 0.3 {
        6.0
    } else if issues.is_empty() {
        9.0 - (external_ratio * 2.0)
    } else {
        7.0 - (issues.len() as f32 * 0.5).min(2.0)
    };

    Ok((score.max(2.0).min(10.0), issues))
}

/// Score naming convention consistency.
fn score_naming(symbols: &[Symbol]) -> (f32, Vec<QualityIssue>) {
    let mut issues = Vec::new();
    let mut violations = 0;

    for symbol in symbols {
        let name = &symbol.name;

        // Check naming conventions based on kind
        let is_valid = match symbol.kind.as_str() {
            "function" | "method" => is_snake_case(name),
            "struct" | "enum" | "class" | "interface" | "type" => is_pascal_case(name),
            "constant" => is_screaming_snake_case(name) || is_snake_case(name),
            _ => true,
        };

        if !is_valid {
            violations += 1;
            if violations <= 10 {
                issues.push(QualityIssue {
                    severity: Severity::Info,
                    category: "naming".to_string(),
                    file: symbol.file_path.clone(),
                    line: Some(symbol.line_start),
                    message: format!(
                        "{} '{}' doesn't follow naming convention",
                        symbol.kind.as_str(),
                        name
                    ),
                    suggestion: Some(suggest_name_fix(&symbol.kind.as_str(), name)),
                });
            }
        }
    }

    // Calculate score based on violation percentage
    let total = symbols.len();
    if total == 0 {
        return (10.0, issues);
    }

    let violation_rate = violations as f32 / total as f32;
    let score = if violation_rate <= 0.01 {
        10.0
    } else if violation_rate <= 0.05 {
        8.0
    } else if violation_rate <= 0.20 {
        6.0
    } else if violation_rate <= 0.40 {
        4.0
    } else {
        2.0
    };

    (score, issues)
}

/// Check if a name is snake_case.
fn is_snake_case(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    // Allow leading underscore for private
    let name = name.strip_prefix('_').unwrap_or(name);
    if name.is_empty() {
        return true;
    }

    // Must be lowercase with underscores
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Check if a name is PascalCase.
fn is_pascal_case(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    // First char must be uppercase
    let first = name.chars().next().unwrap();
    if !first.is_ascii_uppercase() {
        return false;
    }
    // No underscores (except for generic params like T_1)
    !name.contains('_') || name.chars().filter(|&c| c == '_').count() <= 1
}

/// Check if a name is SCREAMING_SNAKE_CASE.
fn is_screaming_snake_case(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    name.chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Suggest a fix for a naming convention violation.
fn suggest_name_fix(kind: &str, name: &str) -> String {
    match kind {
        "function" | "method" => format!("Use snake_case: {}", to_snake_case(name)),
        "struct" | "enum" | "class" | "interface" | "type" => {
            format!("Use PascalCase: {}", to_pascal_case(name))
        }
        "constant" => format!("Use SCREAMING_SNAKE_CASE: {}", name.to_uppercase()),
        _ => "Follow language naming conventions".to_string(),
    }
}

/// Convert a name to snake_case.
fn to_snake_case(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(c.to_ascii_lowercase());
    }
    result
}

/// Convert a name to PascalCase.
fn to_pascal_case(name: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;
    for c in name.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c.to_ascii_lowercase());
        }
    }
    result
}

/// Capitalize the first letter of a string.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snake_case() {
        assert!(is_snake_case("hello_world"));
        assert!(is_snake_case("_private"));
        assert!(is_snake_case("simple"));
        assert!(!is_snake_case("HelloWorld"));
        assert!(!is_snake_case("helloWorld"));
    }

    #[test]
    fn test_pascal_case() {
        assert!(is_pascal_case("HelloWorld"));
        assert!(is_pascal_case("Simple"));
        assert!(!is_pascal_case("hello_world"));
        assert!(!is_pascal_case("helloWorld"));
    }

    #[test]
    fn test_screaming_snake_case() {
        assert!(is_screaming_snake_case("HELLO_WORLD"));
        assert!(is_screaming_snake_case("SIMPLE"));
        assert!(!is_screaming_snake_case("hello_world"));
        assert!(!is_screaming_snake_case("HelloWorld"));
    }

    #[test]
    fn test_quality_report_format() {
        let mut report = QualityReport::new();
        report.add_category(CategoryScore {
            name: "complexity".to_string(),
            score: 7.5,
            issue_count: 5,
            weight: 0.25,
        });
        report.add_category(CategoryScore {
            name: "coverage".to_string(),
            score: 8.0,
            issue_count: 3,
            weight: 0.20,
        });
        report.calculate_overall();

        let text = report.format_text();
        assert!(text.contains("Code Quality Audit"));
        assert!(text.contains("Complexity:"));
        assert!(text.contains("Coverage:"));
    }

    #[test]
    fn test_calculate_overall_score() {
        let mut report = QualityReport::new();
        report.add_category(CategoryScore {
            name: "test1".to_string(),
            score: 8.0,
            issue_count: 0,
            weight: 0.5,
        });
        report.add_category(CategoryScore {
            name: "test2".to_string(),
            score: 6.0,
            issue_count: 0,
            weight: 0.5,
        });
        report.calculate_overall();

        assert!((report.overall_score - 7.0).abs() < 0.01);
    }

    #[test]
    fn test_threshold_pass() {
        let mut report = QualityReport::new();
        report.threshold = Some(7.0);
        report.add_category(CategoryScore {
            name: "test".to_string(),
            score: 8.0,
            issue_count: 0,
            weight: 1.0,
        });
        report.calculate_overall();

        assert!(report.passed);
    }

    #[test]
    fn test_threshold_fail() {
        let mut report = QualityReport::new();
        report.threshold = Some(7.0);
        report.add_category(CategoryScore {
            name: "test".to_string(),
            score: 6.0,
            issue_count: 0,
            weight: 1.0,
        });
        report.calculate_overall();

        assert!(!report.passed);
    }
}
