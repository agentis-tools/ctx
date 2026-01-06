mod analytics;
mod audit;
mod cli;
mod db;
mod default_ignores;
mod diff;
mod embeddings;
mod formatter;
mod index;
#[cfg(feature = "mcp")]
mod mcp;
mod output;
mod parser;
mod shell;
mod smart;
mod tokens;
mod tree;
mod walker;

use std::env;
use std::process;
use std::time::Instant;

use clap::Parser;

use cli::{Args, Command, QueryCommand};
use output::{generate_context, stream_context};
#[allow(unused_imports)]
use std::collections::HashMap;
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
        Some(Command::Index {
            watch,
            verbose,
            force,
            parallel,
            no_gitignore,
            no_default_ignores,
            ignore_patterns,
            include_patterns,
        }) => run_index(
            watch,
            verbose,
            force,
            parallel,
            no_gitignore,
            no_default_ignores,
            ignore_patterns,
            include_patterns,
        ),
        Some(Command::Query { query }) => run_query(query),
        Some(Command::Search {
            query,
            limit,
            output,
        }) => run_search(&query, limit, &output),
        Some(Command::Source { symbol, file, kind }) => {
            run_source(&symbol, file.as_deref(), kind.as_deref())
        }
        Some(Command::Explain { symbol, file, kind }) => {
            run_explain(&symbol, file.as_deref(), kind.as_deref())
        }
        Some(Command::Embed {
            force,
            verbose,
            batch_size,
            openai,
            watch,
        }) => {
            if watch {
                run_embed_watch(verbose, batch_size, openai)
            } else {
                run_embed(force, verbose, batch_size, openai)
            }
        }
        Some(Command::Semantic {
            query,
            limit,
            output,
            openai,
        }) => run_semantic(&query, limit, &output, openai),
        Some(Command::Complexity {
            threshold,
            warnings_only,
            output,
        }) => run_complexity(threshold, warnings_only, &output),
        Some(Command::Duplicates {
            similarity,
            min_lines,
            output,
        }) => run_duplicates(similarity, min_lines, &output),
        Some(Command::Graph {
            output,
            by_file,
            filter,
            depth,
        }) => run_graph(&output, by_file, filter, depth),
        Some(Command::Smart {
            task,
            max_tokens,
            depth,
            top,
            explain,
            dry_run,
            openai,
            format,
            show_sizes,
            no_tree,
        }) => run_smart(
            &task,
            max_tokens,
            depth,
            top,
            explain,
            dry_run,
            openai,
            &format,
            show_sizes,
            no_tree,
        ),
        Some(Command::Diff {
            revision,
            max_tokens,
            depth,
            changes_only,
            staged,
            summary,
            format,
            show_sizes,
            no_tree,
        }) => run_diff(
            &revision,
            max_tokens,
            depth,
            changes_only,
            staged,
            summary,
            &format,
            show_sizes,
            no_tree,
        ),
        Some(Command::Review {
            pr,
            repo,
            include_comments,
            max_tokens,
            depth,
            changes_only,
            summary,
            format,
            show_sizes,
            no_tree,
        }) => run_review(
            &pr,
            repo.as_deref(),
            include_comments,
            max_tokens,
            depth,
            changes_only,
            summary,
            &format,
            show_sizes,
            no_tree,
        ),
        Some(Command::Audit {
            output_format,
            min_score,
            categories,
            incremental,
        }) => run_audit(&output_format, min_score, categories, incremental),
        Some(Command::Shell {
            history,
            no_history,
            vi,
        }) => run_shell(history, no_history, vi),
        #[cfg(feature = "mcp")]
        Some(Command::Serve { mcp }) => run_serve(mcp),
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

    // Parse encoding
    let encoding = tokens::Encoding::from_str(&args.encoding).ok_or_else(|| {
        format!(
            "Invalid encoding '{}'. Valid options: cl100k_base, o200k_base, p50k_base",
            args.encoding
        )
    })?;

    // Handle --count-only mode: just count tokens without output
    if args.count_only {
        return run_count_only(&root, &entries, encoding, args.stats, start);
    }

    // Handle --max-tokens mode: filter files to fit within budget
    let entries = if let Some(max_tokens) = args.max_tokens {
        filter_files_by_tokens(&root, &entries, max_tokens, encoding)?
    } else {
        entries
    };

    if entries.is_empty() {
        eprintln!("No files fit within the token budget.");
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

/// Run --count-only mode: count tokens in files without generating output.
fn run_count_only(
    root: &std::path::Path,
    entries: &[walker::FileEntry],
    encoding: tokens::Encoding,
    show_stats: bool,
    start: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut total_tokens = 0usize;
    let mut total_chars = 0usize;
    let mut file_count = 0usize;
    let mut skipped_count = 0usize;

    for entry in entries {
        let path = root.join(&entry.relative_path);
        // Use lossy read to match read_file_content behavior in output.rs
        match std::fs::read(&path) {
            Ok(bytes) => {
                let content = String::from_utf8_lossy(&bytes);
                let token_count = tokens::count_tokens_with_encoding(&content, encoding)?;
                total_tokens += token_count;
                total_chars += content.chars().count(); // Use char count, not byte length
                file_count += 1;
            }
            Err(e) => {
                eprintln!("Warning: could not read {}: {}", entry.relative_path.display(), e);
                skipped_count += 1;
            }
        }
    }

    // Output token count summary
    println!("Files: {}", file_count);
    if skipped_count > 0 {
        println!("Skipped (unreadable): {}", skipped_count);
    }
    println!("Characters (UTF-8): {}", total_chars);
    println!("Tokens ({}): {}", encoding.as_str(), total_tokens);

    if show_stats {
        let elapsed = start.elapsed();
        eprintln!("Counted in {:.2?}", elapsed);
    }

    Ok(())
}

/// Filter files to fit within a token budget.
fn filter_files_by_tokens(
    root: &std::path::Path,
    entries: &[walker::FileEntry],
    max_tokens: usize,
    encoding: tokens::Encoding,
) -> Result<Vec<walker::FileEntry>, Box<dyn std::error::Error>> {
    // Count tokens for each file
    let mut file_tokens: Vec<(usize, &walker::FileEntry)> = Vec::new();

    for entry in entries {
        let path = root.join(&entry.relative_path);
        // Use lossy read to match read_file_content behavior in output.rs
        if let Ok(bytes) = std::fs::read(&path) {
            let content = String::from_utf8_lossy(&bytes);
            let token_count = tokens::count_tokens_with_encoding(&content, encoding)?;
            file_tokens.push((token_count, entry));
        }
    }

    // Select files that fit within budget (greedy, in order)
    let mut selected = Vec::new();
    let mut total = 0usize;
    let mut omitted = 0usize;

    for (tokens, entry) in file_tokens {
        if total + tokens <= max_tokens {
            total += tokens;
            selected.push(entry.clone());
        } else {
            omitted += 1;
        }
    }

    if omitted > 0 {
        eprintln!(
            "Token budget: {} files included ({} tokens), {} files omitted",
            selected.len(),
            total,
            omitted
        );
    }

    Ok(selected)
}

/// Generate embeddings for all symbols.
fn run_embed(
    force: bool,
    verbose: bool,
    batch_size: usize,
    use_openai: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use embeddings::{embed_missing_symbols, EmbeddingProvider};

    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Create provider based on flag
    let provider: Box<dyn EmbeddingProvider> = if use_openai {
        use embeddings::openai::OpenAIProvider;
        let p = OpenAIProvider::from_env().map_err(|_| {
            "OPENAI_API_KEY environment variable not set.\n\
             Set it with: export OPENAI_API_KEY=sk-..."
        })?;
        Box::new(p)
    } else {
        use embeddings::local::LocalProvider;
        eprintln!("Initializing local embedding model (first run downloads ~90MB)...");
        let p =
            LocalProvider::new().map_err(|e| format!("Failed to initialize local model: {}", e))?;
        Box::new(p)
    };

    if verbose {
        println!(
            "Using embedding provider: {} (dim={})",
            provider.name(),
            provider.dimension()
        );
    }

    // Optionally clear existing embeddings
    if force {
        let deleted = db.delete_embeddings(provider.name(), None)?;
        if verbose {
            println!("Deleted {} existing embeddings", deleted);
        }
    }

    // Check current state
    let total_symbols = db.get_stats()?.symbols;
    let existing_embeddings = db.count_embeddings()?;

    if verbose {
        println!("Total symbols: {}", total_symbols);
        println!("Existing embeddings: {}", existing_embeddings);
    }

    if existing_embeddings >= total_symbols && !force {
        println!("All symbols already have embeddings. Use --force to re-embed.");
        return Ok(());
    }

    println!(
        "Generating embeddings for {} symbols...",
        total_symbols - existing_embeddings
    );

    let start = Instant::now();
    let progress_callback = |done: usize, _total: usize| {
        if verbose {
            eprint!("\rEmbedded {} symbols...", done);
        }
    };

    let embedded =
        embed_missing_symbols(&db, provider.as_ref(), batch_size, Some(&progress_callback))?;

    if verbose {
        eprintln!();
    }

    let elapsed = start.elapsed();
    println!(
        "Embedded {} symbols in {:.2}s ({:.1} symbols/sec)",
        embedded,
        elapsed.as_secs_f64(),
        embedded as f64 / elapsed.as_secs_f64()
    );

    Ok(())
}

/// Watch for index changes and auto-embed new symbols.
fn run_embed_watch(
    verbose: bool,
    batch_size: usize,
    use_openai: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use embeddings::{embed_missing_symbols, EmbeddingProvider};
    use notify::RecursiveMode;
    use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    let root = env::current_dir()?;
    let ctx_dir = root.join(".ctx");
    let _db_path = ctx_dir.join("codebase.sqlite");

    // Create provider based on flag
    let provider: Box<dyn EmbeddingProvider> = if use_openai {
        use embeddings::openai::OpenAIProvider;
        let p = OpenAIProvider::from_env().map_err(|_| {
            "OPENAI_API_KEY environment variable not set.\n\
             Set it with: export OPENAI_API_KEY=sk-..."
        })?;
        Box::new(p)
    } else {
        use embeddings::local::LocalProvider;
        eprintln!("Initializing local embedding model (first run downloads ~90MB)...");
        let p =
            LocalProvider::new().map_err(|e| format!("Failed to initialize local model: {}", e))?;
        Box::new(p)
    };

    println!(
        "Using embedding provider: {} (dim={})",
        provider.name(),
        provider.dimension()
    );

    // Do initial embedding
    {
        let db = index::open_database(&root)?;
        let total_symbols = db.get_stats()?.symbols;
        let existing = db.count_embeddings()?;

        if existing < total_symbols {
            println!(
                "Initial embedding: {} symbols missing embeddings...",
                total_symbols - existing
            );
            let embedded = embed_missing_symbols(&db, provider.as_ref(), batch_size, None)?;
            println!("Embedded {} symbols", embedded);
        } else {
            println!("All {} symbols already have embeddings", total_symbols);
        }
    }

    // Set up file watcher on the database file
    let (tx, rx) = channel();

    let mut debouncer = new_debouncer(Duration::from_secs(2), tx)
        .map_err(|e| format!("Failed to create watcher: {}", e))?;

    // Watch the .ctx directory for database changes
    if ctx_dir.exists() {
        debouncer
            .watcher()
            .watch(&ctx_dir, RecursiveMode::NonRecursive)
            .map_err(|e| format!("Failed to watch .ctx directory: {}", e))?;
    }

    println!("\nWatching for index changes... (press Ctrl+C to stop)");
    println!("Tip: Run 'ctx index --watch' in another terminal to auto-index file changes");

    // Process database change events
    loop {
        match rx.recv() {
            Ok(Ok(events)) => {
                // Check if the database file changed
                let db_changed = events.iter().any(|e| {
                    e.kind == DebouncedEventKind::Any
                        && e.path
                            .file_name()
                            .map(|n| n == "codebase.sqlite")
                            .unwrap_or(false)
                });

                if db_changed {
                    // Re-open database and check for new symbols
                    match index::open_database(&root) {
                        Ok(db) => {
                            let total = db.get_stats().map(|s| s.symbols).unwrap_or(0);
                            let existing = db.count_embeddings().unwrap_or(0);

                            if existing < total {
                                let missing = total - existing;
                                if verbose {
                                    eprintln!("\nIndex updated: {} new symbols to embed", missing);
                                }

                                match embed_missing_symbols(
                                    &db,
                                    provider.as_ref(),
                                    batch_size,
                                    None,
                                ) {
                                    Ok(embedded) => {
                                        if embedded > 0 {
                                            if verbose {
                                                eprintln!("Embedded {} symbols", embedded);
                                            } else {
                                                eprint!("+{} ", embedded);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("\nWarning: failed to embed: {}", e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if verbose {
                                eprintln!("\nWarning: failed to open database: {}", e);
                            }
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("\nWatch error: {:?}", e);
            }
            Err(e) => {
                eprintln!("\nChannel error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

/// Run semantic search using embeddings.
fn run_semantic(
    query: &str,
    limit: usize,
    output: &str,
    use_openai: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use embeddings::{semantic_search, EmbeddingProvider};

    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Check if we have embeddings
    let embedding_count = db.count_embeddings()?;
    if embedding_count == 0 {
        eprintln!("No embeddings found. Run 'ctx embed' first to generate embeddings.");
        return Ok(());
    }

    // Create provider based on flag
    let provider: Box<dyn EmbeddingProvider> = if use_openai {
        use embeddings::openai::OpenAIProvider;
        let p = OpenAIProvider::from_env().map_err(|_| {
            "OPENAI_API_KEY environment variable not set.\n\
             Set it with: export OPENAI_API_KEY=sk-..."
        })?;
        Box::new(p)
    } else {
        use embeddings::local::LocalProvider;
        let p =
            LocalProvider::new().map_err(|e| format!("Failed to initialize local model: {}", e))?;
        Box::new(p)
    };

    // Check for embedding dimension mismatch
    let query_dim = provider.dimension();
    if let Ok(metadata) = db.get_embedding_metadata() {
        for (stored_provider, _model, stored_dim, count) in &metadata {
            let stored_dim = *stored_dim as usize;
            if stored_dim != query_dim {
                eprintln!(
                    "Warning: Embedding dimension mismatch detected!"
                );
                eprintln!(
                    "  Stored: {} embeddings from '{}' with dimension {}",
                    count, stored_provider, stored_dim
                );
                eprintln!(
                    "  Query:  Using '{}' with dimension {}",
                    provider.name(), query_dim
                );
                eprintln!(
                    "  Results may be inaccurate. Re-run 'ctx embed{}' to regenerate embeddings.",
                    if use_openai { " --openai" } else { "" }
                );
                eprintln!();
            }
        }
    }

    // Embed the query
    let query_embedding = provider.embed(query)?;

    // Search for similar symbols
    let results = semantic_search(&db, &query_embedding, limit)?;

    if results.is_empty() {
        eprintln!("No results found for '{}'", query);
        return Ok(());
    }

    if output == "json" {
        let json_results: Vec<_> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "symbol_id": r.symbol_id,
                    "name": r.name,
                    "kind": r.kind,
                    "file": r.file_path,
                    "line": r.line,
                    "score": format!("{:.4}", r.score),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_results)?);
    } else {
        println!(
            "Semantic search for '{}' ({} results):",
            query,
            results.len()
        );
        println!("{}", "-".repeat(80));
        println!("{:<35} {:<10} {:<8} {}", "SYMBOL", "KIND", "SCORE", "FILE");
        println!("{}", "-".repeat(80));

        for result in &results {
            let name = truncate_str(&result.name, 33);
            let file = truncate_path(&result.file_path, 25);

            let score_display = format!("{:.2}%", result.score * 100.0);

            println!(
                "{:<35} {:<10} {:<8} {}:{}",
                name, result.kind, score_display, file, result.line
            );
        }
    }

    Ok(())
}

/// Run the index command.
#[allow(clippy::too_many_arguments)]
fn run_index(
    watch: bool,
    verbose: bool,
    force: bool,
    parallel: bool,
    no_gitignore: bool,
    no_default_ignores: bool,
    ignore_patterns: Vec<String>,
    include_patterns: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;

    // Handle force reindex by removing existing database
    if force {
        let db_path = root.join(index::CTX_DIR).join(index::DB_FILE);
        if db_path.exists() {
            eprintln!("Removing existing database for full reindex...");
            std::fs::remove_file(&db_path)?;
        }
    }

    // Build walker configuration from CLI flags
    let make_walker_config = || walker::WalkerConfig {
        use_gitignore: !no_gitignore,
        use_default_ignores: !no_default_ignores,
        custom_ignores: ignore_patterns.clone(),
        include_patterns: include_patterns.clone(),
    };

    if parallel {
        eprintln!("Indexing codebase (parallel mode)...");
    } else {
        eprintln!("Indexing codebase...");
    }

    let mut indexer = index::Indexer::with_config(&root, verbose, make_walker_config())?;
    let result = if parallel {
        indexer.index_parallel()?
    } else {
        indexer.index()?
    };

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

    // Watch mode
    if watch {
        eprintln!("\nWatching for changes... (Ctrl+C to stop)");
        index::watch::watch_and_index(&root, verbose, make_walker_config())?;
    }

    Ok(())
}

// --- Query subcommand helpers ---

/// Handle 'query find' subcommand.
fn query_find(
    db: &db::Database,
    pattern: &str,
    limit: i32,
    kind: Option<String>,
    file: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let symbols = db.find_symbols_filtered(
        pattern,
        limit,
        file.as_deref(),
        kind.as_deref(),
    )?;

    if symbols.is_empty() {
        eprintln!("No symbols found matching '{}'", pattern);
        if file.is_some() || kind.is_some() {
            eprintln!("Try removing filters to see all matches");
        }
        return Ok(());
    }

    println!(
        "{:<40} {:<12} {:<10} {}",
        "SYMBOL", "KIND", "VISIBILITY", "FILE"
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
fn query_callers(
    db: &db::Database,
    function: &str,
    file_pattern: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
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
        let is_duplicate = edges.iter().any(|e| {
            e.source_id == edge.source_id && e.line == edge.line
        });
        if is_duplicate {
            continue;
        }
        
        // Determine if this edge likely refers to our symbol
        let likely_match = if let Some(ref ctx) = edge.context {
            // Check if context contains our qualified name or parent type
            ctx.contains(qualified_name)
                || parent_prefix.as_ref().map_or(false, |p| ctx.contains(p))
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

    println!(
        "Functions that call '{}' ({}):",
        sym.name, sym.file_path
    );
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
) -> Result<(), Box<dyn std::error::Error>> {
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
        eprintln!(
            "No dependencies found for '{}' ({})",
            symbol, sym.file_path
        );
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

/// Truncate a string with ellipsis, respecting UTF-8 char boundaries.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let target = max.saturating_sub(3);
        let mut end = target;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Truncate a path from the beginning, respecting UTF-8 char boundaries.
fn truncate_path(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let target = s.len() - max + 3;
        let mut start = target;
        while start < s.len() && !s.is_char_boundary(start) {
            start += 1;
        }
        format!("...{}", &s[start..])
    }
}

/// Run query subcommands.
fn run_query(query: QueryCommand) -> Result<(), Box<dyn std::error::Error>> {
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
            let analytics = analytics::Analytics::open(&root)
                .map_err(|e| format!("Failed to open analytics: {}", e))?;

            let nodes = analytics
                .call_graph(&start, depth)
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
            Ok(())
        }

        QueryCommand::Impact { symbol, depth } => {
            // Use DuckDB analytics for recursive impact analysis
            let analytics = analytics::Analytics::open(&root)
                .map_err(|e| format!("Failed to open analytics: {}", e))?;

            let impacts = analytics
                .impact_analysis(&symbol, depth)
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
        let results: Vec<_> = symbols.iter().map(|s| (s, 0.5, "name")).collect();

        print_search_results(&results, query, output)?;
        return Ok(());
    }

    // Convert references for printing
    let results_ref: Vec<_> = results
        .iter()
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
        println!(
            "Search results for '{}' ({} matches):",
            query,
            results.len()
        );
        println!("{}", "-".repeat(75));
        println!("{:<40} {:<8} {:<6} {}", "SYMBOL", "KIND", "SCORE", "FILE");
        println!("{}", "-".repeat(75));

        for (symbol, score, match_type) in results {
            let name = truncate_str(&symbol.name, 38);
            let file = truncate_path(&symbol.file_path, 25);

            let score_display = format!("{:.0}%", score * 100.0);
            let kind_display = format!("{}", symbol.kind.as_str());

            println!(
                "{:<40} {:<8} {:<6} {}:{}",
                name, kind_display, score_display, file, symbol.line_start
            );

            // Show match type indicator
            let indicator = match *match_type {
                "exact" => "[exact]",
                "semantic" => "[semantic]",
                _ => "[name]",
            };

            if let Some(sig) = &symbol.signature {
                let sig_short = truncate_str(sig, 70);
                println!("  {} {}", indicator, sig_short);
            }

            if let Some(brief) = &symbol.brief {
                let brief_short = truncate_str(brief, 70);
                println!("  # {}", brief_short);
            }
            println!();
        }
    }

    Ok(())
}

/// Get source code for a symbol.
fn run_source(
    symbol: &str,
    file_pattern: Option<&str>,
    kind_filter: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Try to find by exact ID first
    if let Some(src) = db.get_source(symbol)? {
        println!("// Source: {}", symbol);
        println!("{}", src);
        return Ok(());
    }

    // Search with filters - get more results for disambiguation
    let symbols = db.find_symbols_filtered(symbol, 100, file_pattern, kind_filter)?;

    if symbols.is_empty() {
        eprintln!("Symbol '{}' not found", symbol);
        if file_pattern.is_some() || kind_filter.is_some() {
            eprintln!("Try removing filters or use 'ctx query find {}' to see all matches", symbol);
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
        eprintln!("\nExample: ctx source {} --file \"{}\"", symbol, symbols[0].file_path);
        return Ok(());
    }

    // Get the first matching symbol's source
    let sym = &symbols[0];
    match db.get_source(&sym.id)? {
        Some(src) => {
            println!("// Source: {}", sym.id);
            println!("{}", src);
        }
        None => {
            eprintln!("Source code not available for '{}'", sym.id);
        }
    }

    Ok(())
}

/// Explain a symbol with its relationships.
fn run_explain(
    symbol: &str,
    file_pattern: Option<&str>,
    kind_filter: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Search with filters - get more results for disambiguation
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
            "\nExample: ctx explain {} --file \"{}\"",
            symbol, symbols[0].file_path
        );
        return Ok(());
    }

    let sym = &symbols[0];

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
                println!(
                    "  {} ({}:{})",
                    caller.name,
                    caller.file_path,
                    edge.line.unwrap_or(0)
                );
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

/// Analyze code complexity and flag high fan-out functions.
fn run_complexity(
    threshold: i64,
    warnings_only: bool,
    output: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;
    let analytics = analytics::Analytics::open(&root)
        .map_err(|e| format!("Failed to open analytics: {}", e))?;

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
            "{:<35} {:>8} {:>8} {:>8} {:<10} {}",
            "FUNCTION", "FAN-OUT", "FAN-IN", "SCORE", "SEVERITY", "FILE"
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

/// Detect duplicate or similar code blocks.
fn run_duplicates(
    similarity_threshold: u32,
    min_lines: u32,
    output: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    let duplicates = db.find_duplicates(similarity_threshold, min_lines)?;

    if duplicates.is_empty() {
        println!(
            "No duplicate code blocks found (threshold: {}%, min lines: {}).",
            similarity_threshold, min_lines
        );
        return Ok(());
    }

    if output == "json" {
        let json_results: Vec<_> = duplicates
            .iter()
            .map(|d| {
                serde_json::json!({
                    "symbol1": {
                        "name": d.name1,
                        "file": d.file1,
                        "line": d.line1,
                    },
                    "symbol2": {
                        "name": d.name2,
                        "file": d.file2,
                        "line": d.line2,
                    },
                    "similarity": d.similarity,
                    "lines": d.lines,
                    "hash": d.hash,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_results)?);
    } else {
        println!(
            "Duplicate Code Detection (similarity >= {}%, min {} lines)",
            similarity_threshold, min_lines
        );
        println!("{}", "=".repeat(100));

        for (i, dup) in duplicates.iter().enumerate() {
            println!(
                "\n{}. Similarity: {:.1}% ({} lines)",
                i + 1,
                dup.similarity,
                dup.lines
            );
            println!("   {} ({}:{})", dup.name1, dup.file1, dup.line1);
            println!("   {} ({}:{})", dup.name2, dup.file2, dup.line2);
        }

        println!("{}", "-".repeat(100));
        println!("Found {} duplicate pairs", duplicates.len());
    }

    Ok(())
}

/// Run smart context selection.
#[allow(clippy::too_many_arguments)]
fn run_smart(
    task: &str,
    max_tokens: usize,
    depth: i32,
    top: usize,
    explain: bool,
    dry_run: bool,
    use_openai: bool,
    format: &cli::OutputFormat,
    show_sizes: bool,
    no_tree: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use embeddings::EmbeddingProvider;
    use smart::{format_dry_run, format_explain, smart_context, SmartConfig};

    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Check if we have embeddings
    let embedding_count = db.count_embeddings()?;
    if embedding_count == 0 {
        eprintln!("No embeddings found. Run 'ctx embed' first to generate embeddings.");
        return Ok(());
    }

    // Create embedding provider
    let provider: Box<dyn EmbeddingProvider> = if use_openai {
        use embeddings::openai::OpenAIProvider;
        let p = OpenAIProvider::from_env().map_err(|_| {
            "OPENAI_API_KEY environment variable not set.\n\
             Set it with: export OPENAI_API_KEY=sk-..."
        })?;
        Box::new(p)
    } else {
        use embeddings::local::LocalProvider;
        let p =
            LocalProvider::new().map_err(|e| format!("Failed to initialize local model: {}", e))?;
        Box::new(p)
    };

    // Check for embedding dimension mismatch
    let query_dim = provider.dimension();
    if let Ok(metadata) = db.get_embedding_metadata() {
        for (stored_provider, _model, stored_dim, count) in &metadata {
            let stored_dim = *stored_dim as usize;
            if stored_dim != query_dim {
                eprintln!("Warning: Embedding dimension mismatch detected!");
                eprintln!(
                    "  Stored: {} embeddings from '{}' with dimension {}",
                    count, stored_provider, stored_dim
                );
                eprintln!(
                    "  Query:  Using '{}' with dimension {}",
                    provider.name(),
                    query_dim
                );
                eprintln!(
                    "  Results may be inaccurate. Re-run 'ctx embed{}' to regenerate embeddings.",
                    if use_openai { " --openai" } else { "" }
                );
                eprintln!();
            }
        }
    }

    // Open analytics for call graph expansion
    let analytics = analytics::Analytics::open(&root)
        .map_err(|e| format!("Failed to open analytics: {}", e))?;

    // Configure and run smart context selection
    // For dry-run, don't limit tokens - show all relevant files
    let effective_max_tokens = if dry_run { usize::MAX } else { max_tokens };
    let config = SmartConfig {
        max_tokens: effective_max_tokens,
        depth,
        top,
        encoding: tokens::Encoding::default(),
    };

    eprintln!("Analyzing task: \"{}\"...", task);

    let result = smart_context(&db, &analytics, provider.as_ref(), task, config)?;

    if result.selected_files.is_empty() {
        eprintln!("No relevant files found for: \"{}\"", task);
        std::process::exit(2);
    }

    // Handle dry-run mode
    if dry_run {
        println!("{}", format_dry_run(&result));
        return Ok(());
    }

    // Handle explain mode (show reasoning then context)
    if explain {
        eprintln!("{}", format_explain(&result));
    }

    eprintln!(
        "Selected {} files ({} tokens){}",
        result.selected_files.len(),
        result.total_tokens,
        if result.truncated {
            format!(", {} omitted", result.omitted_count)
        } else {
            String::new()
        }
    );

    // Convert selected files to FileEntry format for context generation
    let entries: Vec<walker::FileEntry> = result
        .selected_files
        .iter()
        .map(|f| {
            let relative_path = std::path::PathBuf::from(&f.path);
            let absolute_path = root.join(&relative_path);
            let size = std::fs::metadata(&absolute_path)
                .map(|m| m.len())
                .unwrap_or(0);
            walker::FileEntry {
                absolute_path,
                relative_path,
                size,
            }
        })
        .collect();

    // Generate context output
    let output_result = if entries.is_empty() {
        eprintln!("No files to include in context.");
        return Ok(());
    } else {
        output::stream_context(&root, &entries, format, !no_tree, show_sizes)?
    };

    eprintln!(
        "Generated context: {} files",
        output_result.file_count
    );

    Ok(())
}

/// Run diff-aware context generation.
#[allow(clippy::too_many_arguments)]
fn run_diff(
    revision: &str,
    max_tokens: usize,
    depth: i32,
    changes_only: bool,
    staged: bool,
    summary: bool,
    format: &cli::OutputFormat,
    show_sizes: bool,
    no_tree: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use diff::{diff_context, format_summary, DiffConfig, DiffError};

    let root = env::current_dir()?;

    // Check if index exists (for context expansion)
    let db = match index::open_database(&root) {
        Ok(db) => Some(db),
        Err(_) => {
            if !changes_only {
                eprintln!("Warning: No index found. Run 'ctx index' for context expansion.");
                eprintln!("Using --changes-only mode.\n");
            }
            None
        }
    };

    // Open analytics if we have a database
    let analytics = if let Some(ref db) = db {
        analytics::Analytics::open(&root).ok()
    } else {
        None
    };

    // Configure diff context
    let config = DiffConfig {
        max_tokens,
        depth,
        changes_only: changes_only || analytics.is_none(),
        staged,
        summary,
        encoding: tokens::Encoding::default(),
    };

    let revision_display = if staged { "staged changes" } else { revision };
    eprintln!("Analyzing {}...", revision_display);

    // Run diff context analysis
    let result = match (&db, &analytics) {
        (Some(db), Some(analytics)) => diff_context(revision, db, analytics, config),
        _ => {
            // Fallback: just get changed files without context expansion
            let changed = diff::get_changed_files(revision, staged)?;
            Ok(diff::DiffContext {
                revision: revision.to_string(),
                changed_files: changed.clone(),
                affected_symbols: Vec::new(),
                context_files: changed
                    .iter()
                    .filter(|f| f.change_type != diff::ChangeType::Deleted)
                    .map(|f| diff::ContextFile {
                        path: f.path.clone(),
                        priority: 1.0,
                        reason: diff::ContextReason::Changed(f.change_type),
                        token_count: 0,
                    })
                    .collect(),
                total_tokens: 0,
                truncated: false,
                omitted_count: 0,
            })
        }
    };

    let result = match result {
        Ok(r) => r,
        Err(DiffError::NoChanges) => {
            eprintln!("No changes found.");
            std::process::exit(2);
        }
        Err(DiffError::NotGitRepo) => {
            eprintln!("Error: Not a git repository.");
            std::process::exit(1);
        }
        Err(DiffError::InvalidRevision(r)) => {
            eprintln!("Error: Invalid revision '{}'", r);
            std::process::exit(1);
        }
        Err(e) => return Err(e.into()),
    };

    // Show summary if requested
    if summary {
        eprintln!("{}", format_summary(&result));
    }

    eprintln!(
        "Changed {} files, context {} files ({} tokens){}",
        result.changed_files.len(),
        result.context_files.len(),
        result.total_tokens,
        if result.truncated {
            format!(", {} omitted", result.omitted_count)
        } else {
            String::new()
        }
    );

    // Convert to FileEntry for output
    let entries: Vec<walker::FileEntry> = result
        .context_files
        .iter()
        .map(|f| {
            let relative_path = std::path::PathBuf::from(&f.path);
            let absolute_path = root.join(&relative_path);
            let size = std::fs::metadata(&absolute_path)
                .map(|m| m.len())
                .unwrap_or(0);
            walker::FileEntry {
                absolute_path,
                relative_path,
                size,
            }
        })
        .collect();

    if entries.is_empty() {
        eprintln!("No files to include in context.");
        return Ok(());
    }

    // Generate context output
    output::stream_context(&root, &entries, format, !no_tree, show_sizes)?;

    Ok(())
}

/// Run PR review context generation.
#[allow(clippy::too_many_arguments)]
fn run_review(
    pr: &str,
    repo: Option<&str>,
    include_comments: bool,
    max_tokens: usize,
    depth: i32,
    changes_only: bool,
    summary: bool,
    format: &cli::OutputFormat,
    show_sizes: bool,
    no_tree: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use diff::{format_pr_header, get_pr_info, DiffError};

    eprintln!("Fetching PR #{}...", pr);

    // Get PR info from GitHub
    let pr_info = match get_pr_info(pr, repo) {
        Ok(info) => info,
        Err(DiffError::InvalidRevision(r)) => {
            eprintln!("Error: {}", r);
            std::process::exit(3);
        }
        Err(DiffError::GitError(e)) if e.contains("not found") => {
            eprintln!("Error: GitHub CLI (gh) not found.");
            eprintln!("Install it from https://cli.github.com/");
            std::process::exit(1);
        }
        Err(e) => return Err(e.into()),
    };

    // Print PR header
    eprintln!("{}", format_pr_header(&pr_info, include_comments));

    // Get the diff for the PR's changes
    // We use the base..head format to get the PR diff
    let revision = format!("{}...{}", pr_info.base, pr_info.head);

    // Run diff with the PR revision
    run_diff(
        &revision,
        max_tokens,
        depth,
        changes_only,
        false, // not staged
        summary,
        format,
        show_sizes,
        no_tree,
    )
}

/// Run code quality audit.
fn run_audit(
    format: &str,
    min_score: Option<f32>,
    categories: Option<String>,
    _incremental: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use audit::{run_audit as do_audit, AuditConfig};

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
        incremental: false, // Not implemented yet
        min_score,
    };

    eprintln!("Running code quality audit...\n");

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

/// Run the interactive shell.
fn run_shell(
    history: Option<std::path::PathBuf>,
    no_history: bool,
    vi: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;

    let mut config = shell::ShellConfig::default();
    config.db_path = root;
    config.no_history = no_history;
    config.vi_mode = vi;

    if let Some(h) = history {
        config.history_file = h;
    }

    shell::run_shell(config)
}

/// Run the MCP server.
#[cfg(feature = "mcp")]
fn run_serve(mcp: bool) -> Result<(), Box<dyn std::error::Error>> {
    if !mcp {
        eprintln!("Error: Please specify --mcp flag to start the MCP server.");
        eprintln!("Usage: ctx serve --mcp");
        std::process::exit(1);
    }

    let root = env::current_dir()?;

    // Create a tokio runtime for the async MCP server
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async {
        mcp::run_mcp_server(root).await.map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            )) as Box<dyn std::error::Error>
        })
    })
}

// --- Graph output helpers ---

/// Output file dependencies in DOT format.
fn output_file_deps_dot(deps: &[(String, String, i64)]) {
    println!("digraph dependencies {{");
    println!("  rankdir=LR;");
    println!("  node [shape=box, style=filled, fillcolor=lightblue];");
    println!("  edge [color=gray];");

    let mut nodes: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (src, tgt, _) in deps {
        nodes.insert(src.clone());
        if tgt != "external" {
            nodes.insert(tgt.clone());
        }
    }

    for node in &nodes {
        let short_name = node.split('/').last().unwrap_or(node);
        println!("  \"{}\" [label=\"{}\"];", node, short_name);
    }

    for (src, tgt, count) in deps {
        if tgt != "external" {
            let weight = (*count as f64).sqrt().ceil() as i64;
            println!("  \"{}\" -> \"{}\" [penwidth={}];", src, tgt, weight.max(1));
        }
    }
    println!("}}");
}

/// Output file dependencies in Mermaid format.
fn output_file_deps_mermaid(deps: &[(String, String, i64)]) {
    println!("```mermaid");
    println!("graph LR");
    for (i, (src, tgt, _)) in deps.iter().enumerate() {
        if tgt != "external" {
            let src_short = src.split('/').last().unwrap_or(src);
            let tgt_short = tgt.split('/').last().unwrap_or(tgt);
            println!("  A{}[{}] --> B{}[{}]", i, src_short, i, tgt_short);
        }
    }
    println!("```");
}

/// Output file dependencies in JSON format.
fn output_file_deps_json(deps: &[(String, String, i64)]) -> Result<(), Box<dyn std::error::Error>> {
    let nodes: Vec<_> = deps
        .iter()
        .flat_map(|(src, tgt, _)| vec![src.clone(), tgt.clone()])
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .filter(|n| n != "external")
        .collect();

    let edges: Vec<_> = deps
        .iter()
        .filter(|(_, tgt, _)| tgt != "external")
        .map(|(src, tgt, count)| serde_json::json!({"source": src, "target": tgt, "weight": count}))
        .collect();

    let graph = serde_json::json!({"type": "file_dependencies", "nodes": nodes, "edges": edges});
    println!("{}", serde_json::to_string_pretty(&graph)?);
    Ok(())
}

/// Output call graph in DOT format.
fn output_call_graph_dot(graph: &[(String, String, String, String)]) {
    println!("digraph call_graph {{");
    println!("  rankdir=LR;");
    println!("  node [shape=ellipse];");

    let mut files: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (src_file, src_name, tgt_file, tgt_name) in graph {
        files
            .entry(src_file.clone())
            .or_default()
            .push(src_name.clone());
        files
            .entry(tgt_file.clone())
            .or_default()
            .push(tgt_name.clone());
    }

    for (i, (file, symbols)) in files.iter().enumerate() {
        let short_file = file.split('/').last().unwrap_or(file);
        println!("  subgraph cluster_{} {{", i);
        println!("    label=\"{}\";", short_file);
        println!("    style=filled;");
        println!("    color=lightgrey;");
        for sym in symbols.iter().collect::<std::collections::HashSet<_>>() {
            println!("    \"{}\";", sym);
        }
        println!("  }}");
    }

    for (_, src_name, _, tgt_name) in graph {
        println!("  \"{}\" -> \"{}\";", src_name, tgt_name);
    }
    println!("}}");
}

/// Output call graph in Mermaid format.
fn output_call_graph_mermaid(graph: &[(String, String, String, String)]) {
    println!("```mermaid");
    println!("graph LR");
    for (_, src_name, _, tgt_name) in graph {
        println!(
            "  {}[{}] --> {}[{}]",
            src_name.replace("::", "_"),
            src_name,
            tgt_name.replace("::", "_"),
            tgt_name
        );
    }
    println!("```");
}

/// Output call graph in JSON format.
fn output_call_graph_json(
    graph: &[(String, String, String, String)],
) -> Result<(), Box<dyn std::error::Error>> {
    let nodes: Vec<_> = graph
        .iter()
        .flat_map(|(sf, sn, tf, tn)| {
            vec![
                serde_json::json!({"name": sn, "file": sf}),
                serde_json::json!({"name": tn, "file": tf}),
            ]
        })
        .collect();

    let edges: Vec<_> = graph
        .iter()
        .map(|(_, src, _, tgt)| serde_json::json!({"source": src, "target": tgt}))
        .collect();

    let result = serde_json::json!({"type": "call_graph", "nodes": nodes, "edges": edges});
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

/// Generate a dependency graph visualization.
fn run_graph(
    output: &str,
    by_file: bool,
    filter: Option<String>,
    depth: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;
    let analytics = analytics::Analytics::open(&root)
        .map_err(|e| format!("Failed to open analytics: {}", e))?;

    let filter_files: Option<Vec<&str>> = filter
        .as_ref()
        .map(|f| f.split(',').map(|s| s.trim()).collect());

    if by_file {
        let deps = analytics.file_dependencies()?;
        let deps: Vec<_> = if let Some(ref filters) = filter_files {
            deps.into_iter()
                .filter(|(src, tgt, _)| filters.iter().any(|f| src.contains(f) || tgt.contains(f)))
                .collect()
        } else {
            deps
        };

        match output {
            "dot" => output_file_deps_dot(&deps),
            "mermaid" => output_file_deps_mermaid(&deps),
            "json" => output_file_deps_json(&deps)?,
            _ => {
                println!("File Dependency Graph");
                println!("{}", "=".repeat(80));
                for (src, tgt, count) in &deps {
                    println!("{} -> {} ({} calls)", src, tgt, count);
                }
            }
        }
    } else {
        let graph = analytics.full_call_graph(depth)?;
        let graph: Vec<_> = if let Some(ref filters) = filter_files {
            graph
                .into_iter()
                .filter(|(src_file, _, tgt_file, _)| {
                    filters
                        .iter()
                        .any(|f| src_file.contains(f) || tgt_file.contains(f))
                })
                .collect()
        } else {
            graph
        };

        match output {
            "dot" => output_call_graph_dot(&graph),
            "mermaid" => output_call_graph_mermaid(&graph),
            "json" => output_call_graph_json(&graph)?,
            _ => {
                println!("Symbol Call Graph");
                println!("{}", "=".repeat(80));
                for (src_file, src_name, tgt_file, tgt_name) in &graph {
                    println!("{} ({}) -> {} ({})", src_name, src_file, tgt_name, tgt_file);
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_str_ascii() {
        // No truncation needed
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello", 5), "hello");

        // Truncation needed
        assert_eq!(truncate_str("hello world", 8), "hello...");
        assert_eq!(truncate_str("abcdefghij", 7), "abcd...");
    }

    #[test]
    fn test_truncate_str_unicode() {
        // Box drawing (─ is 3 bytes)
        let box_line = "┌────────────────────┐";
        let result = truncate_str(box_line, 10);
        assert!(result.ends_with("..."));
        // Should not panic

        // Emoji (🎉 is 4 bytes)
        let emoji = "Hello 🎉🎊🎁 World";
        let result = truncate_str(emoji, 10);
        assert!(result.ends_with("..."));

        // Chinese (each char is 3 bytes)
        let chinese = "你好世界测试";
        let result = truncate_str(chinese, 8);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_path_ascii() {
        // No truncation needed
        assert_eq!(truncate_path("src/main.rs", 20), "src/main.rs");

        // Truncation needed - keeps end of path
        let result = truncate_path("/very/long/path/to/file.rs", 15);
        assert!(result.starts_with("..."));
        assert!(result.contains("file.rs"));
    }

    #[test]
    fn test_truncate_path_unicode() {
        // Path with Unicode
        let path = "/home/用户/项目/文件.rs";
        let result = truncate_path(path, 15);
        assert!(result.starts_with("..."));
        // Should not panic

        // Path with emoji folder names
        let path = "/home/📁/🎉/file.rs";
        let result = truncate_path(path, 12);
        assert!(result.starts_with("..."));
    }

    #[test]
    fn test_truncate_edge_cases() {
        // Very short max
        assert_eq!(truncate_str("hello", 3), "...");
        assert_eq!(truncate_str("hi", 3), "hi");

        // Empty string
        assert_eq!(truncate_str("", 10), "");
        assert_eq!(truncate_path("", 10), "");
    }
}
