//! Embedding command implementations.
//!
//! Handles embedding generation, watching, and semantic search.

use std::env;
use std::time::Instant;

use ctx::embeddings::{self, Provider};
use ctx::error::Result;
use ctx::index;
use ctx::utils::{truncate_path, truncate_str};

/// Emit the one-time hint before the local model is (down)loaded.
fn local_model_hint(provider: Provider) {
    if provider == Provider::Local {
        eprintln!("Initializing local embedding model (first run downloads ~90MB)...");
    }
}

/// Generate embeddings for all symbols.
pub fn run_embed(
    force: bool,
    verbose: bool,
    batch_size: usize,
    provider: Provider,
    serial: bool,
) -> Result<()> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    local_model_hint(provider);
    let provider =
        embeddings::build_provider(provider, &ctx::config::CtxConfig::load(&root).embedding)?;

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

    let embedded = embeddings::embed_missing_symbols(
        &db,
        provider.as_ref(),
        batch_size,
        serial,
        Some(&progress_callback),
    )?;

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
pub fn run_embed_watch(
    verbose: bool,
    batch_size: usize,
    provider: Provider,
    serial: bool,
) -> Result<()> {
    use notify::RecursiveMode;
    use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    let root = env::current_dir()?;
    let ctx_dir = root.join(".ctx");
    let _db_path = ctx_dir.join("codebase.sqlite");

    local_model_hint(provider);
    let provider =
        embeddings::build_provider(provider, &ctx::config::CtxConfig::load(&root).embedding)?;

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
            let embedded = embeddings::embed_missing_symbols(
                &db,
                provider.as_ref(),
                batch_size,
                serial,
                None,
            )?;
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

                                match embeddings::embed_missing_symbols(
                                    &db,
                                    provider.as_ref(),
                                    batch_size,
                                    serial,
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
pub fn run_semantic(query: &str, limit: usize, output: &str, provider: Provider) -> Result<()> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Check if we have embeddings
    let embedding_count = db.count_embeddings()?;
    if embedding_count == 0 {
        eprintln!("No embeddings found. Run 'ctx embed' first to generate embeddings.");
        if output == "json" {
            return ctx::json::emit("semantic", semantic_data(query, limit, &[], &db));
        }
        return Ok(());
    }

    local_model_hint(provider);
    let provider =
        embeddings::build_provider(provider, &ctx::config::CtxConfig::load(&root).embedding)?;

    // Warn if the query provider/dimension differs from the index.
    embeddings::warn_index_mismatch(&db, provider.as_ref());

    // Embed the query
    let query_embedding = provider.embed(query)?;

    // Search for similar symbols
    let results = embeddings::semantic_search(&db, &query_embedding, limit)?;

    if output == "json" {
        return ctx::json::emit("semantic", semantic_data(query, limit, &results, &db));
    }

    if results.is_empty() {
        eprintln!("No results found for '{}'", query);
        return Ok(());
    }

    println!(
        "Semantic search for '{}' ({} results):",
        query,
        results.len()
    );
    println!("{}", "-".repeat(80));
    println!("{:<35} {:<10} {:<8} FILE", "SYMBOL", "KIND", "SCORE");
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

    Ok(())
}

/// Build the `semantic` JSON payload.
fn semantic_data(
    query: &str,
    limit: usize,
    results: &[embeddings::SearchResult],
    db: &ctx::db::Database,
) -> serde_json::Value {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            // Resolve the full symbol for a complete SymbolRef; fall back to
            // the fields carried by the search result.
            let symbol = match db.get_symbol(&r.symbol_id) {
                Ok(Some(s)) => ctx::json::SymbolRef::from(&s).to_value(),
                _ => serde_json::json!({
                    "name": r.name,
                    "qualified_name": serde_json::Value::Null,
                    "kind": r.kind,
                    "file": r.file_path,
                    "line_start": r.line,
                    "line_end": r.line,
                }),
            };
            serde_json::json!({
                "symbol": symbol,
                "symbol_id": r.symbol_id,
                "score": r.score,
            })
        })
        .collect();

    serde_json::json!({
        "query": query,
        "limit": limit,
        "results": items,
    })
}
