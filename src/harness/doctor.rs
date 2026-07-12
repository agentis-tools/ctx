//! `ctx harness doctor` -- integration diagnostics.
//!
//! Runs a set of independent checks over the project root and returns
//! findings; nothing here exits or prints. Severity semantics:
//!
//! - `error` / `warning`: something needs fixing -> exit code 1 at the CLI
//! - `info`: informational only -> does not affect the exit code
//!
//! Checks never cascade: a missing index is a *finding* (with a hint), not
//! an operational error, and the remaining checks still run.

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::Serialize;

use super::checksum::{content_checksum, generated_version, recorded_checksum};
use super::templates::CTX_VERSION;
use super::{read_lock, HOOK_NAMES, LOCAL_HOOKS_DIR, RULES_PATH};

const CODEX_LOCAL_HOOKS_DIR: &str = ".codex/hooks/ctx";
use crate::db::SCHEMA_VERSION;
use crate::walker::{discover_files, WalkerConfig};

/// Severity of a doctor finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// One doctor finding.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub severity: Severity,
    /// Stable machine-readable identifier (e.g. `index_missing`).
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Finding {
    fn new(severity: Severity, code: &'static str, message: String, hint: Option<&str>) -> Self {
        Finding {
            severity,
            code,
            message,
            hint: hint.map(String::from),
        }
    }
}

/// Run every doctor check against `root` and collect the findings.
pub fn run_doctor_checks(root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    check_binary_version(&mut findings);
    check_scaffold(root, &mut findings);
    check_templates_stale(root, &mut findings);
    check_index(root, &mut findings);
    check_rules(root, &mut findings);
    check_hooks(root, &mut findings);
    check_settings_wiring(root, &mut findings);
    check_mcp(root, &mut findings);

    findings
}

/// True when no finding is an error or a warning.
pub fn is_healthy(findings: &[Finding]) -> bool {
    findings
        .iter()
        .all(|f| matches!(f.severity, Severity::Info))
}

// ============================================================================
// Individual checks
// ============================================================================

fn check_binary_version(findings: &mut Vec<Finding>) {
    let mcp = if cfg!(feature = "mcp") {
        "compiled in"
    } else {
        "not compiled in"
    };
    findings.push(Finding::new(
        Severity::Info,
        "binary_version",
        format!("ctx v{CTX_VERSION} (mcp feature: {mcp})"),
        None,
    ));
}

fn local_scaffold(root: &Path) -> bool {
    root.join(LOCAL_HOOKS_DIR).is_dir()
}

fn plugin_scaffold(root: &Path) -> bool {
    root.join(".claude-plugin/plugin.json").exists()
}

fn codex_local_scaffold(root: &Path) -> bool {
    root.join(CODEX_LOCAL_HOOKS_DIR).is_dir()
}

fn codex_plugin_scaffold(root: &Path) -> bool {
    root.join(".codex-plugin/plugin.json").exists()
}

fn check_scaffold(root: &Path, findings: &mut Vec<Finding>) {
    if !local_scaffold(root)
        && !plugin_scaffold(root)
        && !codex_local_scaffold(root)
        && !codex_plugin_scaffold(root)
        && read_lock(root).is_none()
    {
        findings.push(Finding::new(
            Severity::Info,
            "harness_not_initialized",
            "no harness files found (nothing scaffolded)".to_string(),
            Some("run 'ctx harness init --target claude|codex' to wire ctx into an agent"),
        ));
    }
}

fn check_templates_stale(root: &Path, findings: &mut Vec<Finding>) {
    let mut stale: Vec<String> = Vec::new();

    // Manifest entries record the generating version for every file,
    // including JSON files without in-file headers.
    if let Some(lock) = read_lock(root) {
        for (rel, entry) in &lock.files {
            if entry.ctx_version != CTX_VERSION && root.join(rel).exists() {
                stale.push(format!("{rel} (v{})", entry.ctx_version));
            }
        }
    }

    // In-file headers cover files generated before the manifest existed (or
    // with a deleted manifest).
    for dir in [LOCAL_HOOKS_DIR, CODEX_LOCAL_HOOKS_DIR, "hooks"] {
        for name in HOOK_NAMES {
            let rel = format!("{dir}/{name}.sh");
            if stale.iter().any(|s| s.starts_with(&rel)) {
                continue;
            }
            if let Ok(content) = fs::read_to_string(root.join(&rel)) {
                if let Some(version) = generated_version(&content) {
                    if version != CTX_VERSION {
                        stale.push(format!("{rel} (v{version})"));
                    }
                }
            }
        }
    }

    if !stale.is_empty() {
        findings.push(Finding::new(
            Severity::Warning,
            "templates_stale",
            format!(
                "generated files are from a different ctx version than v{CTX_VERSION}: {}",
                stale.join(", ")
            ),
            Some("rerun 'ctx harness init' to regenerate them"),
        ));
    }
}

fn check_index(root: &Path, findings: &mut Vec<Finding>) {
    let db_path = root.join(".ctx/codebase.sqlite");
    if !db_path.exists() {
        findings.push(Finding::new(
            Severity::Warning,
            "index_missing",
            "no code intelligence index (.ctx/codebase.sqlite)".to_string(),
            Some("run 'ctx index' to build it"),
        ));
        return;
    }

    // Read the schema version and freshness directly (read-only) so a
    // schema mismatch is a finding, not a hard failure.
    let conn = match rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(conn) => conn,
        Err(e) => {
            findings.push(Finding::new(
                Severity::Error,
                "index_schema",
                format!("cannot open .ctx/codebase.sqlite: {e}"),
                Some("run 'ctx index --force' to rebuild the index"),
            ));
            return;
        }
    };

    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(-1);
    if user_version != SCHEMA_VERSION {
        findings.push(Finding::new(
            Severity::Error,
            "index_schema",
            format!("index schema version is {user_version}, this ctx expects {SCHEMA_VERSION}"),
            Some("run 'ctx index --force' to rebuild the index"),
        ));
        return;
    }

    // Freshness: any source file modified after the newest index entry?
    let last_indexed: Option<i64> = conn
        .query_row("SELECT MAX(last_indexed) FROM files", [], |row| row.get(0))
        .unwrap_or(None);
    let Some(last_indexed) = last_indexed else {
        return; // empty index; index_missing semantics don't apply
    };
    let Ok(entries) = discover_files(root, &WalkerConfig::default()) else {
        return;
    };
    let stale = entries
        .iter()
        .filter(|entry| {
            entry
                .absolute_path
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64 > last_indexed)
                .unwrap_or(false)
        })
        .count();
    if stale > 0 {
        findings.push(Finding::new(
            Severity::Warning,
            "index_stale",
            format!(
                "{stale} file{} modified after the last index update",
                if stale == 1 { "" } else { "s" }
            ),
            Some("run 'ctx index' to refresh"),
        ));
    }
}

fn check_rules(root: &Path, findings: &mut Vec<Finding>) {
    let rules_path = root.join(RULES_PATH);
    let content = match fs::read_to_string(&rules_path) {
        Ok(content) => content,
        Err(_) => {
            findings.push(Finding::new(
                Severity::Info,
                "rules_missing",
                format!("no rules file ({RULES_PATH})"),
                Some("'ctx harness init' writes a commented starter; see 'ctx check --help'"),
            ));
            return;
        }
    };

    let compiled = toml::from_str::<crate::rules::RulesFile>(&content)
        .map_err(|e| e.to_string())
        .and_then(|file| crate::rules::CompiledRules::compile(file).map_err(|e| e.to_string()));
    if let Err(e) = compiled {
        findings.push(Finding::new(
            Severity::Error,
            "rules_invalid",
            format!("{RULES_PATH} is invalid: {}", e.trim()),
            Some("fix the rules file; 'ctx check --help' documents the format"),
        ));
    }
}

fn check_hooks(root: &Path, findings: &mut Vec<Finding>) {
    let mut dirs: Vec<&str> = Vec::new();
    if local_scaffold(root) {
        dirs.push(LOCAL_HOOKS_DIR);
    }
    if plugin_scaffold(root) {
        dirs.push("hooks");
    }
    if codex_local_scaffold(root) {
        dirs.push(CODEX_LOCAL_HOOKS_DIR);
    }
    if codex_plugin_scaffold(root) && !dirs.contains(&"hooks") {
        dirs.push("hooks");
    }

    let lock = read_lock(root);
    let mut missing: Vec<String> = Vec::new();
    let mut modified: Vec<String> = Vec::new();

    for dir in dirs {
        for name in HOOK_NAMES {
            let rel = format!("{dir}/{name}.sh");
            let path = root.join(&rel);
            let Ok(bytes) = fs::read(&path) else {
                missing.push(rel);
                continue;
            };
            let actual = content_checksum(&bytes);
            let expected = lock
                .as_ref()
                .and_then(|l| l.files.get(&rel))
                .map(|e| {
                    e.checksum
                        .strip_prefix("sha256:")
                        .unwrap_or(&e.checksum)
                        .to_string()
                })
                .or_else(|| std::str::from_utf8(&bytes).ok().and_then(recorded_checksum));
            match expected {
                Some(expected) if expected == actual => {}
                _ => modified.push(rel),
            }
        }
    }

    if !missing.is_empty() {
        findings.push(Finding::new(
            Severity::Warning,
            "hooks_missing",
            format!("hook scripts missing: {}", missing.join(", ")),
            Some("rerun 'ctx harness init' to regenerate them"),
        ));
    }
    if !modified.is_empty() {
        findings.push(Finding::new(
            Severity::Warning,
            "hooks_modified",
            format!(
                "hook scripts modified since generation (checksum mismatch): {}",
                modified.join(", ")
            ),
            Some("rerun 'ctx harness init --force' to restore them"),
        ));
    }
}

fn check_settings_wiring(root: &Path, findings: &mut Vec<Finding>) {
    check_codex_wiring(root, findings);
    if !local_scaffold(root) {
        return;
    }
    let wired = fs::read_to_string(root.join(".claude/settings.json"))
        .map(|content| content.contains(".claude/hooks/ctx/"))
        .unwrap_or(false);
    if !wired {
        findings.push(Finding::new(
            Severity::Warning,
            "settings_not_wired",
            ".claude/settings.json does not reference the generated hooks under .claude/hooks/ctx/"
                .to_string(),
            Some("add the settings snippet printed by 'ctx harness init' to .claude/settings.json"),
        ));
    }
}

fn check_codex_wiring(root: &Path, findings: &mut Vec<Finding>) {
    if !codex_local_scaffold(root) {
        return;
    }
    let path = root.join(".codex/hooks.json");
    let wired = fs::read_to_string(&path)
        .ok()
        .and_then(|content| {
            serde_json::from_str::<serde_json::Value>(&content)
                .ok()
                .map(|_| content)
        })
        .map(|content| content.contains(".codex/hooks/ctx/"))
        .unwrap_or(false);
    if !wired {
        findings.push(Finding::new(
            Severity::Warning,
            "codex_hooks_not_wired",
            ".codex/hooks.json is missing, invalid, or does not reference .codex/hooks/ctx/"
                .to_string(),
            Some("rerun 'ctx harness init --target codex' to regenerate it"),
        ));
    }
}

fn check_mcp(root: &Path, findings: &mut Vec<Finding>) {
    let mcp_json = fs::read_to_string(root.join(".mcp.json")).ok();
    let wires_ctx_serve = mcp_json
        .as_deref()
        .map(|content| content.contains("\"ctx\"") && content.contains("--mcp"))
        .unwrap_or(false);

    if wires_ctx_serve && !cfg!(feature = "mcp") {
        findings.push(Finding::new(
            Severity::Warning,
            "mcp_unavailable",
            ".mcp.json wires 'ctx serve --mcp' but this binary lacks the mcp feature".to_string(),
            Some("install a build with MCP: cargo install agentis-ctx --features mcp"),
        ));
    }

    if cfg!(feature = "mcp")
        && (plugin_scaffold(root) || codex_plugin_scaffold(root))
        && mcp_json.is_none()
    {
        findings.push(Finding::new(
            Severity::Info,
            "mcp_not_wired",
            "this binary has the mcp feature, but the plugin scaffold has no .mcp.json".to_string(),
            Some("rerun 'ctx harness init --mode plugin' to generate it"),
        ));
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn codes(findings: &[Finding]) -> Vec<&'static str> {
        findings.iter().map(|f| f.code).collect()
    }

    fn find<'a>(findings: &'a [Finding], code: &str) -> &'a Finding {
        findings
            .iter()
            .find(|f| f.code == code)
            .unwrap_or_else(|| panic!("no finding {code} in {:?}", codes(findings)))
    }

    #[test]
    fn test_empty_dir_reports_missing_index_and_not_initialized() {
        let temp = TempDir::new().unwrap();
        let findings = run_doctor_checks(temp.path());

        assert_eq!(find(&findings, "binary_version").severity, Severity::Info);
        let missing = find(&findings, "index_missing");
        assert_eq!(missing.severity, Severity::Warning);
        assert!(missing.hint.as_deref().unwrap().contains("ctx index"));
        assert_eq!(
            find(&findings, "harness_not_initialized").severity,
            Severity::Info
        );
        assert_eq!(find(&findings, "rules_missing").severity, Severity::Info);
        assert!(!is_healthy(&findings), "index_missing is a warning");
    }

    #[test]
    fn test_invalid_rules_is_a_distinct_error_finding() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join(".ctx")).unwrap();
        std::fs::write(temp.path().join(RULES_PATH), "[layers\nbroken = [").unwrap();

        let findings = run_doctor_checks(temp.path());
        let invalid = find(&findings, "rules_invalid");
        assert_eq!(invalid.severity, Severity::Error);
        assert!(invalid.message.contains(".ctx/rules.toml"));
    }

    #[test]
    fn test_codex_scaffold_checks_hook_wiring() {
        let temp = TempDir::new().unwrap();
        let plan = super::super::plan_codex_local(temp.path());
        super::super::write_plan(temp.path(), &plan, false).unwrap();

        let findings = run_doctor_checks(temp.path());
        assert!(!codes(&findings).contains(&"codex_hooks_not_wired"));

        std::fs::write(temp.path().join(".codex/hooks.json"), "{}\n").unwrap();
        let findings = run_doctor_checks(temp.path());
        assert_eq!(
            find(&findings, "codex_hooks_not_wired").severity,
            Severity::Warning
        );
    }

    #[test]
    fn test_index_missing_and_rules_invalid_are_reported_simultaneously() {
        // No cascading: both findings, with distinct codes, in one run.
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join(".ctx")).unwrap();
        std::fs::write(temp.path().join(RULES_PATH), "not toml at all [[[").unwrap();

        let findings = run_doctor_checks(temp.path());
        let all = codes(&findings);
        assert!(all.contains(&"index_missing"), "codes: {all:?}");
        assert!(all.contains(&"rules_invalid"), "codes: {all:?}");
        assert_ne!("index_missing", "rules_invalid");
        assert!(!is_healthy(&findings));
    }

    #[test]
    fn test_healthy_scaffold_reports_only_info() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Full local scaffold + wired settings + fresh index.
        let plan = super::super::plan_local(root);
        super::super::write_plan(root, &plan, false).unwrap();
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(
            root.join(".claude/settings.json"),
            "{\"hooks\": {\"SessionStart\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"\\\"$CLAUDE_PROJECT_DIR\\\"/.claude/hooks/ctx/session-start.sh\"}]}]}}",
        )
        .unwrap();
        std::fs::write(root.join("src.rs"), "fn main() {}\n").unwrap();
        let mut indexer =
            crate::index::Indexer::with_config(root, false, WalkerConfig::default()).unwrap();
        indexer.index().unwrap();

        // The generated files postdate the index; refresh once more so the
        // staleness check sees a fresh index. (harness files live in dot
        // dirs the walker skips, but src.rs was just written.)
        let mut indexer =
            crate::index::Indexer::with_config(root, false, WalkerConfig::default()).unwrap();
        indexer.index().unwrap();

        let findings = run_doctor_checks(root);
        for finding in &findings {
            assert_eq!(
                finding.severity,
                Severity::Info,
                "unexpected non-info finding: {:?}",
                finding
            );
        }
        assert!(is_healthy(&findings));
    }

    #[test]
    fn test_modified_hook_is_flagged() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let plan = super::super::plan_local(root);
        super::super::write_plan(root, &plan, false).unwrap();

        let stop = root.join(".claude/hooks/ctx/stop.sh");
        let content = std::fs::read_to_string(&stop).unwrap() + "echo extra\n";
        std::fs::write(&stop, content).unwrap();
        std::fs::remove_file(root.join(".claude/hooks/ctx/session-start.sh")).unwrap();

        let findings = run_doctor_checks(root);
        let modified = find(&findings, "hooks_modified");
        assert!(modified.message.contains("stop.sh"));
        let missing = find(&findings, "hooks_missing");
        assert!(missing.message.contains("session-start.sh"));
    }

    #[test]
    fn test_severity_serializes_lowercase() {
        let finding = Finding::new(Severity::Warning, "index_missing", "msg".into(), Some("h"));
        let value = serde_json::to_value(&finding).unwrap();
        assert_eq!(value["severity"], "warning");
        assert_eq!(value["code"], "index_missing");
        assert_eq!(value["hint"], "h");
    }
}
