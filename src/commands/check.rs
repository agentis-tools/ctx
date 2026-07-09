//! `ctx check` -- architecture rules engine.
//!
//! Loads `.ctx/rules.toml`, builds a file-level dependency set from the code
//! intelligence index (resolved call/implements/extends/uses edges plus
//! resolved imports), evaluates the rules, and reports violations.
//!
//! Exit codes follow the ctx convention: 0 = no violations, 1 = at least one
//! violation, 2 = operational error (missing/invalid rules file, unknown
//! layer names, overlapping layers, missing index, bad git ref).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use ctx::db::{Database, EdgeSymbol};
use ctx::error::{CtxError, Result};
use ctx::exit::Outcome;
use ctx::gitutil;
use ctx::index;
use ctx::json::SymbolRef;
use ctx::rules::{self, CompiledRules, Endpoint, FileDep, RulesFile, Violation};

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

/// Everything loaded and validated, ready for evaluation.
struct CheckContext {
    compiled: CompiledRules,
    indexed: Vec<String>,
    db: Database,
    rules_path: PathBuf,
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

/// Load and compile the rules file, open the index, and validate.
fn load_context(root: &Path, rules_path: Option<PathBuf>) -> Result<CheckContext> {
    let rules_path = match rules_path {
        Some(p) if p.is_absolute() => p,
        Some(p) => root.join(p),
        None => root.join(rules::DEFAULT_RULES_PATH),
    };

    if !rules_path.exists() {
        return Err(CtxError::Other(format!(
            "rules file not found: {}\n\
             Create it with a [layers] table and [[rules.*]] entries; \
             run 'ctx check --help' for a full example.",
            rules_path.display()
        )));
    }
    let content = fs::read_to_string(&rules_path)?;
    let parsed: RulesFile = toml::from_str(&content).map_err(|e| {
        CtxError::Other(format!(
            "invalid rules file {}: {}",
            rules_path.display(),
            e
        ))
    })?;
    let compiled = CompiledRules::compile(parsed)?;

    let db = index::open_database(root)?;
    let indexed = db.get_indexed_files()?;
    compiled.validate(&indexed)?;

    Ok(CheckContext {
        compiled,
        indexed,
        db,
        rules_path,
    })
}

/// Build the dependency set and evaluate all rules.
fn collect_violations(
    root: &Path,
    context: &CheckContext,
    against: Option<&str>,
) -> Result<Vec<Violation>> {
    let changed = match against {
        Some(reference) => Some(gitutil::changed_files_against_in(root, reference)?),
        None => None,
    };

    let indexed_set: HashSet<String> = context.indexed.iter().cloned().collect();
    let deps = build_deps(&context.db, &indexed_set)?;
    let symbol_metrics = context.db.symbol_metrics()?;
    let file_complexity = context.db.file_complexity()?;

    Ok(rules::evaluate(
        &context.compiled,
        &deps,
        &symbol_metrics,
        &file_complexity,
        changed.as_ref(),
    ))
}

// ============================================================================
// Dependency set
// ============================================================================

/// Build the cross-file dependency set: resolved symbol edges plus imports
/// resolved against the set of indexed files.
fn build_deps(db: &Database, indexed: &HashSet<String>) -> Result<Vec<FileDep>> {
    let mut deps = Vec::new();

    // Resolved symbol-to-symbol edges (calls/implements/extends/uses).
    for edge in db.get_cross_file_edges()? {
        deps.push(FileDep {
            from: Endpoint::Symbol(symbol_ref(&edge.source)),
            to: Endpoint::Symbol(symbol_ref(&edge.target)),
            kind: edge.kind,
            line: edge.line,
        });
    }

    // Imports recorded in module metadata (TS/JS, Rust, Python, Solidity).
    for (file, imports) in db.get_file_imports()? {
        for import in imports {
            if let Some(target) = resolve_import(indexed, &file, &import.from) {
                if target != file {
                    deps.push(FileDep {
                        from: Endpoint::File { file: file.clone() },
                        to: Endpoint::File { file: target },
                        kind: "import".to_string(),
                        line: None,
                    });
                }
            }
        }
    }

    // File-level import edges (Go records imports in the edges table with the
    // importing file path as the source).
    for (source, target_path, line) in db.get_import_edges()? {
        if !indexed.contains(&source) {
            continue;
        }
        if let Some(target) = resolve_import(indexed, &source, &target_path) {
            if target != source {
                deps.push(FileDep {
                    from: Endpoint::File {
                        file: source.clone(),
                    },
                    to: Endpoint::File { file: target },
                    kind: "import".to_string(),
                    line,
                });
            }
        }
    }

    Ok(deps)
}

fn symbol_ref(s: &EdgeSymbol) -> SymbolRef {
    SymbolRef {
        name: s.name.clone(),
        qualified_name: s.qualified_name.clone(),
        kind: s.kind.clone(),
        file: s.file_path.clone(),
        line_start: s.line_start,
        line_end: s.line_end,
    }
}

// ============================================================================
// Import resolution
// ============================================================================

const JS_SUFFIXES: &[&str] = &[
    "",
    ".ts",
    ".tsx",
    ".js",
    ".jsx",
    "/index.ts",
    "/index.tsx",
    "/index.js",
];
const SOL_SUFFIXES: &[&str] = &["", ".sol"];

/// Resolve an import specifier to an indexed file, or `None` when the target
/// is external / unresolvable (unresolvable imports are silently skipped).
///
/// Heuristics are keyed off the importing file's extension.
fn resolve_import(indexed: &HashSet<String>, importing_file: &str, from: &str) -> Option<String> {
    let ext = Path::new(importing_file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => {
            resolve_relative(indexed, importing_file, from, JS_SUFFIXES)
        }
        "sol" => resolve_relative(indexed, importing_file, from, SOL_SUFFIXES),
        "rs" => resolve_rust(indexed, importing_file, from),
        "py" => resolve_python(indexed, importing_file, from),
        "go" => resolve_go(indexed, from),
        _ => None,
    }
}

/// Directory part of a slash-separated path (`""` for root-level files).
fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

/// Resolve `.`/`..` segments. Returns `None` when the path escapes the root
/// or normalizes to nothing.
fn normalize(path: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            s => out.push(s),
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join("/"))
    }
}

/// Relative-path resolution for TS/JS and Solidity. Non-relative specifiers
/// are third-party packages and resolve to `None`.
fn resolve_relative(
    indexed: &HashSet<String>,
    importing_file: &str,
    from: &str,
    suffixes: &[&str],
) -> Option<String> {
    if !from.starts_with("./") && !from.starts_with("../") {
        return None;
    }
    let base = normalize(&format!("{}/{}", parent_dir(importing_file), from))?;
    for suffix in suffixes {
        let candidate = format!("{}{}", base, suffix);
        if indexed.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Rust `use` path resolution: `crate::` from `src/`, `self::`/`super::`
/// relative to the importing file's module, bare paths are external crates.
fn resolve_rust(indexed: &HashSet<String>, importing_file: &str, from: &str) -> Option<String> {
    let segs: Vec<&str> = from.split("::").filter(|s| !s.is_empty()).collect();
    let first = *segs.first()?;

    let (base, rest): (String, &[&str]) = match first {
        "crate" => ("src".to_string(), &segs[1..]),
        "self" => (rust_self_dir(importing_file), &segs[1..]),
        "super" => {
            let mut dir = rust_self_dir(importing_file);
            let mut i = 0;
            while i < segs.len() && segs[i] == "super" {
                dir = parent_dir(&dir).to_string();
                i += 1;
            }
            (dir, &segs[i..])
        }
        _ => return None, // external crate (or 2015-style bare module)
    };

    // Drop trailing segments one by one: the last segment(s) may be items
    // (types, functions) rather than modules.
    for k in (1..=rest.len()).rev() {
        let module_path = join_path(&base, &rest[..k].join("/"));
        for candidate in [
            format!("{}.rs", module_path),
            format!("{}/mod.rs", module_path),
        ] {
            if indexed.contains(&candidate) {
                return Some(candidate);
            }
        }
    }

    // `use crate::Item` / `use super::Item`: the target is the parent module
    // file itself.
    if !base.is_empty() {
        for candidate in [format!("{}/mod.rs", base), format!("{}.rs", base)] {
            if indexed.contains(&candidate) {
                return Some(candidate);
            }
        }
        if base == "src" {
            for candidate in ["src/lib.rs".to_string(), "src/main.rs".to_string()] {
                if indexed.contains(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// The directory that holds child modules of the importing Rust file:
/// `src/a/b.rs` (module `a::b`) -> `src/a/b`; `src/a/mod.rs`, `src/lib.rs`,
/// and `src/main.rs` -> their own directory.
fn rust_self_dir(importing_file: &str) -> String {
    let file_name = importing_file.rsplit('/').next().unwrap_or(importing_file);
    if matches!(file_name, "mod.rs" | "lib.rs" | "main.rs") {
        parent_dir(importing_file).to_string()
    } else {
        importing_file
            .strip_suffix(".rs")
            .unwrap_or(importing_file)
            .to_string()
    }
}

fn join_path(base: &str, rest: &str) -> String {
    if base.is_empty() {
        rest.to_string()
    } else if rest.is_empty() {
        base.to_string()
    } else {
        format!("{}/{}", base, rest)
    }
}

/// Python import resolution: dotted absolute modules (also probed under
/// `src/`), and `from .`-style relative imports resolved from the importing
/// file's directory.
fn resolve_python(indexed: &HashSet<String>, importing_file: &str, from: &str) -> Option<String> {
    if let Some(mut rest) = from.strip_prefix('.') {
        // One leading dot = current package; each extra dot pops a level.
        let mut dir = parent_dir(importing_file).to_string();
        while let Some(r) = rest.strip_prefix('.') {
            dir = parent_dir(&dir).to_string();
            rest = r;
        }
        let segs: Vec<&str> = rest.split('.').filter(|s| !s.is_empty()).collect();
        if segs.is_empty() {
            // `from . import x`: the target is the package itself.
            if dir.is_empty() {
                return None;
            }
            let candidate = format!("{}/__init__.py", dir);
            return indexed.contains(&candidate).then_some(candidate);
        }
        probe_python(indexed, &join_path(&dir, &segs.join("/")))
    } else {
        let rel = from.replace('.', "/");
        probe_python(indexed, &rel).or_else(|| probe_python(indexed, &format!("src/{}", rel)))
    }
}

fn probe_python(indexed: &HashSet<String>, base: &str) -> Option<String> {
    [format!("{}.py", base), format!("{}/__init__.py", base)]
        .into_iter()
        .find(|candidate| indexed.contains(candidate))
}

/// Go import resolution: import paths name a package directory
/// (e.g. `example.com/proj/pkg/util`). Match the longest trailing-directory
/// suffix against indexed directories and return the first `.go` file in the
/// matched directory.
fn resolve_go(indexed: &HashSet<String>, from: &str) -> Option<String> {
    let segs: Vec<&str> = from.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return None;
    }

    let mut go_files: Vec<&String> = indexed.iter().filter(|f| f.ends_with(".go")).collect();
    go_files.sort();

    for k in (1..=segs.len()).rev() {
        let suffix = segs[segs.len() - k..].join("/");
        let hit = go_files.iter().find(|f| {
            let dir = parent_dir(f);
            dir == suffix || dir.ends_with(&format!("/{}", suffix))
        });
        if let Some(f) = hit {
            return Some((*f).clone());
        }
    }
    None
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
    use ctx::db::{Edge, EdgeKind, FileRecord, Symbol, SymbolKind, Visibility};
    use ctx::index::Indexer;
    use ctx::rules::RuleKind;
    use ctx::testutil::GitRepo;
    use ctx::walker::WalkerConfig;
    use tempfile::TempDir;

    // ---------- import resolver ----------

    fn indexed(files: &[&str]) -> HashSet<String> {
        files.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_resolve_import_typescript() {
        let idx = indexed(&[
            "src/domain/order.ts",
            "src/infra/db.ts",
            "src/app/util/index.ts",
            "src/legacy/api.js",
        ]);
        let cases = [
            // relative sibling with probing
            (
                "src/domain/order.ts",
                "../infra/db",
                Some("src/infra/db.ts"),
            ),
            (
                "src/domain/order.ts",
                "../infra/db.ts",
                Some("src/infra/db.ts"),
            ),
            // directory import -> index file
            (
                "src/domain/order.ts",
                "../app/util",
                Some("src/app/util/index.ts"),
            ),
            // .js target
            (
                "src/domain/order.ts",
                "../legacy/api",
                Some("src/legacy/api.js"),
            ),
            // same-dir
            ("src/infra/db.ts", "./db", Some("src/infra/db.ts")),
            // third-party
            ("src/domain/order.ts", "react", None),
            ("src/domain/order.ts", "@scope/pkg", None),
            // escapes the root
            ("src/domain/order.ts", "../../../outside", None),
            // missing
            ("src/domain/order.ts", "./nope", None),
        ];
        for (file, from, expected) in cases {
            assert_eq!(
                resolve_import(&idx, file, from).as_deref(),
                expected,
                "{} imports {:?}",
                file,
                from
            );
        }
    }

    #[test]
    fn test_resolve_import_rust() {
        let idx = indexed(&[
            "src/lib.rs",
            "src/main.rs",
            "src/domain/order.rs",
            "src/domain/mod.rs",
            "src/infra/mod.rs",
            "src/infra/db.rs",
        ]);
        let cases = [
            // crate:: module path
            ("src/main.rs", "crate::infra::db", Some("src/infra/db.rs")),
            // trailing item segment dropped
            (
                "src/main.rs",
                "crate::infra::db::Pool",
                Some("src/infra/db.rs"),
            ),
            // module dir with mod.rs
            ("src/main.rs", "crate::domain", Some("src/domain/mod.rs")),
            // bare crate item -> lib.rs
            ("src/domain/order.rs", "crate", Some("src/lib.rs")),
            // super:: from a nested module
            (
                "src/infra/db.rs",
                "super::super::domain::order",
                Some("src/domain/order.rs"),
            ),
            // self:: children (src/domain/order.rs would own src/domain/order/)
            (
                "src/domain/mod.rs",
                "self::order",
                Some("src/domain/order.rs"),
            ),
            // super item falls back to the parent module file
            (
                "src/domain/order.rs",
                "super::Registry",
                Some("src/domain/mod.rs"),
            ),
            // external crates
            ("src/main.rs", "std::collections::HashMap", None),
            ("src/main.rs", "serde::Serialize", None),
        ];
        for (file, from, expected) in cases {
            assert_eq!(
                resolve_import(&idx, file, from).as_deref(),
                expected,
                "{} imports {:?}",
                file,
                from
            );
        }
    }

    #[test]
    fn test_resolve_import_python() {
        let idx = indexed(&[
            "app/__init__.py",
            "app/models.py",
            "app/api/routes.py",
            "src/pkg/util.py",
        ]);
        let cases = [
            // absolute dotted path
            ("app/api/routes.py", "app.models", Some("app/models.py")),
            // package __init__
            ("app/api/routes.py", "app", Some("app/__init__.py")),
            // src/ prefix probing
            ("app/models.py", "pkg.util", Some("src/pkg/util.py")),
            // relative: one dot = current package
            ("app/api/routes.py", ".routes", Some("app/api/routes.py")),
            // relative: two dots pop a level
            ("app/api/routes.py", "..models", Some("app/models.py")),
            // `from . import x` -> package itself
            ("app/models.py", ".", Some("app/__init__.py")),
            // third-party
            ("app/models.py", "django.db", None),
        ];
        for (file, from, expected) in cases {
            assert_eq!(
                resolve_import(&idx, file, from).as_deref(),
                expected,
                "{} imports {:?}",
                file,
                from
            );
        }
    }

    #[test]
    fn test_resolve_import_go_and_solidity() {
        let idx = indexed(&[
            "cmd/server/main.go",
            "pkg/util/strings.go",
            "pkg/util/numbers.go",
            "contracts/Token.sol",
            "contracts/lib/Math.sol",
        ]);
        // Go: tail-dir suffix match, first file in the matched dir.
        assert_eq!(
            resolve_import(&idx, "cmd/server/main.go", "example.com/proj/pkg/util").as_deref(),
            Some("pkg/util/numbers.go")
        );
        assert_eq!(
            resolve_import(&idx, "cmd/server/main.go", "example.com/other/nowhere"),
            None
        );
        // Solidity: relative like TS.
        assert_eq!(
            resolve_import(&idx, "contracts/Token.sol", "./lib/Math.sol").as_deref(),
            Some("contracts/lib/Math.sol")
        );
        assert_eq!(
            resolve_import(
                &idx,
                "contracts/Token.sol",
                "@openzeppelin/contracts/token.sol"
            ),
            None
        );
    }

    #[test]
    fn test_resolve_import_unknown_extension() {
        let idx = indexed(&["a.yaml", "b.yaml"]);
        assert_eq!(resolve_import(&idx, "a.yaml", "./b"), None);
    }

    // ---------- limit rules against a hand-built database ----------

    fn make_symbol(id: &str, name: &str, file: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            id: id.to_string(),
            file_path: file.to_string(),
            name: name.to_string(),
            qualified_name: None,
            kind,
            visibility: Visibility::Public,
            signature: None,
            brief: None,
            docstring: None,
            line_start: 1,
            line_end: 5,
            col_start: 0,
            col_end: 0,
            parent_id: None,
            source: None,
        }
    }

    fn make_call(source: &str, target: &str) -> Edge {
        Edge {
            source_id: source.to_string(),
            target_id: Some(target.to_string()),
            target_name: target.rsplit("::").next().unwrap_or(target).to_string(),
            kind: EdgeKind::Calls,
            line: Some(3),
            col: None,
            context: None,
        }
    }

    fn add_file(db: &Database, path: &str) {
        db.upsert_file(
            &FileRecord {
                path: path.to_string(),
                content_hash: format!("hash-{}", path),
                size_bytes: 1,
                language: Some("rust".to_string()),
                last_indexed: 0,
            },
            None,
        )
        .unwrap();
    }

    #[test]
    fn test_limit_rules_against_in_memory_db() {
        let db = Database::open_in_memory().unwrap();
        add_file(&db, "src/hub.rs");
        add_file(&db, "src/a.rs");
        add_file(&db, "src/b.rs");
        add_file(&db, "src/c.rs");

        // hub() is called by a(), b(), c() -> fan_in 3.
        db.insert_symbol(&make_symbol(
            "src/hub.rs::hub",
            "hub",
            "src/hub.rs",
            SymbolKind::Function,
        ))
        .unwrap();
        for f in ["a", "b", "c"] {
            let id = format!("src/{}.rs::{}", f, f);
            db.insert_symbol(&make_symbol(
                &id,
                f,
                &format!("src/{}.rs", f),
                SymbolKind::Function,
            ))
            .unwrap();
            db.insert_edge(&make_call(&id, "src/hub.rs::hub")).unwrap();
        }

        let rules_file: RulesFile = toml::from_str(
            r#"
            [[rules.limit]]
            metric = "fan_in"
            max = 2
            "#,
        )
        .unwrap();
        let compiled = CompiledRules::compile(rules_file).unwrap();
        compiled.validate(&db.get_indexed_files().unwrap()).unwrap();

        let metrics = db.symbol_metrics().unwrap();
        let files = db.file_complexity().unwrap();
        let violations = rules::evaluate(&compiled, &[], &metrics, &files, None);

        assert_eq!(violations.len(), 1);
        let v = &violations[0];
        assert_eq!(v.rule, RuleKind::Limit);
        assert_eq!(v.file, "src/hub.rs");
        assert_eq!(v.value, Some(3));
        assert_eq!(v.max, Some(2));
    }

    #[test]
    fn test_build_deps_from_in_memory_db() {
        let db = Database::open_in_memory().unwrap();
        add_file(&db, "src/a.rs");
        add_file(&db, "src/b.rs");

        db.insert_symbol(&make_symbol(
            "src/a.rs::a",
            "a",
            "src/a.rs",
            SymbolKind::Function,
        ))
        .unwrap();
        db.insert_symbol(&make_symbol(
            "src/b.rs::b",
            "b",
            "src/b.rs",
            SymbolKind::Function,
        ))
        .unwrap();
        // Cross-file resolved call.
        db.insert_edge(&make_call("src/a.rs::a", "src/b.rs::b"))
            .unwrap();
        // Unresolved edge and same-file edge must be ignored.
        db.insert_edge(&Edge {
            source_id: "src/a.rs::a".to_string(),
            target_id: None,
            target_name: "external".to_string(),
            kind: EdgeKind::Calls,
            line: Some(4),
            col: None,
            context: None,
        })
        .unwrap();

        // Module-level import (Rust style).
        db.upsert_module(&ctx::db::ModuleInfo {
            file_path: "src/b.rs".to_string(),
            module_name: Some("b".to_string()),
            exports: vec![],
            imports: vec![ctx::db::ImportInfo {
                from: "crate::a".to_string(),
                names: vec!["a".to_string()],
                alias: None,
            }],
        })
        .unwrap();

        let indexed: HashSet<String> = db.get_indexed_files().unwrap().into_iter().collect();
        let deps = build_deps(&db, &indexed).unwrap();

        assert_eq!(deps.len(), 2);
        // The resolved call edge, with symbol endpoints.
        assert_eq!(deps[0].kind, "calls");
        assert_eq!(deps[0].from.file(), "src/a.rs");
        assert_eq!(deps[0].to.file(), "src/b.rs");
        assert!(matches!(deps[0].from, Endpoint::Symbol(_)));
        // The resolved import, with file endpoints.
        assert_eq!(deps[1].kind, "import");
        assert_eq!(deps[1].from.file(), "src/b.rs");
        assert_eq!(deps[1].to.file(), "src/a.rs");
        assert!(matches!(deps[1].from, Endpoint::File { .. }));
    }

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
