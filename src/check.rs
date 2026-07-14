//! Architecture-rules check engine (backs `ctx check` and the
//! `check_violations` metric of `ctx score`).
//!
//! Loads `.ctx/rules.toml`, builds a file-level dependency set from the code
//! intelligence index (resolved call/implements/extends/uses edges plus
//! resolved imports), evaluates the rules, and returns violations. The CLI
//! wrapper in `src/commands/check.rs` handles all printing.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::db::{Database, EdgeSymbol};
use crate::error::{CtxError, Result};
use crate::gitutil;
use crate::index;
use crate::json::SymbolRef;
use crate::rules::{self, CompiledRules, Endpoint, FileDep, RulesFile, Violation};

/// Everything loaded and validated, ready for evaluation.
pub struct CheckContext {
    pub compiled: CompiledRules,
    pub indexed: Vec<String>,
    pub db: Database,
    pub rules_path: PathBuf,
}

/// Load and compile the rules file, open the index, and validate.
///
/// `rules_path` defaults to [`rules::DEFAULT_RULES_PATH`] under `root`.
/// Errors (missing or invalid rules file, unknown/overlapping layers,
/// missing index) map to exit code 2 in the CLI.
pub fn load_context(root: &Path, rules_path: Option<PathBuf>) -> Result<CheckContext> {
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
///
/// With `against`, only violations touching files changed relative to that
/// git reference are reported.
pub fn collect_violations(
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
        "c" | "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" | "ipp" | "tpp" => {
            resolve_c_include(indexed, importing_file, from)
        }
        "zig" => resolve_zig(indexed, importing_file, from),
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

/// Zig file imports are relative to the importing file. Package imports such
/// as `std` have no `.zig` suffix and remain external.
fn resolve_zig(indexed: &HashSet<String>, importing_file: &str, from: &str) -> Option<String> {
    if !from.ends_with(".zig") {
        return None;
    }
    let candidate = normalize(&format!("{}/{}", parent_dir(importing_file), from))?;
    indexed.contains(&candidate).then_some(candidate)
}

/// Resolve C/C++ includes without build-system search paths. Quoted includes
/// try the importing directory, repository root, then a unique indexed suffix.
/// Angle includes resolve only as exact repository-relative paths.
fn resolve_c_include(
    indexed: &HashSet<String>,
    importing_file: &str,
    from: &str,
) -> Option<String> {
    if let Some(enclosed) = from
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
    {
        return indexed.contains(enclosed).then(|| enclosed.to_string());
    }
    if from.starts_with('/') || from.contains('\\') {
        return None;
    }
    if let Some(relative) = normalize(&format!("{}/{}", parent_dir(importing_file), from)) {
        if indexed.contains(&relative) {
            return Some(relative);
        }
    }
    let root = normalize(from)?;
    if indexed.contains(&root) {
        return Some(root);
    }
    let suffix = format!("/{root}");
    let mut matches = indexed
        .iter()
        .filter(|path| path.ends_with(&suffix))
        .cloned();
    let only = matches.next()?;
    matches.next().is_none().then_some(only)
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
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Edge, EdgeKind, FileRecord, Symbol, SymbolKind, Visibility};
    use crate::rules::RuleKind;

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
    fn test_resolve_import_zig() {
        let idx = indexed(&[
            "src/app/main.zig",
            "src/app/util.zig",
            "src/shared.zig",
            "outside.zig",
        ]);
        assert_eq!(
            resolve_import(&idx, "src/app/main.zig", "util.zig").as_deref(),
            Some("src/app/util.zig")
        );
        assert_eq!(
            resolve_import(&idx, "src/app/main.zig", "../shared.zig").as_deref(),
            Some("src/shared.zig")
        );
        assert_eq!(
            resolve_import(&idx, "src/app/main.zig", "missing.zig"),
            None
        );
        assert_eq!(resolve_import(&idx, "main.zig", "../outside.zig"), None);
        assert_eq!(resolve_import(&idx, "src/app/main.zig", "std"), None);
    }

    #[test]
    fn test_resolve_c_cpp_includes() {
        let idx = indexed(&[
            "src/app/main.cpp",
            "src/app/local.h",
            "include/project/api.hpp",
            "vendor/exact.h",
            "one/shared/ambiguous.h",
            "two/shared/ambiguous.h",
            "unique/deep/suffix.h",
        ]);
        assert_eq!(
            resolve_import(&idx, "src/app/main.cpp", "local.h").as_deref(),
            Some("src/app/local.h")
        );
        assert_eq!(
            resolve_import(&idx, "src/app/main.cpp", "include/project/api.hpp").as_deref(),
            Some("include/project/api.hpp")
        );
        assert_eq!(
            resolve_import(&idx, "src/app/main.cpp", "deep/suffix.h").as_deref(),
            Some("unique/deep/suffix.h")
        );
        assert_eq!(
            resolve_import(&idx, "src/app/main.cpp", "<vendor/exact.h>").as_deref(),
            Some("vendor/exact.h")
        );
        assert_eq!(resolve_import(&idx, "src/app/main.cpp", "<exact.h>"), None);
        assert_eq!(
            resolve_import(&idx, "src/app/main.cpp", "shared/ambiguous.h"),
            None
        );
        assert_eq!(
            resolve_import(&idx, "src/app/main.cpp", "../../../escape.h"),
            None
        );
        assert_eq!(resolve_import(&idx, "src/app/main.cpp", "missing.h"), None);
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
        db.upsert_module(&crate::db::ModuleInfo {
            file_path: "src/b.rs".to_string(),
            module_name: Some("b".to_string()),
            exports: vec![],
            imports: vec![crate::db::ImportInfo {
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
}
