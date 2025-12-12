mod analytics;
mod cli;
mod db;
mod default_ignores;
mod formatter;
mod index;
mod output;
mod parser;
mod tree;
mod walker;

use std::env;
use std::process;
use std::time::Instant;

use clap::Parser;

use cli::{Args, Command, QueryCommand};
use output::{generate_context, stream_context};
use walker::{discover_files, WalkerConfig};

fn main() {
    let args = Args::parse();

    if let Err(e) = run(args) {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    // Handle subcommands
    match args.command {
        Some(Command::Index { watch, verbose }) => run_index(watch, verbose),
        Some(Command::Query { query }) => run_query(query),
        Some(Command::Search { query, limit, output }) => run_search(&query, limit, &output),
        Some(Command::Source { symbol }) => run_source(&symbol),
        Some(Command::Explain { symbol }) => run_explain(&symbol),
        None => run_context(args),
    }
}

/// Run the original context generation command.
fn run_context(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();

    // Determine root directory
    let root = env::current_dir()?;

    // Build walker configuration
    let config = WalkerConfig {
        use_gitignore: !args.no_gitignore,
        use_default_ignores: !args.no_default_ignores,
        custom_ignores: args.ignore_patterns,
        include_patterns: args.patterns,
    };

    // Discover files
    let entries = discover_files(&root, &config)?;

    if entries.is_empty() {
        eprintln!("No files found matching the specified patterns.");
        return Ok(());
    }

    // Generate context (streaming by default, buffered with --no-stream)
    let result = if args.no_stream {
        let result = generate_context(
            &root,
            &entries,
            &args.format,
            !args.no_tree,
            args.show_sizes,
        )?;
        // Output to stdout (only in buffered mode)
        println!("{}", result.content);
        result
    } else {
        stream_context(
            &root,
            &entries,
            &args.format,
            !args.no_tree,
            args.show_sizes,
        )?
    };

    // Print stats to stderr (only if --stats flag is passed)
    if args.stats {
        let elapsed = start.elapsed();
        eprintln!(
            "Generated context: {} files, {} in {:.2?}",
            result.file_count,
            walker::format_size(result.total_size),
            elapsed
        );
    }

    Ok(())
}

/// Run the index command.
fn run_index(watch: bool, verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;

    if watch {
        eprintln!("Watch mode not yet implemented");
        return Ok(());
    }

    eprintln!("Indexing codebase...");

    let mut indexer = index::Indexer::new(&root, verbose)?;
    let result = indexer.index()?;

    eprintln!(
        "Indexed {} files ({} skipped, {} failed)",
        result.files_indexed, result.files_skipped, result.files_failed
    );
    eprintln!(
        "Extracted {} symbols, {} edges in {}ms",
        result.symbols_extracted, result.edges_extracted, result.elapsed_ms
    );

    // Show stats
    let stats = indexer.database().get_stats()?;
    eprintln!("\nCodebase statistics:");
    eprintln!("  Files:     {}", stats.files);
    eprintln!("  Symbols:   {}", stats.symbols);
    eprintln!("  Functions: {}", stats.functions);
    eprintln!("  Structs:   {}", stats.structs);
    eprintln!("  Enums:     {}", stats.enums);
    eprintln!("  Traits:    {}", stats.traits);
    eprintln!("  Edges:     {}", stats.edges);

    Ok(())
}

/// Run query subcommands.
fn run_query(query: QueryCommand) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    match query {
        QueryCommand::Find { pattern, limit, kind } => {
            let symbols = db.find_symbols(&pattern, limit)?;

            if symbols.is_empty() {
                eprintln!("No symbols found matching '{}'", pattern);
                return Ok(());
            }

            // Filter by kind if specified
            let symbols: Vec<_> = if let Some(ref k) = kind {
                symbols
                    .into_iter()
                    .filter(|s| s.kind.as_str() == k)
                    .collect()
            } else {
                symbols
            };

            println!("{:<40} {:<12} {:<10} {}", "SYMBOL", "KIND", "VISIBILITY", "FILE");
            println!("{}", "-".repeat(90));

            for symbol in symbols {
                let name = if symbol.name.len() > 38 {
                    format!("{}...", &symbol.name[..35])
                } else {
                    symbol.name.clone()
                };

                let file = if symbol.file_path.len() > 30 {
                    format!("...{}", &symbol.file_path[symbol.file_path.len() - 27..])
                } else {
                    symbol.file_path.clone()
                };

                println!(
                    "{:<40} {:<12} {:<10} {}:{}",
                    name,
                    symbol.kind.as_str(),
                    symbol.visibility.as_str(),
                    file,
                    symbol.line_start
                );
            }
        }

        QueryCommand::Callers { function, depth: _ } => {
            let edges = db.get_incoming_edges(&function)?;

            if edges.is_empty() {
                eprintln!("No callers found for '{}'", function);
                return Ok(());
            }

            println!("Functions that call '{}':", function);
            println!("{}", "-".repeat(60));

            for edge in edges {
                let source = db.get_symbol(&edge.source_id)?;
                if let Some(s) = source {
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
        }

        QueryCommand::Deps { symbol, depth: _ } => {
            // Find the symbol first
            let symbols = db.find_symbols(&symbol, 1)?;
            let sym = symbols.first().ok_or("Symbol not found")?;

            let edges = db.get_outgoing_edges(&sym.id)?;

            if edges.is_empty() {
                eprintln!("No dependencies found for '{}'", symbol);
                return Ok(());
            }

            println!("Dependencies of '{}':", symbol);
            println!("{}", "-".repeat(60));

            for edge in edges {
                let kind = edge.kind.as_str();
                println!("  {} {} (line {})", kind, edge.target_name, edge.line.unwrap_or(0));
            }
        }

        QueryCommand::Graph { start, depth, output } => {
            // Use DuckDB analytics for recursive graph traversal
            let analytics = analytics::Analytics::open(&root)
                .map_err(|e| format!("Failed to open analytics: {}", e))?;

            let nodes = analytics.call_graph(&start, depth)
                .map_err(|e| format!("Call graph query failed: {}", e))?;

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
                        for prev in &prev_depth_nodes {
                            println!("  \"{}\" -> \"{}\";", prev, node.name);
                            break; // Only show one edge per node for simplicity
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
        }

        QueryCommand::Impact { symbol, depth } => {
            // Use DuckDB analytics for recursive impact analysis
            let analytics = analytics::Analytics::open(&root)
                .map_err(|e| format!("Failed to open analytics: {}", e))?;

            let impacts = analytics.impact_analysis(&symbol, depth)
                .map_err(|e| format!("Impact analysis query failed: {}", e))?;

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
                println!("{:<35} {:>6} {:>6} {:>6} {:>6}", "FILE", "TOTAL", "FUNCS", "PUB", "TYPES");
                
                if let Ok(file_stats) = analytics.file_statistics() {
                    for fs in file_stats.iter().take(15) {
                        let file = if fs.file_path.len() > 33 {
                            format!("...{}", &fs.file_path[fs.file_path.len() - 30..])
                        } else {
                            fs.file_path.clone()
                        };
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
                        let name_display = if name.len() > 28 {
                            format!("{}...", &name[..25])
                        } else {
                            name
                        };
                        println!("{:<30} {:>10} {:>10}", name_display, out_degree, in_degree);
                    }
                }
            }
        }

        QueryCommand::Files => {
            let files = db.get_indexed_files()?;

            println!("Indexed files ({}):", files.len());
            println!("{}", "-".repeat(60));

            for file in files {
                println!("  {}", file);
            }
        }
    }

    Ok(())
}

/// Run semantic/text search.
fn run_search(query: &str, limit: i32, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Use hybrid search combining exact matches with FTS5 semantic search
    let results = db.hybrid_search(query, limit)?;

    if results.is_empty() {
        // Fallback to simple name search
        let symbols = db.find_symbols(query, limit)?;
        if symbols.is_empty() {
            eprintln!("No results found for '{}'", query);
            return Ok(());
        }
        
        // Convert to format with scores
        let results: Vec<_> = symbols.iter()
            .map(|s| (s, 0.5, "name"))
            .collect();
        
        print_search_results(&results, query, output)?;
        return Ok(());
    }

    // Convert references for printing
    let results_ref: Vec<_> = results.iter()
        .map(|(s, score, match_type)| (s, *score, match_type.as_str()))
        .collect();
    
    print_search_results(&results_ref, query, output)?;

    Ok(())
}

/// Print search results in the specified format.
fn print_search_results(
    results: &[(&db::Symbol, f64, &str)],
    query: &str,
    output: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if output == "json" {
        let json_results: Vec<_> = results
            .iter()
            .map(|(s, score, match_type)| {
                serde_json::json!({
                    "name": s.name,
                    "kind": s.kind.as_str(),
                    "file": s.file_path,
                    "line": s.line_start,
                    "signature": s.signature,
                    "brief": s.brief,
                    "relevance": format!("{:.2}", score),
                    "match_type": match_type,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_results)?);
    } else {
        println!("Search results for '{}' ({} matches):", query, results.len());
        println!("{}", "-".repeat(75));
        println!("{:<40} {:<8} {:<6} {}", "SYMBOL", "KIND", "SCORE", "FILE");
        println!("{}", "-".repeat(75));

        for (symbol, score, match_type) in results {
            let name = if symbol.name.len() > 38 {
                format!("{}...", &symbol.name[..35])
            } else {
                symbol.name.clone()
            };

            let file = if symbol.file_path.len() > 25 {
                format!("...{}", &symbol.file_path[symbol.file_path.len() - 22..])
            } else {
                symbol.file_path.clone()
            };

            let score_display = format!("{:.0}%", score * 100.0);
            let kind_display = format!("{}", symbol.kind.as_str());

            println!(
                "{:<40} {:<8} {:<6} {}:{}",
                name,
                kind_display,
                score_display,
                file,
                symbol.line_start
            );

            // Show match type indicator
            let indicator = match *match_type {
                "exact" => "[exact]",
                "semantic" => "[semantic]",
                _ => "[name]",
            };

            if let Some(sig) = &symbol.signature {
                let sig_short = if sig.len() > 70 {
                    format!("{}...", &sig[..67])
                } else {
                    sig.clone()
                };
                println!("  {} {}", indicator, sig_short);
            }

            if let Some(brief) = &symbol.brief {
                let brief_short = if brief.len() > 70 {
                    format!("{}...", &brief[..67])
                } else {
                    brief.clone()
                };
                println!("  # {}", brief_short);
            }
            println!();
        }
    }

    Ok(())
}

/// Get source code for a symbol.
fn run_source(symbol: &str) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Try to find by exact ID first, then by name
    let source = if let Some(src) = db.get_source(symbol)? {
        Some((symbol.to_string(), src))
    } else {
        let symbols = db.find_symbols(symbol, 1)?;
        symbols.first().and_then(|s| {
            db.get_source(&s.id).ok().flatten().map(|src| (s.id.clone(), src))
        })
    };

    match source {
        Some((id, src)) => {
            println!("// Source: {}", id);
            println!("{}", src);
        }
        None => {
            eprintln!("Symbol '{}' not found", symbol);
        }
    }

    Ok(())
}

/// Explain a symbol with its relationships.
fn run_explain(symbol: &str) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Find the symbol
    let symbols = db.find_symbols(symbol, 1)?;
    let sym = symbols.first().ok_or("Symbol not found")?;

    println!("Symbol: {}", sym.name);
    println!("{}", "=".repeat(60));
    println!("Kind:       {}", sym.kind.as_str());
    println!("File:       {}:{}", sym.file_path, sym.line_start);
    println!("Visibility: {}", sym.visibility.as_str());

    if let Some(ref sig) = sym.signature {
        println!("\nSignature:");
        println!("  {}", sig);
    }

    if let Some(ref brief) = sym.brief {
        println!("\nDescription:");
        println!("  {}", brief);
    }

    // Show callers
    let callers = db.get_incoming_edges(&sym.name)?;
    if !callers.is_empty() {
        println!("\nCalled by ({}):", callers.len());
        for edge in callers.iter().take(10) {
            if let Some(caller) = db.get_symbol(&edge.source_id)? {
                println!("  {} ({}:{})", caller.name, caller.file_path, edge.line.unwrap_or(0));
            }
        }
        if callers.len() > 10 {
            println!("  ... and {} more", callers.len() - 10);
        }
    }

    // Show dependencies
    let deps = db.get_outgoing_edges(&sym.id)?;
    if !deps.is_empty() {
        println!("\nCalls ({}):", deps.len());
        for edge in deps.iter().take(10) {
            println!("  {} [{}]", edge.target_name, edge.kind.as_str());
        }
        if deps.len() > 10 {
            println!("  ... and {} more", deps.len() - 10);
        }
    }

    Ok(())
}
