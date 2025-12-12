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

        QueryCommand::Graph { start, depth: _, output } => {
            // Find the starting symbol
            let symbols = db.find_symbols(&start, 1)?;
            let sym = symbols.first().ok_or("Symbol not found")?;

            let edges = db.get_outgoing_edges(&sym.id)?;

            if output == "json" {
                let graph = serde_json::json!({
                    "root": sym.id,
                    "nodes": [{
                        "name": sym.name,
                        "file": sym.file_path,
                        "kind": sym.kind.as_str(),
                    }],
                    "edges": edges.iter().map(|e| {
                        serde_json::json!({
                            "from": sym.name,
                            "to": e.target_name,
                            "kind": e.kind.as_str(),
                        })
                    }).collect::<Vec<_>>()
                });
                println!("{}", serde_json::to_string_pretty(&graph)?);
            } else {
                println!("Call graph from '{}':", start);
                println!("{}", "-".repeat(60));
                println!("{} ({})", sym.name, sym.file_path);
                for edge in edges {
                    println!("  -> {} [{}]", edge.target_name, edge.kind.as_str());
                }
            }
        }

        QueryCommand::Impact { symbol, depth: _ } => {
            let edges = db.get_incoming_edges(&symbol)?;

            if edges.is_empty() {
                eprintln!("No impact detected for changes to '{}'", symbol);
                return Ok(());
            }

            println!("Impact analysis for '{}':", symbol);
            println!("The following would be affected by changes:");
            println!("{}", "-".repeat(60));

            for edge in edges {
                let source = db.get_symbol(&edge.source_id)?;
                if let Some(s) = source {
                    println!("  {} ({})", s.name, s.file_path);
                }
            }
        }

        QueryCommand::Stats => {
            let stats = db.get_stats()?;

            println!("Codebase Statistics");
            println!("{}", "=".repeat(40));
            println!("Files indexed:  {}", stats.files);
            println!("Total symbols:  {}", stats.symbols);
            println!("  - Functions:  {}", stats.functions);
            println!("  - Structs:    {}", stats.structs);
            println!("  - Enums:      {}", stats.enums);
            println!("  - Traits:     {}", stats.traits);
            println!("Total edges:    {}", stats.edges);
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

    // For now, just do a simple text search
    let symbols = db.find_symbols(query, limit)?;

    if symbols.is_empty() {
        eprintln!("No results found for '{}'", query);
        return Ok(());
    }

    if output == "json" {
        let results: Vec<_> = symbols
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "kind": s.kind.as_str(),
                    "file": s.file_path,
                    "line": s.line_start,
                    "signature": s.signature,
                    "brief": s.brief,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        println!("Search results for '{}':", query);
        println!("{}", "-".repeat(70));

        for symbol in symbols {
            println!(
                "{} ({}) - {}:{}",
                symbol.name,
                symbol.kind.as_str(),
                symbol.file_path,
                symbol.line_start
            );
            if let Some(sig) = &symbol.signature {
                println!("  {}", sig);
            }
            if let Some(brief) = &symbol.brief {
                println!("  # {}", brief);
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
