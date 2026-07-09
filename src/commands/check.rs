//! `ctx check` -- architecture rules CLI.
//!
//! Thin wrapper around the [`ctx::check`] engine: loads `.ctx/rules.toml`,
//! evaluates the rules against the index, and prints violations (human or
//! `--json`).
//!
//! Exit codes follow the ctx convention: 0 = no violations, 1 = at least one
//! violation, 2 = operational error (missing/invalid rules file, unknown
//! layer names, overlapping layers, missing index, bad git ref).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ctx::check::{collect_violations, load_context, CheckContext};
use ctx::error::Result;
use ctx::exit::Outcome;
use ctx::rules::Violation;

/// Run `ctx check` in the current directory.
pub fn run_check(
    rules_path: Option<PathBuf>,
    against: Option<String>,
    list: bool,
    json: bool,
) -> Result<Outcome> {
    let root = std::env::current_dir()?;
    run_check_in(&root, rules_path, against, list, json)
}

/// Dir-explicit implementation (used directly by tests).
fn run_check_in(
    root: &Path,
    rules_path: Option<PathBuf>,
    against: Option<String>,
    list: bool,
    json_mode: bool,
) -> Result<Outcome> {
    let context = load_context(root, rules_path)?;

    if list {
        return print_list(&context, json_mode);
    }

    let violations = collect_violations(root, &context, against.as_deref())?;
    let rules_violated = count_rule_groups(&violations);

    if json_mode {
        ctx::json::emit(
            "check",
            check_data(&context.rules_path, against.as_deref(), &violations),
        )?;
    } else {
        print_human(&violations, rules_violated);
    }

    Ok(if violations.is_empty() {
        Outcome::Clean
    } else {
        Outcome::Findings
    })
}

// ============================================================================
// Output
// ============================================================================

/// The `data` payload for `--json` mode (see docs/json-output.md, `check`).
fn check_data(
    rules_path: &Path,
    against: Option<&str>,
    violations: &[Violation],
) -> serde_json::Value {
    serde_json::json!({
        "rules_path": rules_path.display().to_string(),
        "against": against,
        "summary": {
            "violations": violations.len(),
            "rules_violated": count_rule_groups(violations),
        },
        "violations": violations,
    })
}

fn count_rule_groups(violations: &[Violation]) -> usize {
    let ids: HashSet<&str> = violations.iter().map(|v| v.rule_id.as_str()).collect();
    ids.len()
}

/// Human output: violations grouped by rule, one `file:line` + reason per
/// line, and a `N violations across M rules` summary.
fn print_human(violations: &[Violation], rules_violated: usize) {
    if violations.is_empty() {
        println!("No architecture violations found.");
        return;
    }

    let mut order: Vec<&str> = Vec::new();
    for v in violations {
        if !order.contains(&v.rule_id.as_str()) {
            order.push(&v.rule_id);
        }
    }

    for rule_id in &order {
        println!("{}", rule_id);
        for v in violations.iter().filter(|v| v.rule_id == *rule_id) {
            if v.message.contains(&v.reason) {
                println!("  {}", v.message);
            } else {
                println!("  {}  ({})", v.message, v.reason);
            }
        }
        println!();
    }

    println!(
        "{} violation{} across {} rule{}",
        violations.len(),
        if violations.len() == 1 { "" } else { "s" },
        rules_violated,
        if rules_violated == 1 { "" } else { "s" }
    );
}

/// `--list`: print the parsed rules and layer membership counts, exit 0.
fn print_list(context: &CheckContext, json_mode: bool) -> Result<Outcome> {
    let counts = context.compiled.layer_file_counts(&context.indexed);
    let groups = &context.compiled.file.rules;

    if json_mode {
        ctx::json::emit("check.list", list_data(context))?;
        return Ok(Outcome::Clean);
    }

    println!(
        "Rules file: {} (version {})",
        context.rules_path.display(),
        context.compiled.file.version
    );
    println!();

    println!("Layers:");
    if counts.is_empty() {
        println!("  (none)");
    }
    for (name, n) in &counts {
        println!(
            "  {:<16} {}  ({} file{})",
            name,
            context.compiled.file.layers[name].join(", "),
            n,
            if *n == 1 { "" } else { "s" }
        );
    }
    println!();

    println!("Rules:");
    let total = groups.forbidden.len()
        + groups.allowed_dependents.len()
        + groups.limit.len()
        + groups.no_new_dependents.len();
    if total == 0 {
        println!("  (none)");
    }
    for r in &groups.forbidden {
        let reason = r
            .reason
            .as_deref()
            .map(|s| format!("  ({})", s))
            .unwrap_or_default();
        println!("  forbidden: {} -> {}{}", r.from, r.to, reason);
    }
    for r in &groups.allowed_dependents {
        println!(
            "  allowed_dependents: only [{}] may depend on {}",
            r.only.join(", "),
            r.layer
        );
    }
    for r in &groups.limit {
        let exclude = if r.exclude.is_empty() {
            String::new()
        } else {
            format!("  (exclude [{}])", r.exclude.join(", "))
        };
        println!(
            "  limit: {} <= {} ({}){}",
            r.metric.as_str(),
            r.max,
            r.scope.as_str(),
            exclude
        );
    }
    for r in &groups.no_new_dependents {
        let reason = r
            .reason
            .as_deref()
            .map(|s| format!("  ({})", s))
            .unwrap_or_default();
        println!("  no_new_dependents: [{}]{}", r.paths.join(", "), reason);
    }

    Ok(Outcome::Clean)
}

/// The `data` payload for `--list --json`.
fn list_data(context: &CheckContext) -> serde_json::Value {
    let counts = context.compiled.layer_file_counts(&context.indexed);
    let layers: Vec<serde_json::Value> = counts
        .iter()
        .map(|(name, n)| {
            serde_json::json!({
                "name": name,
                "patterns": context.compiled.file.layers[name],
                "files": n,
            })
        })
        .collect();
    let groups = &context.compiled.file.rules;
    serde_json::json!({
        "rules_path": context.rules_path.display().to_string(),
        "version": context.compiled.file.version,
        "layers": layers,
        "rules": {
            "forbidden": groups.forbidden,
            "allowed_dependents": groups.allowed_dependents,
            "limit": groups.limit,
            "no_new_dependents": groups.no_new_dependents,
        },
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ctx::error::CtxError;
    use ctx::index::Indexer;
    use ctx::rules::RuleKind;
    use ctx::testutil::GitRepo;
    use ctx::walker::WalkerConfig;
    use std::fs;
    use tempfile::TempDir;

    // ---------- end-to-end acceptance tests ----------

    const LAYER_RULES: &str = r#"
version = 1

[layers]
domain         = ["src/domain/**"]
infrastructure = ["src/infra/**"]

[[rules.forbidden]]
from   = "domain"
to     = "infrastructure"
reason = "Domain layer must stay persistence-agnostic"
"#;

    /// Index `root` on disk (creates .ctx/codebase.sqlite).
    fn index_project(root: &Path) {
        let mut indexer = Indexer::with_config(root, false, WalkerConfig::default()).unwrap();
        indexer.index().unwrap();
    }

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn test_check_forbidden_import_end_to_end() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        write(
            root,
            "src/domain/order.ts",
            "import { query } from \"../infra/db\";\nexport function order() { return query(); }\n",
        );
        write(
            root,
            "src/infra/db.ts",
            "export function query() { return 42; }\n",
        );
        index_project(root);
        write(root, ".ctx/rules.toml", LAYER_RULES);

        // The deliberate domain -> infrastructure import is a finding.
        let context = load_context(root, None).unwrap();
        let violations = collect_violations(root, &context, None).unwrap();
        assert!(!violations.is_empty(), "expected at least one violation");
        let v = violations
            .iter()
            .find(|v| v.rule == RuleKind::Forbidden)
            .expect("expected a forbidden violation");
        // Names the exact edge: both endpoints of the offending import.
        assert_eq!(v.from.as_ref().unwrap().file(), "src/domain/order.ts");
        assert_eq!(v.to.as_ref().unwrap().file(), "src/infra/db.ts");
        assert_eq!(v.reason, "Domain layer must stay persistence-agnostic");

        let outcome = run_check_in(root, None, None, false, false).unwrap();
        assert_eq!(outcome, Outcome::Findings);

        // Remove the import and re-index: clean.
        write(
            root,
            "src/domain/order.ts",
            "export function order() { return 1; }\n",
        );
        index_project(root);
        let outcome = run_check_in(root, None, None, false, false).unwrap();
        assert_eq!(outcome, Outcome::Clean);
    }

    #[test]
    fn test_check_against_reports_only_new_violations() {
        let temp = TempDir::new().unwrap();
        let repo = GitRepo::init(temp.path());
        let root = &repo.root;

        // Commit v1 with pre-existing violation A (a.ts -> infra).
        repo.write(
            "src/domain/a.ts",
            "import { query } from \"../infra/db\";\nexport function a() { return query(); }\n",
        );
        repo.write(
            "src/infra/db.ts",
            "export function query() { return 42; }\n",
        );
        repo.write(".ctx/rules.toml", LAYER_RULES);
        repo.commit_all("v1 with violation A");

        // Commit v2 introduces violation B (b.ts -> infra).
        repo.commit_file(
            "src/domain/b.ts",
            "import { query } from \"../infra/db\";\nexport function b() { return query(); }\n",
            "add violation B",
        );

        index_project(root);
        let context = load_context(root, None).unwrap();

        // Without --against: both violations.
        let all = collect_violations(root, &context, None).unwrap();
        let files: Vec<&str> = all.iter().map(|v| v.file.as_str()).collect();
        assert!(files.contains(&"src/domain/a.ts"), "files: {:?}", files);
        assert!(files.contains(&"src/domain/b.ts"), "files: {:?}", files);

        // --against HEAD~1: only the newly introduced violation B.
        let new_only = collect_violations(root, &context, Some("HEAD~1")).unwrap();
        assert!(!new_only.is_empty());
        assert!(
            new_only.iter().all(|v| v.file == "src/domain/b.ts"),
            "violations: {:?}",
            new_only.iter().map(|v| &v.message).collect::<Vec<_>>()
        );

        let outcome = run_check_in(root, None, Some("HEAD~1".to_string()), false, false).unwrap();
        assert_eq!(outcome, Outcome::Findings);

        // Bad ref -> operational error.
        let err = collect_violations(root, &context, Some("no-such-ref")).unwrap_err();
        assert!(matches!(err, CtxError::InvalidRevision(_)), "err: {}", err);
    }

    #[test]
    fn test_check_invalid_toml_and_overlap_are_errors() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write(root, "src/domain/a.ts", "export const a = 1;\n");
        index_project(root);

        // Invalid TOML.
        write(root, ".ctx/rules.toml", "[layers\ndomain = [");
        let err = run_check_in(root, None, None, false, false).unwrap_err();
        assert!(
            err.to_string().contains("invalid rules file"),
            "err: {}",
            err
        );

        // Overlapping layer membership.
        write(
            root,
            ".ctx/rules.toml",
            r#"
            [layers]
            domain = ["src/domain/**"]
            everything = ["src/**"]
            "#,
        );
        let err = run_check_in(root, None, None, false, false).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("src/domain/a.ts"), "err: {}", msg);
        assert!(
            msg.contains("domain") && msg.contains("everything"),
            "err: {}",
            msg
        );

        // Missing rules file mentions the expected path.
        fs::remove_file(root.join(".ctx/rules.toml")).unwrap();
        let err = run_check_in(root, None, None, false, false).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("rules file not found"), "err: {}", msg);
        assert!(msg.contains("rules.toml"), "err: {}", msg);
    }

    #[test]
    fn test_check_missing_index_is_error() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write(root, ".ctx/rules.toml", LAYER_RULES);
        let err = run_check_in(root, None, None, false, false).unwrap_err();
        assert!(matches!(err, CtxError::IndexNotFound(_)), "err: {}", err);
    }

    #[test]
    fn test_check_json_data_one_entry_per_violation() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write(
            root,
            "src/domain/order.ts",
            "import { query } from \"../infra/db\";\nexport function order() { return query(); }\n",
        );
        write(
            root,
            "src/infra/db.ts",
            "export function query() { return 42; }\n",
        );
        index_project(root);
        write(root, ".ctx/rules.toml", LAYER_RULES);

        let context = load_context(root, None).unwrap();
        let violations = collect_violations(root, &context, None).unwrap();
        let data = check_data(&context.rules_path, None, &violations);

        let entries = data["violations"].as_array().unwrap();
        assert_eq!(entries.len(), violations.len());
        assert_eq!(data["summary"]["violations"], violations.len());
        assert!(data["summary"]["rules_violated"].as_u64().unwrap() >= 1);
        assert!(data["against"].is_null());

        // Every entry carries the rule type and reason.
        for entry in entries {
            assert_eq!(entry["rule"], "forbidden");
            assert_eq!(
                entry["reason"],
                "Domain layer must stay persistence-agnostic"
            );
        }

        // File-level endpoints (imports) are {"file": ...} objects.
        let import_entry = entries
            .iter()
            .find(|e| e["from"] == serde_json::json!({"file": "src/domain/order.ts"}))
            .expect("expected an import violation with file-level endpoints");
        assert_eq!(
            import_entry["to"],
            serde_json::json!({"file": "src/infra/db.ts"})
        );

        // Symbol-level endpoints are full SymbolRefs (the resolved call
        // order() -> query() crosses the same layer boundary).
        if let Some(sym_entry) = entries.iter().find(|e| e["from"].get("name").is_some()) {
            assert_eq!(sym_entry["from"]["file"], "src/domain/order.ts");
            assert_eq!(sym_entry["from"]["kind"], "function");
            assert_eq!(sym_entry["to"]["name"], "query");
            assert_eq!(sym_entry["to"]["file"], "src/infra/db.ts");
        }
    }

    #[test]
    fn test_check_list_mode() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write(root, "src/domain/a.ts", "export const a = 1;\n");
        write(root, "src/infra/db.ts", "export const b = 2;\n");
        index_project(root);
        write(root, ".ctx/rules.toml", LAYER_RULES);

        // --list always exits clean, even though a check would find nothing
        // here anyway; also verify the JSON payload shape.
        let outcome = run_check_in(root, None, None, true, false).unwrap();
        assert_eq!(outcome, Outcome::Clean);

        let context = load_context(root, None).unwrap();
        let data = list_data(&context);
        assert_eq!(data["version"], 1);
        let layers = data["layers"].as_array().unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0]["name"], "domain");
        assert_eq!(layers[0]["files"], 1);
        assert_eq!(layers[1]["name"], "infrastructure");
        assert_eq!(layers[1]["files"], 1);
        assert_eq!(data["rules"]["forbidden"][0]["from"], "domain");
        assert_eq!(
            data["rules"]["forbidden"][0]["reason"],
            "Domain layer must stay persistence-agnostic"
        );
    }
}
