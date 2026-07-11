//! Index command implementation.
//!
//! Handles codebase indexing with tree-sitter parsing.

use std::env;

use ctx::error::Result;
use ctx::index;
use ctx::walker;

/// Configuration for the index command.
#[derive(Debug, Clone)]
pub struct IndexConfig {
    /// Watch for file changes after initial indexing
    pub watch: bool,
    /// Verbose output
    pub verbose: bool,
    /// Force full reindex
    pub force: bool,
    /// Disable parallel indexing (single-threaded). Parallel is the default.
    pub serial: bool,
    /// Walker configuration
    pub walker: walker::WalkerConfig,
}

impl IndexConfig {
    /// Create a new IndexConfig from CLI arguments.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        watch: bool,
        verbose: bool,
        force: bool,
        serial: bool,
        no_gitignore: bool,
        no_default_ignores: bool,
        ignore_patterns: Vec<String>,
        include_patterns: Vec<String>,
    ) -> Self {
        Self {
            watch,
            verbose,
            force,
            serial,
            walker: walker::WalkerConfig {
                use_gitignore: !no_gitignore,
                use_default_ignores: !no_default_ignores,
                custom_ignores: ignore_patterns,
                include_patterns,
            },
        }
    }
}

/// Run the index command.
pub fn run_index(config: IndexConfig) -> Result<()> {
    let root = env::current_dir()?;

    // Handle force reindex by removing existing database (including the
    // SQLite WAL/SHM sidecar files, which would otherwise be stale)
    if config.force {
        let ctx_dir = root.join(index::CTX_DIR);
        let db_path = ctx_dir.join(index::DB_FILE);
        if db_path.exists() {
            eprintln!("Removing existing database for full reindex...");
            std::fs::remove_file(&db_path)?;
        }
        for suffix in ["-wal", "-shm"] {
            let sidecar = ctx_dir.join(format!("{}{}", index::DB_FILE, suffix));
            if sidecar.exists() {
                std::fs::remove_file(&sidecar)?;
            }
        }
    }

    if config.serial {
        eprintln!("Indexing codebase (serial mode)...");
    } else {
        eprintln!("Indexing codebase...");
    }

    let mut indexer = index::Indexer::with_config(&root, config.verbose, config.walker.clone())?;
    let result = if config.serial {
        indexer.index()?
    } else {
        indexer.index_parallel()?
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
    if config.watch {
        eprintln!("\nWatching for changes... (Ctrl+C to stop)");
        index::watch::watch_and_index(&root, config.verbose, config.walker)?;
    }

    Ok(())
}
