//! Query command implementations.
//!
//! Handles codebase queries: find symbols, callers, dependencies, graph traversal.

use std::env;

use crate::cli::QueryCommand;
use ctx::analytics;
use ctx::db;
use ctx::error::Result;
use ctx::index;
use ctx::utils::{truncate_path, truncate_str};

/// Handle 'query find' subcommand.
fn query_find(
    db: &db::Database,
    pattern: &str,
    limit: i32,
    kind: Option<String>,
    file: Option<String>,
) -> Result<()> {
    let symbols = db.find_symbols_filtered(pattern, limit, file.as_deref(), kind.as_deref())?;

    if symbols.is_empty() {
        eprintln!("No symbols found matching '{}'", pattern);
        if file.is_some() || kind.is_some() {
            eprintln!("Try removing filters to see all matches");
        }
        return Ok(());
    }

    println!(
        "{:<40} {:<12} {:<10} FILE",
        "SYMBOL", "KIND", "VISIBILITY"
    );
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

/// Handle 'query callers' subcommand.
fn query_callers(db: &db::Database, function: &str, file_pattern: Option<&str>) -> Result<()> {
    // First, find the symbol(s) matching the function name with optional file filter
    let symbols = db.find_symbols_filtered(function, 100, file_pattern, None)?;

    if symbols.is_empty() {
        eprintln!("Symbol '{}' not found", function);
        if file_pattern.is_some() {
            eprintln!(
                "Try removing --file filter or use 'ctx query find {}' to see all matches",
                function
            );
        }
        return Ok(());
    }

    // If multiple symbols match and no file filter, show disambiguation help
    if symbols.len() > 1 && file_pattern.is_none() {
        eprintln!(
            "Found {} symbols named '{}'. Use --file to disambiguate:\n",
            symbols.len(),
            function
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
        eprintln!(
            "\nExample: ctx query callers {} --file \"{}\"",
            function, symbols[0].file_path
        );
        return Ok(());
    }

    let sym = &symbols[0];

    // Get callers for this specific symbol
    // Strategy:
    // 1. Get edges resolved to this symbol's ID (most accurate)
    // 2. Get edges by name, filtered to likely matches based on context
    let id_edges = db.get_incoming_edges(&sym.id)?;
    let name_edges = db.get_incoming_edges(&sym.name)?;

    // Build patterns for context matching
    // For "src/foo.rs::MyType::method@10", we check for:
    // - Full qualified name: "MyType::method"
    // - Just the parent type: "MyType::" (for cases like "MyType::new()")
    let qualified_name = sym.qualified_name.as_deref().unwrap_or(&sym.name);
    let parent_prefix = qualified_name
        .rsplit_once("::")
        .map(|(parent, _)| format!("{}::", parent));

    // Start with ID-resolved edges (most accurate)
    let mut edges = id_edges;
    let has_id_edges = !edges.is_empty();

    // Add name-based edges that aren't duplicates and likely refer to this symbol
    for edge in name_edges {
        // Skip if already have this edge (by source_id + line)
        let is_duplicate = edges
            .iter()
            .any(|e| e.source_id == edge.source_id && e.line == edge.line);
        if is_duplicate {
            continue;
        }

        // Determine if this edge likely refers to our symbol
        let likely_match = if let Some(ref ctx) = edge.context {
            // Check if context contains our qualified name or parent type
            ctx.contains(qualified_name)
                || parent_prefix.as_ref().is_some_and(|p| ctx.contains(p))
        } else {
            // No context - include only if we have no ID-resolved edges
            // (fallback for completely unresolved graphs)
            !has_id_edges
        };

        if likely_match {
            edges.push(edge);
        }
    }

    if edges.is_empty() {
        eprintln!("No callers found for '{}' ({})", function, sym.file_path);
        return Ok(());
    }

    println!("Functions that call '{}' ({}):", sym.name, sym.file_path);
    println!("{}", "-".repeat(60));

    for edge in edges {
        if let Some(s) = db.get_symbol(&edge.source_id)? {
            println!(
                "  {} ({}:{})",
                s.name,
                s.file_path,
                edge.line.unwrap_or(s.line_start)
            );
            if let Some(ctx) = edge.context {
                println!("    > {}", ctx);
            }
        }
    }
    Ok(())
}

/// Handle 'query deps' subcommand.
fn query_deps(
    db: &db::Database,
    symbol: &str,
    file_pattern: Option<&str>,
    kind_filter: Option<&str>,
) -> Result<()> {
    let symbols = db.find_symbols_filtered(symbol, 100, file_pattern, kind_filter)?;

    if symbols.is_empty() {
        eprintln!("Symbol '{}' not found", symbol);
        if file_pattern.is_some() || kind_filter.is_some() {
            eprintln!(
                "Try removing filters or use 'ctx query find {}' to see all matches",
                symbol
            );
        }
        return Ok(());
    }

    // If multiple symbols match and no filters, show disambiguation help
    if symbols.len() > 1 && file_pattern.is_none() && kind_filter.is_none() {
        eprintln!(
            "Found {} symbols named '{}'. Use --file or --kind to disambiguate:\n",
            symbols.len(),
            symbol
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
        eprintln!(
            "\nExample: ctx query deps {} --file \"{}\"",
            symbol, symbols[0].file_path
        );
        return Ok(());
    }

    let sym = &symbols[0];
    let edges = db.get_outgoing_edges(&sym.id)?;

    if edges.is_empty() {
        eprintln!("No dependencies found for '{}' ({})", symbol, sym.file_path);
        return Ok(());
    }

    println!("Dependencies of '{}' ({}):", sym.name, sym.file_path);
    println!("{}", "-".repeat(60));

    for edge in edges {
        println!(
            "  {} {} (line {})",
            edge.kind.as_str(),
            edge.target_name,
            edge.line.unwrap_or(0)
        );
    }
    Ok(())
}

/// Run query subcommands.
pub fn run_query(query: QueryCommand) -> Result<()> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    match query {
        QueryCommand::Find {
            pattern,
            limit,
            kind,
            file,
        } => query_find(&db, &pattern, limit, kind, file),
        QueryCommand::Callers {
            function,
            depth: _,
            file,
        } => query_callers(&db, &function, file.as_deref()),
        QueryCommand::Deps {
            symbol,
            depth: _,
            file,
            kind,
        } => query_deps(&db, &symbol, file.as_deref(), kind.as_deref()),
        QueryCommand::Graph {
            start,
            depth,
            output,
        } => {
            // Use DuckDB analytics for recursive graph traversal
            let analytics = analytics::Analytics::open(&root)?;

            let nodes = analytics.call_graph(&start, depth)?;

            if output == "json" {
                let graph = serde_json::json!({
                    "root": start,
                    "nodes": nodes.iter().map(|n| {
                        serde_json::json!({
                            "name": n.name,
                            "file": n.file_path,
                            "kind": n.kind,
                            "depth": n.depth,
                        })
                    }).collect::<Vec<_>>()
                });
                println!("{}", serde_json::to_string_pretty(&graph)?);
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
            println!("Indexed files ({}):", files.len());
            println!("{}", "-".repeat(60));
            for file in files {
                println!("  {}", file);
            }
            Ok(())
        }
    }
}
