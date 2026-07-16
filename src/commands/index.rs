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

/// Merge the global positional patterns with `-p/--pattern` values into a
/// single include-pattern list for the walker.
///
/// The positional argument defaults to `.` (the whole repository), so a bare
/// `.` (or `./`) adds no scoping and is dropped; anything else the user typed
/// (literal paths, directories, globs) scopes the index exactly like `-p`.
pub fn merge_include_patterns(positional: Vec<String>, flags: Vec<String>) -> Vec<String> {
    let mut merged: Vec<String> = positional
        .into_iter()
        .filter(|p| p.trim_end_matches('/') != "." && !p.is_empty())
        .collect();
    merged.extend(flags);
    merged
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

    // Echo the effective scope so a mistyped pattern is visible before the
    // (potentially expensive) parse phase rather than after it.
    let scope = if config.walker.include_patterns.is_empty() {
        String::new()
    } else {
        format!(
            " (scoped to: {})",
            config.walker.include_patterns.join(", ")
        )
    };
    if config.serial {
        eprintln!("Indexing codebase (serial mode){}...", scope);
    } else {
        eprintln!("Indexing codebase{}...", scope);
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

#[cfg(test)]
mod tests {
    use super::merge_include_patterns;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_dot_adds_no_scoping() {
        assert!(merge_include_patterns(v(&["."]), v(&[])).is_empty());
        assert!(merge_include_patterns(v(&["./"]), v(&[])).is_empty());
    }

    #[test]
    fn positional_paths_scope_the_index() {
        assert_eq!(merge_include_patterns(v(&["src"]), v(&[])), v(&["src"]));
        assert_eq!(merge_include_patterns(v(&["src/"]), v(&[])), v(&["src/"]));
        assert_eq!(
            merge_include_patterns(v(&["src/**/*.rs"]), v(&[])),
            v(&["src/**/*.rs"])
        );
    }

    #[test]
    fn positional_and_flag_patterns_are_combined() {
        assert_eq!(
            merge_include_patterns(v(&["lib"]), v(&["src/**"])),
            v(&["lib", "src/**"])
        );
        // A stray `.` among real patterns still means "everything is already
        // included by default" and contributes nothing.
        assert_eq!(
            merge_include_patterns(v(&[".", "lib"]), v(&[])),
            v(&["lib"])
        );
    }

    #[test]
    fn flags_alone_pass_through() {
        assert_eq!(
            merge_include_patterns(v(&["."]), v(&["src/**"])),
            v(&["src/**"])
        );
    }
}
