//! Query command implementations.
//!
//! Handles codebase queries: find symbols, callers, dependencies, graph traversal.

use std::env;

use crate::cli::QueryCommand;
use ctx::analytics;
use ctx::db;
use ctx::error::Result;
use ctx::index;
use ctx::json::SymbolRef;
use ctx::utils::{truncate_path, truncate_str};

/// Handle 'query find' subcommand.
fn query_find(
    db: &db::Database,
    pattern: &str,
    limit: i32,
    kind: Option<String>,
    file: Option<String>,
    json: bool,
) -> Result<()> {
    let symbols = db.find_symbols_filtered(pattern, limit, file.as_deref(), kind.as_deref())?;

    if json {
        return ctx::json::emit(
            "query.find",
            find_data(pattern, kind.as_deref(), file.as_deref(), &symbols),
        );
    }

    if symbols.is_empty() {
        eprintln!("No symbols found matching '{}'", pattern);
        if file.is_some() || kind.is_some() {
            eprintln!("Try removing filters to see all matches");
        }
        return Ok(());
    }

    println!("{:<40} {:<12} {:<10} FILE", "SYMBOL", "KIND", "VISIBILITY");
    println!("{}", "-".repeat(90));

    for symbol in symbols {
        let name = truncate_str(&symbol.name, 38);
        let file = truncate_path(&symbol.file_path, 30);
        println!(
            "{:<40} {:<12} {:<10} {}:{}",
            name,
            symbol.kind.as_str(),
            symbol.visibility.as_str(),
            file,
            symbol.line_start
        );
    }
    Ok(())
}

/// Build the `query.find` JSON payload.
fn find_data(
    pattern: &str,
    kind: Option<&str>,
    file: Option<&str>,
    symbols: &[db::Symbol],
) -> serde_json::Value {
    serde_json::json!({
        "pattern": pattern,
        "filters": { "kind": kind, "file": file },
        "symbols": symbols
            .iter()
            .map(|s| {
                let mut value = SymbolRef::from(s).to_value();
                if let serde_json::Value::Object(ref mut map) = value {
                    map.insert(
                        "visibility".to_string(),
                        serde_json::json!(s.visibility.as_str()),
                    );
                }
                value
            })
            .collect::<Vec<_>>(),
    })
}

/// A resolved caller of a symbol.
struct CallerEntry {
    symbol: db::Symbol,
    line: Option<u32>,
    context: Option<String>,
}

/// Result of looking up the callers of a symbol.
enum CallersOutcome {
    /// No symbol matched the query.
    NotFound,
    /// Multiple symbols matched and no filter was given to disambiguate.
    Ambiguous(Vec<db::Symbol>),
    /// A single symbol was selected; `callers` may be empty.
    Found {
        target: Box<db::Symbol>,
        callers: Vec<CallerEntry>,
        unresolved_callers: Vec<CallerEntry>,
    },
}

/// Whether an unresolved call's source text is credible evidence for `target`.
///
/// Methods and other qualified symbols require their qualified name. A bare
/// symbol accepts only a bare call: receiver/type-qualified calls are evidence
/// for a different symbol even when the final name is the same.
fn unresolved_context_matches(target: &db::Symbol, context: Option<&str>) -> bool {
    let qualified_name = target
        .qualified_name
        .as_deref()
        .filter(|qualified| *qualified != target.name);

    if let Some(qualified_name) = qualified_name {
        return context.is_some_and(|context| context.contains(qualified_name));
    }

    let Some(context) = context else {
        return true;
    };

    context.match_indices(&target.name).any(|(offset, _)| {
        let prefix = context[..offset].trim_end();
        !prefix.ends_with("::") && !prefix.ends_with('.') && !prefix.ends_with("->")
    })
}

fn sort_caller_entries(entries: &mut [CallerEntry]) {
    entries.sort_by(|left, right| {
        left.symbol
            .file_path
            .cmp(&right.symbol.file_path)
            .then_with(|| left.symbol.line_start.cmp(&right.symbol.line_start))
            .then_with(|| left.symbol.id.cmp(&right.symbol.id))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.context.cmp(&right.context))
    });
}

/// Look up the callers of `function`, without printing anything.
fn collect_callers(
    db: &db::Database,
    function: &str,
    file_pattern: Option<&str>,
) -> Result<CallersOutcome> {
    // First, find the symbol(s) matching the function name with optional file filter
    let symbols = db.find_symbols_filtered(function, 100, file_pattern, None)?;

    if symbols.is_empty() {
        return Ok(CallersOutcome::NotFound);
    }

    // If multiple symbols match and no file filter, request disambiguation
    if symbols.len() > 1 && file_pattern.is_none() {
        return Ok(CallersOutcome::Ambiguous(symbols));
    }

    let sym = &symbols[0];

    // Ordinary caller results are identity-based: only resolved call edges to
    // the selected symbol may appear here.
    let mut callers = Vec::new();
    for edge in db.get_incoming_edges(&sym.id)? {
        if edge.kind != db::EdgeKind::Calls || edge.target_id.as_deref() != Some(sym.id.as_str()) {
            continue;
        }
        if let Some(s) = db.get_symbol(&edge.source_id)? {
            callers.push(CallerEntry {
                symbol: s,
                line: edge.line,
                context: edge.context,
            });
        }
    }
    sort_caller_entries(&mut callers);

    // Preserve conservative, potentially useful name evidence separately.
    // Cross-language edges and edges resolved to any symbol are excluded.
    let target_language = db.get_file_language(&sym.file_path)?;
    let mut unresolved_callers = Vec::new();
    if target_language.is_some() {
        for edge in db.get_incoming_edges(&sym.name)? {
            if edge.kind != db::EdgeKind::Calls
                || edge.target_id.is_some()
                || edge.target_name != sym.name
                || !unresolved_context_matches(sym, edge.context.as_deref())
            {
                continue;
            }
            if let Some(source) = db.get_symbol(&edge.source_id)? {
                if db.get_file_language(&source.file_path)? != target_language {
                    continue;
                }
                unresolved_callers.push(CallerEntry {
                    symbol: source,
                    line: edge.line,
                    context: edge.context,
                });
            }
        }
    }
    sort_caller_entries(&mut unresolved_callers);

    Ok(CallersOutcome::Found {
        target: Box::new(symbols.into_iter().next().expect("checked non-empty")),
        callers,
        unresolved_callers,
    })
}

/// Build the `query.callers` JSON payload.
fn callers_data(outcome: &CallersOutcome) -> serde_json::Value {
    match outcome {
        CallersOutcome::NotFound => serde_json::json!({
            "target": serde_json::Value::Null,
            "callers": [],
            "unresolved_callers": [],
            "ambiguous": [],
        }),
        CallersOutcome::Ambiguous(symbols) => serde_json::json!({
            "target": serde_json::Value::Null,
            "callers": [],
            "unresolved_callers": [],
            "ambiguous": symbols.iter().map(SymbolRef::from).collect::<Vec<_>>(),
        }),
        CallersOutcome::Found {
            target,
            callers,
            unresolved_callers,
        } => serde_json::json!({
            "target": SymbolRef::from(target.as_ref()),
            "callers": callers
                .iter()
                .map(|c| serde_json::json!({
                    "symbol": SymbolRef::from(&c.symbol),
                    "line": c.line,
                    "context": c.context,
                }))
                .collect::<Vec<_>>(),
            "unresolved_callers": unresolved_callers
                .iter()
                .map(|c| serde_json::json!({
                    "symbol": SymbolRef::from(&c.symbol),
                    "line": c.line,
                    "context": c.context,
                }))
                .collect::<Vec<_>>(),
            "ambiguous": [],
        }),
    }
}

/// Handle 'query callers' subcommand.
fn query_callers(
    db: &db::Database,
    function: &str,
    file_pattern: Option<&str>,
    json: bool,
) -> Result<()> {
    let outcome = collect_callers(db, function, file_pattern)?;

    if json {
        return ctx::json::emit("query.callers", callers_data(&outcome));
    }

    match outcome {
        CallersOutcome::NotFound => {
            eprintln!("Symbol '{}' not found", function);
            if file_pattern.is_some() {
                eprintln!(
                    "Try removing --file filter or use 'ctx query find {}' to see all matches",
                    function
                );
            }
        }
        CallersOutcome::Ambiguous(symbols) => {
            print_disambiguation(
                &symbols,
                function,
                "--file",
                &format!(
                    "ctx query callers {} --file \"{}\"",
                    function, symbols[0].file_path
                ),
            );
        }
        CallersOutcome::Found {
            target,
            callers,
            unresolved_callers,
        } => {
            if callers.is_empty() && unresolved_callers.is_empty() {
                eprintln!("No callers found for '{}' ({})", function, target.file_path);
                return Ok(());
            }

            if !callers.is_empty() {
                println!(
                    "Functions that call '{}' ({}):",
                    target.name, target.file_path
                );
                println!("{}", "-".repeat(60));

                for entry in callers {
                    println!(
                        "  {} ({}:{})",
                        entry.symbol.name,
                        entry.symbol.file_path,
                        entry.line.unwrap_or(entry.symbol.line_start)
                    );
                    if let Some(ctx) = entry.context {
                        println!("    > {}", ctx);
                    }
                }
            } else {
                eprintln!(
                    "No resolved callers found for '{}' ({})",
                    function, target.file_path
                );
            }

            if !unresolved_callers.is_empty() {
                println!("\nUnresolved same-language call evidence:");
                println!("{}", "-".repeat(60));
                for entry in unresolved_callers {
                    println!(
                        "  {} ({}:{})",
                        entry.symbol.name,
                        entry.symbol.file_path,
                        entry.line.unwrap_or(entry.symbol.line_start)
                    );
                    if let Some(ctx) = entry.context {
                        println!("    > {}", ctx);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Result of looking up the dependencies of a symbol.
enum DepsOutcome {
    NotFound,
    Ambiguous(Vec<db::Symbol>),
    Found {
        target: Box<db::Symbol>,
        /// Outgoing edges with the resolved target symbol (if any).
        deps: Vec<(db::Edge, Option<db::Symbol>)>,
    },
}

/// Look up the dependencies of `symbol`, without printing anything.
fn collect_deps(
    db: &db::Database,
    symbol: &str,
    file_pattern: Option<&str>,
    kind_filter: Option<&str>,
) -> Result<DepsOutcome> {
    let symbols = db.find_symbols_filtered(symbol, 100, file_pattern, kind_filter)?;

    if symbols.is_empty() {
        return Ok(DepsOutcome::NotFound);
    }

    if symbols.len() > 1 && file_pattern.is_none() && kind_filter.is_none() {
        return Ok(DepsOutcome::Ambiguous(symbols));
    }

    let sym = symbols.into_iter().next().expect("checked non-empty");
    let edges = db.get_outgoing_edges(&sym.id)?;

    let mut deps = Vec::new();
    for edge in edges {
        let resolved = match &edge.target_id {
            Some(id) => db.get_symbol(id)?,
            None => None,
        };
        deps.push((edge, resolved));
    }

    Ok(DepsOutcome::Found {
        target: Box::new(sym),
        deps,
    })
}

/// Build the `query.deps` JSON payload.
fn deps_data(outcome: &DepsOutcome) -> serde_json::Value {
    match outcome {
        DepsOutcome::NotFound => serde_json::json!({
            "target": serde_json::Value::Null,
            "dependencies": [],
            "ambiguous": [],
        }),
        DepsOutcome::Ambiguous(symbols) => serde_json::json!({
            "target": serde_json::Value::Null,
            "dependencies": [],
            "ambiguous": symbols.iter().map(SymbolRef::from).collect::<Vec<_>>(),
        }),
        DepsOutcome::Found { target, deps } => serde_json::json!({
            "target": SymbolRef::from(target.as_ref()),
            "dependencies": deps
                .iter()
                .map(|(edge, resolved)| serde_json::json!({
                    "kind": edge.kind.as_str(),
                    "target_name": edge.target_name,
                    "line": edge.line,
                    "resolved": resolved.as_ref().map(SymbolRef::from),
                }))
                .collect::<Vec<_>>(),
            "ambiguous": [],
        }),
    }
}

/// Handle 'query deps' subcommand.
fn query_deps(
    db: &db::Database,
    symbol: &str,
    file_pattern: Option<&str>,
    kind_filter: Option<&str>,
    json: bool,
) -> Result<()> {
    let outcome = collect_deps(db, symbol, file_pattern, kind_filter)?;

    if json {
        return ctx::json::emit("query.deps", deps_data(&outcome));
    }

    match outcome {
        DepsOutcome::NotFound => {
            eprintln!("Symbol '{}' not found", symbol);
            if file_pattern.is_some() || kind_filter.is_some() {
                eprintln!(
                    "Try removing filters or use 'ctx query find {}' to see all matches",
                    symbol
                );
            }
        }
        DepsOutcome::Ambiguous(symbols) => {
            print_disambiguation(
                &symbols,
                symbol,
                "--file or --kind",
                &format!(
                    "ctx query deps {} --file \"{}\"",
                    symbol, symbols[0].file_path
                ),
            );
        }
        DepsOutcome::Found { target, deps } => {
            if deps.is_empty() {
                eprintln!(
                    "No dependencies found for '{}' ({})",
                    symbol, target.file_path
                );
                return Ok(());
            }

            println!("Dependencies of '{}' ({}):", target.name, target.file_path);
            println!("{}", "-".repeat(60));

            for (edge, _) in deps {
                println!(
                    "  {} {} (line {})",
                    edge.kind.as_str(),
                    edge.target_name,
                    edge.line.unwrap_or(0)
                );
            }
        }
    }
    Ok(())
}

/// Print the shared "multiple symbols matched" disambiguation help to stderr.
fn print_disambiguation(symbols: &[db::Symbol], name: &str, filters: &str, example: &str) {
    eprintln!(
        "Found {} symbols named '{}'. Use {} to disambiguate:\n",
        symbols.len(),
        name,
        filters
    );
    for s in symbols.iter().take(10) {
        eprintln!(
            "  {} ({}) - {}:{}",
            s.name,
            s.kind.as_str(),
            s.file_path,
            s.line_start
        );
    }
    if symbols.len() > 10 {
        eprintln!("  ... and {} more", symbols.len() - 10);
    }
    eprintln!("\nExample: {}", example);
}

/// Build a SymbolRef-shaped value from a call-graph/impact node.
///
/// Graph traversal results don't carry line information, so `line_start` and
/// `line_end` are 0.
fn node_symbol(name: &str, file_path: &str, kind: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "qualified_name": serde_json::Value::Null,
        "kind": kind,
        "file": file_path,
        "line_start": 0,
        "line_end": 0,
    })
}

/// Build the `query.graph` JSON payload.
fn graph_data(root: &str, depth: i32, nodes: &[analytics::CallGraphNode]) -> serde_json::Value {
    serde_json::json!({
        "root": root,
        "depth": depth,
        "nodes": nodes
            .iter()
            .map(|n| serde_json::json!({
                "symbol": node_symbol(&n.name, &n.file_path, &n.kind),
                "depth": n.depth,
            }))
            .collect::<Vec<_>>(),
    })
}

/// Build the `query.impact` JSON payload.
fn impact_data(target: &str, depth: i32, impacts: &[analytics::ImpactNode]) -> serde_json::Value {
    serde_json::json!({
        "target": target,
        "depth": depth,
        "impacted": impacts
            .iter()
            .map(|n| serde_json::json!({
                "symbol": node_symbol(&n.name, &n.file_path, &n.kind),
                "distance": n.distance,
            }))
            .collect::<Vec<_>>(),
        "total": impacts.len(),
    })
}

/// Build the `query.stats` JSON payload.
fn stats_data(
    stats: &db::CodebaseStats,
    per_file: &[analytics::FileStats],
    most_connected: &[(String, String, i64, i64)],
) -> serde_json::Value {
    serde_json::json!({
        "files": stats.files,
        "symbols": stats.symbols,
        "functions": stats.functions,
        "structs": stats.structs,
        "enums": stats.enums,
        "traits": stats.traits,
        "edges": stats.edges,
        "per_file": per_file
            .iter()
            .map(|fs| serde_json::json!({
                "file": fs.file_path,
                "symbols": fs.symbol_count,
                "functions": fs.functions,
                "public": fs.public_symbols,
                "types": fs.structs + fs.enums,
            }))
            .collect::<Vec<_>>(),
        "most_connected": most_connected
            .iter()
            .map(|(name, file, calls_out, called_by)| serde_json::json!({
                "name": name,
                "file": file,
                "calls_out": calls_out,
                "called_by": called_by,
            }))
            .collect::<Vec<_>>(),
    })
}

/// Run query subcommands.
pub fn run_query(query: QueryCommand, json: bool) -> Result<()> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    match query {
        QueryCommand::Find {
            pattern,
            limit,
            kind,
            file,
        } => query_find(&db, &pattern, limit, kind, file, json),
        QueryCommand::Callers {
            function,
            depth: _,
            file,
        } => query_callers(&db, &function, file.as_deref(), json),
        QueryCommand::Deps {
            symbol,
            depth: _,
            file,
            kind,
        } => query_deps(&db, &symbol, file.as_deref(), kind.as_deref(), json),
        QueryCommand::Graph {
            start,
            depth,
            output,
        } => {
            // Use DuckDB analytics for recursive graph traversal
            let analytics = analytics::Analytics::open(&root)?;

            let nodes = analytics.call_graph(&start, depth)?;

            // `--output json` remains accepted as an alias for `--json`.
            if json || output == "json" {
                ctx::json::emit("query.graph", graph_data(&start, depth, &nodes))?;
            } else if output == "dot" {
                // GraphViz DOT format
                println!("digraph call_graph {{");
                println!("  rankdir=LR;");
                println!("  node [shape=box];");
                println!("  \"{}\" [style=filled, fillcolor=lightblue];", start);
                for node in &nodes {
                    let color = match node.depth {
                        1 => "lightgreen",
                        2 => "lightyellow",
                        _ => "white",
                    };
                    println!("  \"{}\" [fillcolor={}];", node.name, color);
                }
                // Add edges based on depth
                let mut prev_depth_nodes: Vec<&str> = vec![&start];
                for d in 1..=depth {
                    let current: Vec<_> = nodes.iter().filter(|n| n.depth == d).collect();
                    for node in &current {
                        if let Some(prev) = prev_depth_nodes.first() {
                            println!("  \"{}\" -> \"{}\";", prev, node.name);
                        }
                    }
                    prev_depth_nodes = current.iter().map(|n| n.name.as_str()).collect();
                }
                println!("}}");
            } else {
                println!("Call graph from '{}' (depth={}):", start, depth);
                println!("{}", "-".repeat(70));

                let mut current_depth = 0;
                for node in &nodes {
                    if node.depth != current_depth {
                        current_depth = node.depth;
                        println!("\nDepth {}:", current_depth);
                    }
                    println!("  {} ({}) [{}]", node.name, node.file_path, node.kind);
                }

                if nodes.is_empty() {
                    println!("  (no outgoing calls found)");
                }
            }
            Ok(())
        }

        QueryCommand::Impact { symbol, depth } => {
            // Use DuckDB analytics for recursive impact analysis
            let analytics = analytics::Analytics::open(&root)?;

            let impacts = analytics.impact_analysis(&symbol, depth)?;

            if json {
                return ctx::json::emit("query.impact", impact_data(&symbol, depth, &impacts));
            }

            if impacts.is_empty() {
                eprintln!("No impact detected for changes to '{}'", symbol);
                return Ok(());
            }

            println!("Impact analysis for '{}' (depth={}):", symbol, depth);
            println!("The following would be affected by changes:");
            println!("{}", "-".repeat(70));

            let mut current_distance = 0;
            for impact in &impacts {
                if impact.distance != current_distance {
                    current_distance = impact.distance;
                    println!("\nDistance {}:", current_distance);
                }
                println!("  {} ({}) [{}]", impact.name, impact.file_path, impact.kind);
            }

            println!("\nTotal: {} symbols affected", impacts.len());
            Ok(())
        }

        QueryCommand::Stats => {
            let stats = db.get_stats()?;

            if json {
                let (per_file, most_connected) = match analytics::Analytics::open(&root) {
                    Ok(analytics) => (
                        analytics.file_statistics().unwrap_or_default(),
                        analytics.most_connected(10).unwrap_or_default(),
                    ),
                    Err(_) => (Vec::new(), Vec::new()),
                };
                return ctx::json::emit(
                    "query.stats",
                    stats_data(&stats, &per_file, &most_connected),
                );
            }

            println!("Codebase Statistics");
            println!("{}", "=".repeat(60));
            println!("Files indexed:  {}", stats.files);
            println!("Total symbols:  {}", stats.symbols);
            println!("  - Functions:  {}", stats.functions);
            println!("  - Structs:    {}", stats.structs);
            println!("  - Enums:      {}", stats.enums);
            println!("  - Traits:     {}", stats.traits);
            println!("Total edges:    {}", stats.edges);

            // Use DuckDB for detailed stats
            if let Ok(analytics) = analytics::Analytics::open(&root) {
                println!("\nPer-file breakdown:");
                println!("{}", "-".repeat(60));
                println!(
                    "{:<35} {:>6} {:>6} {:>6} {:>6}",
                    "FILE", "TOTAL", "FUNCS", "PUB", "TYPES"
                );

                if let Ok(file_stats) = analytics.file_statistics() {
                    for fs in file_stats.iter().take(15) {
                        let file = truncate_path(&fs.file_path, 33);
                        println!(
                            "{:<35} {:>6} {:>6} {:>6} {:>6}",
                            file,
                            fs.symbol_count,
                            fs.functions,
                            fs.public_symbols,
                            fs.structs + fs.enums
                        );
                    }
                    if file_stats.len() > 15 {
                        println!("  ... and {} more files", file_stats.len() - 15);
                    }
                }

                // Most connected functions
                println!("\nMost connected functions:");
                println!("{}", "-".repeat(60));
                println!("{:<30} {:>10} {:>10}", "FUNCTION", "CALLS OUT", "CALLED BY");

                if let Ok(connected) = analytics.most_connected(10) {
                    for (name, _file, out_degree, in_degree) in connected {
                        let name_display = truncate_str(&name, 28);
                        println!("{:<30} {:>10} {:>10}", name_display, out_degree, in_degree);
                    }
                }
            }
            Ok(())
        }

        QueryCommand::Files => {
            let files = db.get_indexed_files()?;

            if json {
                return ctx::json::emit("query.files", serde_json::json!({ "files": files }));
            }

            println!("Indexed files ({}):", files.len());
            println!("{}", "-".repeat(60));
            for file in files {
                println!("  {}", file);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx::db::{Database, Edge, EdgeKind, FileRecord, Symbol, SymbolKind, Visibility};

    fn make_symbol(id: &str, name: &str, file: &str, line: u32) -> Symbol {
        Symbol {
            id: id.to_string(),
            file_path: file.to_string(),
            name: name.to_string(),
            qualified_name: Some(name.to_string()),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: None,
            brief: None,
            docstring: None,
            line_start: line,
            line_end: line + 3,
            col_start: 0,
            col_end: 0,
            parent_id: None,
            source: None,
        }
    }

    fn seeded_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        db.upsert_file(
            &FileRecord {
                path: "src/x.rs".to_string(),
                content_hash: "h".to_string(),
                size_bytes: 1,
                language: Some("rust".to_string()),
                last_indexed: 0,
            },
            None,
        )
        .unwrap();
        db.insert_symbol(&make_symbol(
            "src/x.rs::targetfn",
            "targetfn",
            "src/x.rs",
            1,
        ))
        .unwrap();
        db.insert_symbol(&make_symbol(
            "src/x.rs::callerfn",
            "callerfn",
            "src/x.rs",
            10,
        ))
        .unwrap();
        db.insert_edge(&Edge {
            source_id: "src/x.rs::callerfn".to_string(),
            target_id: Some("src/x.rs::targetfn".to_string()),
            target_name: "targetfn".to_string(),
            kind: EdgeKind::Calls,
            line: Some(12),
            col: None,
            context: Some("targetfn()".to_string()),
        })
        .unwrap();
        db
    }

    #[test]
    fn test_find_data_payload() {
        let db = seeded_db();
        let symbols = db
            .find_symbols_filtered("targetfn", 10, None, None)
            .unwrap();
        let data = find_data("targetfn", Some("function"), None, &symbols);

        assert_eq!(data["pattern"], "targetfn");
        assert_eq!(data["filters"]["kind"], "function");
        assert!(data["filters"]["file"].is_null());

        let first = &data["symbols"][0];
        assert_eq!(first["name"], "targetfn");
        assert_eq!(first["kind"], "function");
        assert_eq!(first["file"], "src/x.rs");
        assert_eq!(first["visibility"], "public");
        assert_eq!(first["line_start"], 1);
    }

    #[test]
    fn test_find_data_empty() {
        let data = find_data("nope", None, None, &[]);
        assert_eq!(data["symbols"], serde_json::json!([]));
    }

    #[test]
    fn test_callers_data_found() {
        let db = seeded_db();
        let outcome = collect_callers(&db, "targetfn", None).unwrap();
        let data = callers_data(&outcome);

        assert_eq!(data["target"]["name"], "targetfn");
        assert_eq!(data["ambiguous"], serde_json::json!([]));

        let callers = data["callers"].as_array().unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0]["symbol"]["name"], "callerfn");
        assert_eq!(callers[0]["line"], 12);
        assert_eq!(callers[0]["context"], "targetfn()");
        assert_eq!(data["unresolved_callers"], serde_json::json!([]));
    }

    #[test]
    fn test_callers_data_not_found() {
        let db = seeded_db();
        let outcome = collect_callers(&db, "missingfn", None).unwrap();
        let data = callers_data(&outcome);

        assert!(data["target"].is_null());
        assert_eq!(data["callers"], serde_json::json!([]));
        assert_eq!(data["unresolved_callers"], serde_json::json!([]));
        assert_eq!(data["ambiguous"], serde_json::json!([]));
    }

    #[test]
    fn test_callers_data_ambiguous() {
        let db = seeded_db();
        // Add a second file with a symbol of the same name.
        db.upsert_file(
            &FileRecord {
                path: "src/y.rs".to_string(),
                content_hash: "h2".to_string(),
                size_bytes: 1,
                language: Some("rust".to_string()),
                last_indexed: 0,
            },
            None,
        )
        .unwrap();
        db.insert_symbol(&make_symbol(
            "src/y.rs::targetfn",
            "targetfn",
            "src/y.rs",
            5,
        ))
        .unwrap();

        let outcome = collect_callers(&db, "targetfn", None).unwrap();
        let data = callers_data(&outcome);

        assert!(data["target"].is_null());
        assert_eq!(data["callers"], serde_json::json!([]));
        assert_eq!(data["unresolved_callers"], serde_json::json!([]));
        let ambiguous = data["ambiguous"].as_array().unwrap();
        assert_eq!(ambiguous.len(), 2);
        assert_eq!(ambiguous[0]["name"], "targetfn");
    }

    #[test]
    fn test_callers_separate_same_language_unresolved_evidence() {
        let db = seeded_db();
        for (path, language) in [
            ("src/a.rs", "rust"),
            ("src/z.rs", "rust"),
            ("tests/helpers.py", "python"),
        ] {
            db.upsert_file(
                &FileRecord {
                    path: path.to_string(),
                    content_hash: format!("hash-{path}"),
                    size_bytes: 1,
                    language: Some(language.to_string()),
                    last_indexed: 0,
                },
                None,
            )
            .unwrap();
        }

        for (id, name, file, line) in [
            ("src/a.rs::early", "early", "src/a.rs", 5),
            ("src/z.rs::late", "late", "src/z.rs", 20),
            ("src/z.rs::other", "targetfn", "src/z.rs", 1),
            (
                "tests/helpers.py::python_caller",
                "python_caller",
                "tests/helpers.py",
                3,
            ),
        ] {
            db.insert_symbol(&make_symbol(id, name, file, line))
                .unwrap();
        }

        let edges = [
            // Same-language bare calls remain useful, deterministic evidence.
            Edge {
                source_id: "src/z.rs::late".to_string(),
                target_id: None,
                target_name: "targetfn".to_string(),
                kind: EdgeKind::Calls,
                line: Some(22),
                col: None,
                context: Some("targetfn()".to_string()),
            },
            Edge {
                source_id: "src/a.rs::early".to_string(),
                target_id: None,
                target_name: "targetfn".to_string(),
                kind: EdgeKind::Calls,
                line: Some(7),
                col: None,
                context: Some("targetfn()".to_string()),
            },
            // Cross-language evidence is never associated with the Rust target.
            Edge {
                source_id: "tests/helpers.py::python_caller".to_string(),
                target_id: None,
                target_name: "targetfn".to_string(),
                kind: EdgeKind::Calls,
                line: Some(4),
                col: None,
                context: Some("targetfn()".to_string()),
            },
            // Qualified syntax is not evidence for an unqualified free function.
            Edge {
                source_id: "src/z.rs::late".to_string(),
                target_id: None,
                target_name: "targetfn".to_string(),
                kind: EdgeKind::Calls,
                line: Some(23),
                col: None,
                context: Some("Other::targetfn()".to_string()),
            },
            // An edge resolved to the same-named symbol in another file belongs
            // in neither result set for the selected target.
            Edge {
                source_id: "src/z.rs::late".to_string(),
                target_id: Some("src/z.rs::other".to_string()),
                target_name: "targetfn".to_string(),
                kind: EdgeKind::Calls,
                line: Some(24),
                col: None,
                context: Some("targetfn()".to_string()),
            },
            // Incoming non-call relationships are not callers.
            Edge {
                source_id: "src/z.rs::late".to_string(),
                target_id: Some("src/x.rs::targetfn".to_string()),
                target_name: "targetfn".to_string(),
                kind: EdgeKind::Uses,
                line: Some(25),
                col: None,
                context: Some("targetfn".to_string()),
            },
        ];
        for edge in edges {
            db.insert_edge(&edge).unwrap();
        }

        let outcome = collect_callers(&db, "targetfn", Some("src/x.rs")).unwrap();
        let data = callers_data(&outcome);

        let callers = data["callers"].as_array().unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0]["symbol"]["name"], "callerfn");

        let unresolved = data["unresolved_callers"].as_array().unwrap();
        assert_eq!(unresolved.len(), 2);
        assert_eq!(unresolved[0]["symbol"]["name"], "early");
        assert_eq!(unresolved[1]["symbol"]["name"], "late");
    }

    #[test]
    fn test_qualified_unresolved_callers_require_qualified_context() {
        let db = seeded_db();
        let mut target = make_symbol("src/x.rs::Worker::run", "run", "src/x.rs", 30);
        target.kind = SymbolKind::Method;
        target.qualified_name = Some("Worker::run".to_string());
        db.insert_symbol(&target).unwrap();

        for (line, context) in [(40, "run()"), (41, "Other::run()"), (42, "Worker::run()")] {
            db.insert_edge(&Edge {
                source_id: "src/x.rs::callerfn".to_string(),
                target_id: None,
                target_name: "run".to_string(),
                kind: EdgeKind::Calls,
                line: Some(line),
                col: None,
                context: Some(context.to_string()),
            })
            .unwrap();
        }

        let outcome = collect_callers(&db, "run", Some("src/x.rs")).unwrap();
        let data = callers_data(&outcome);
        assert_eq!(data["callers"], serde_json::json!([]));
        let unresolved = data["unresolved_callers"].as_array().unwrap();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0]["line"], 42);
        assert_eq!(unresolved[0]["context"], "Worker::run()");
    }

    #[test]
    fn test_deps_data_found() {
        let db = seeded_db();
        let outcome = collect_deps(&db, "callerfn", None, None).unwrap();
        let data = deps_data(&outcome);

        assert_eq!(data["target"]["name"], "callerfn");
        let deps = data["dependencies"].as_array().unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0]["kind"], "calls");
        assert_eq!(deps[0]["target_name"], "targetfn");
        assert_eq!(deps[0]["line"], 12);
        assert_eq!(deps[0]["resolved"]["name"], "targetfn");
        assert_eq!(deps[0]["resolved"]["file"], "src/x.rs");
    }

    #[test]
    fn test_graph_and_impact_data() {
        let nodes = vec![analytics::CallGraphNode {
            name: "helper".to_string(),
            file_path: "src/x.rs".to_string(),
            kind: "function".to_string(),
            depth: 1,
        }];
        let data = graph_data("main", 3, &nodes);
        assert_eq!(data["root"], "main");
        assert_eq!(data["depth"], 3);
        assert_eq!(data["nodes"][0]["symbol"]["name"], "helper");
        assert_eq!(data["nodes"][0]["depth"], 1);

        let impacts = vec![analytics::ImpactNode {
            name: "caller".to_string(),
            file_path: "src/x.rs".to_string(),
            kind: "function".to_string(),
            distance: 2,
        }];
        let data = impact_data("main", 5, &impacts);
        assert_eq!(data["target"], "main");
        assert_eq!(data["total"], 1);
        assert_eq!(data["impacted"][0]["symbol"]["name"], "caller");
        assert_eq!(data["impacted"][0]["distance"], 2);
    }

    #[test]
    fn test_stats_data() {
        let db = seeded_db();
        let stats = db.get_stats().unwrap();
        let data = stats_data(&stats, &[], &[]);
        assert_eq!(data["files"], 1);
        assert_eq!(data["symbols"], 2);
        assert_eq!(data["functions"], 2);
        assert_eq!(data["edges"], 1);
        assert_eq!(data["per_file"], serde_json::json!([]));
        assert_eq!(data["most_connected"], serde_json::json!([]));
    }
}
