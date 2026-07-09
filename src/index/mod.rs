//! Code indexing module.
//!
//! This module provides functionality to:
//! - Walk the codebase and discover source files
//! - Parse files and extract symbols/edges (in parallel)
//! - Store extracted data in SQLite
//! - Support incremental updates

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use flate2::write::GzEncoder;
use flate2::Compression;
use rayon::prelude::*;
use sha2::{Digest, Sha256};

use crate::db::{Database, FileRecord, ParseResult};
use crate::parser::CodeParser;
use crate::walker::{discover_files, WalkerConfig};

// --- Helper functions for store_file ---

/// Convert database error to io::Error.
fn db_error<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

/// Extract parent name from an ID string (format: "path::parent::name").
fn extract_parent_name(parent_id: Option<&str>) -> Option<&str> {
    parent_id.and_then(|p| {
        let parts: Vec<&str> = p.split("::").collect();
        if parts.len() >= 2 {
            Some(parts[parts.len() - 1])
        } else {
            None
        }
    })
}

/// Rewrite an ID using the mapping, or fallback to path rewriting.
fn rewrite_id(
    id: &str,
    rel_path: &str,
    id_map: &std::collections::HashMap<String, String>,
) -> String {
    if let Some(new_id) = id_map.get(id) {
        new_id.clone()
    } else if let Some((_, rest)) = id.split_once("::") {
        format!("{}::{}", rel_path, rest)
    } else {
        id.to_string()
    }
}

/// Default directory name for storing the database.
pub const CTX_DIR: &str = ".ctx";

/// Default database filename.
pub const DB_FILE: &str = "codebase.sqlite";

/// Result of indexing operation.
#[derive(Debug)]
pub struct IndexResult {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_failed: usize,
    pub symbols_extracted: usize,
    pub edges_extracted: usize,
    pub elapsed_ms: u128,
}

/// Result of parsing a single file (used for parallel indexing).
struct ParsedFile {
    rel_path: String,
    content: String,
    hash: String,
    compressed: Vec<u8>,
    parse_result: ParseResult,
}

/// Indexer for building the code intelligence database.
pub struct Indexer {
    /// The database connection (pub for watch mode access).
    pub db: Database,
    parser: CodeParser,
    root: PathBuf,
    verbose: bool,
    /// Walker configuration for file discovery.
    walker_config: WalkerConfig,
}

impl Indexer {
    /// Create a new indexer with custom walker configuration.
    pub fn with_config(
        root: &Path,
        verbose: bool,
        walker_config: WalkerConfig,
    ) -> io::Result<Self> {
        let root = root.canonicalize()?;

        // Create .ctx directory if needed
        let ctx_dir = root.join(CTX_DIR);
        if !ctx_dir.exists() {
            fs::create_dir_all(&ctx_dir)?;
        }

        // Open database
        let db_path = ctx_dir.join(DB_FILE);
        let db = Database::open(&db_path).map_err(|e| io::Error::other(e.to_string()))?;

        Ok(Self {
            db,
            parser: CodeParser::new(),
            root,
            verbose,
            walker_config,
        })
    }

    /// Create an indexer with an in-memory database (for testing).
    #[allow(dead_code)]
    pub fn new_in_memory(root: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        let db = Database::open_in_memory().map_err(|e| io::Error::other(e.to_string()))?;

        Ok(Self {
            db,
            parser: CodeParser::new(),
            root,
            verbose: false,
            walker_config: WalkerConfig::default(),
        })
    }

    /// Index the codebase.
    pub fn index(&mut self) -> io::Result<IndexResult> {
        let start = Instant::now();

        // Discover files using the configured walker
        let entries = discover_files(&self.root, &self.walker_config)?;

        let mut result = IndexResult {
            files_indexed: 0,
            files_skipped: 0,
            files_failed: 0,
            symbols_extracted: 0,
            edges_extracted: 0,
            elapsed_ms: 0,
        };

        // Track files we've seen for cleanup
        let mut seen_files: Vec<String> = Vec::new();

        for entry in &entries {
            let rel_path = entry.relative_path.to_string_lossy().replace('\\', "/");

            // Only process supported languages
            if !self.parser.is_supported(&entry.relative_path) {
                result.files_skipped += 1;
                continue;
            }

            // Read file content
            let content = match fs::read_to_string(&entry.absolute_path) {
                Ok(c) => c,
                Err(e) => {
                    if self.verbose {
                        eprintln!("Warning: could not read {}: {}", rel_path, e);
                    }
                    result.files_failed += 1;
                    continue;
                }
            };

            // Calculate hash
            let hash = compute_hash(&content);

            // Check if file needs updating
            let needs_update = self
                .db
                .needs_update(&rel_path, &hash)
                .map_err(|e| io::Error::other(e.to_string()))?;

            if !needs_update {
                seen_files.push(rel_path.clone());
                result.files_skipped += 1;
                continue;
            }

            if self.verbose {
                eprintln!("Indexing: {}", rel_path);
            }

            // Parse the file
            let parse_result = match self.parser.parse(&entry.absolute_path, &content) {
                Some(r) => r,
                None => {
                    if self.verbose {
                        eprintln!("Warning: failed to parse {}", rel_path);
                    }
                    result.files_failed += 1;
                    continue;
                }
            };

            // Store in database
            if let Err(e) = self.store_file(&rel_path, &content, &hash, &parse_result) {
                if self.verbose {
                    eprintln!("Warning: failed to store {}: {}", rel_path, e);
                }
                result.files_failed += 1;
                continue;
            }

            seen_files.push(rel_path);
            result.files_indexed += 1;
            result.symbols_extracted += parse_result.symbols.len();
            result.edges_extracted += parse_result.edges.len();
        }

        // Clean up deleted files
        if let Err(e) = self.cleanup_deleted_files(&seen_files) {
            if self.verbose {
                eprintln!("Warning: cleanup failed: {}", e);
            }
        }

        // Resolve cross-file edge targets
        match self.db.resolve_edge_targets() {
            Ok(resolved) => {
                if self.verbose && resolved > 0 {
                    eprintln!("Resolved {} cross-file edge targets", resolved);
                }
            }
            Err(e) => {
                if self.verbose {
                    eprintln!("Warning: edge resolution failed: {}", e);
                }
            }
        }

        result.elapsed_ms = start.elapsed().as_millis();
        Ok(result)
    }

    /// Index the codebase using parallel parsing.
    ///
    /// This method uses rayon to parse files in parallel, then batch-inserts
    /// the results into the database. This is significantly faster for large
    /// codebases on multi-core systems.
    pub fn index_parallel(&mut self) -> io::Result<IndexResult> {
        let start = Instant::now();

        // Discover files using the configured walker
        let entries = discover_files(&self.root, &self.walker_config)?;

        // Counters for statistics (atomic for parallel access)
        let files_skipped = AtomicUsize::new(0);
        let files_failed = AtomicUsize::new(0);

        // First pass: determine which files need updating (sequential, requires DB)
        let files_to_index: Vec<_> = entries
            .iter()
            .filter_map(|entry| {
                let rel_path = entry.relative_path.to_string_lossy().replace('\\', "/");

                // Only process supported languages
                if !CodeParser::is_supported_static(&entry.relative_path) {
                    files_skipped.fetch_add(1, Ordering::Relaxed);
                    return None;
                }

                // Check if file needs updating (read content for hash)
                let content = match fs::read_to_string(&entry.absolute_path) {
                    Ok(c) => c,
                    Err(_) => {
                        files_failed.fetch_add(1, Ordering::Relaxed);
                        return None;
                    }
                };

                let hash = compute_hash(&content);

                // Check if file needs updating
                match self.db.needs_update(&rel_path, &hash) {
                    Ok(true) => Some((entry.clone(), rel_path, content, hash)),
                    Ok(false) => {
                        files_skipped.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                    Err(_) => {
                        files_failed.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                }
            })
            .collect();

        let verbose = self.verbose;

        // Parallel parse phase: parse all files that need updating
        let parsed_files: Vec<ParsedFile> = files_to_index
            .par_iter()
            .filter_map(|(entry, rel_path, content, hash)| {
                // Create thread-local parser
                let mut parser = CodeParser::new();

                if verbose {
                    eprintln!("Indexing: {}", rel_path);
                }

                // Parse the file
                let parse_result = parser.parse(&entry.absolute_path, content)?;

                // Compress content
                let compressed = compress_source(content);

                Some(ParsedFile {
                    rel_path: rel_path.clone(),
                    content: content.clone(),
                    hash: hash.clone(),
                    compressed,
                    parse_result,
                })
            })
            .collect();

        // Sequential store phase: batch insert into database
        let mut result = IndexResult {
            files_indexed: 0,
            files_skipped: files_skipped.load(Ordering::Relaxed),
            files_failed: files_failed.load(Ordering::Relaxed),
            symbols_extracted: 0,
            edges_extracted: 0,
            elapsed_ms: 0,
        };

        // Track files we've seen for cleanup (both indexed and skipped)
        let seen_files: Vec<String> = entries
            .iter()
            .filter_map(|e| {
                let rel = e.relative_path.to_string_lossy().replace('\\', "/");
                if CodeParser::is_supported_static(&e.relative_path) {
                    Some(rel)
                } else {
                    None
                }
            })
            .collect();

        // Store parsed files
        for parsed in &parsed_files {
            let file_record = FileRecord {
                path: parsed.rel_path.clone(),
                content_hash: parsed.hash.clone(),
                size_bytes: parsed.content.len() as i64,
                language: Some(parsed.parse_result.language.clone()),
                last_indexed: 0,
            };

            // Store file and delete existing symbols
            if let Err(e) = self.db.upsert_file(&file_record, Some(&parsed.compressed)) {
                if self.verbose {
                    eprintln!("Warning: failed to store file {}: {}", parsed.rel_path, e);
                }
                result.files_failed += 1;
                continue;
            }

            if let Err(e) = self.db.delete_symbols_for_file(&parsed.rel_path) {
                if self.verbose {
                    eprintln!(
                        "Warning: failed to clear symbols for {}: {}",
                        parsed.rel_path, e
                    );
                }
                result.files_failed += 1;
                continue;
            }

            // Store symbols and build ID mapping
            let id_map = match self.store_symbols(&parsed.rel_path, &parsed.parse_result.symbols) {
                Ok(map) => map,
                Err(e) => {
                    if self.verbose {
                        eprintln!(
                            "Warning: failed to store symbols for {}: {}",
                            parsed.rel_path, e
                        );
                    }
                    result.files_failed += 1;
                    continue;
                }
            };

            // Store edges
            if let Err(e) = self.store_edges(&parsed.rel_path, &parsed.parse_result.edges, &id_map)
            {
                if self.verbose {
                    eprintln!(
                        "Warning: failed to store edges for {}: {}",
                        parsed.rel_path, e
                    );
                }
                result.files_failed += 1;
                continue;
            }

            // Store module info
            if let Some(ref module) = parsed.parse_result.module {
                let mut m = module.clone();
                m.file_path = parsed.rel_path.clone();
                if let Err(e) = self.db.upsert_module(&m) {
                    if self.verbose {
                        eprintln!(
                            "Warning: failed to store module for {}: {}",
                            parsed.rel_path, e
                        );
                    }
                }
            }

            result.files_indexed += 1;
            result.symbols_extracted += parsed.parse_result.symbols.len();
            result.edges_extracted += parsed.parse_result.edges.len();
        }

        // Clean up deleted files
        if let Err(e) = self.cleanup_deleted_files(&seen_files) {
            if self.verbose {
                eprintln!("Warning: cleanup failed: {}", e);
            }
        }

        // Resolve cross-file edge targets
        match self.db.resolve_edge_targets() {
            Ok(resolved) => {
                if self.verbose && resolved > 0 {
                    eprintln!("Resolved {} cross-file edge targets", resolved);
                }
            }
            Err(e) => {
                if self.verbose {
                    eprintln!("Warning: edge resolution failed: {}", e);
                }
            }
        }

        result.elapsed_ms = start.elapsed().as_millis();
        Ok(result)
    }

    /// Index a single file.
    pub fn index_file(&mut self, path: &Path) -> io::Result<bool> {
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };

        let rel_path = abs_path
            .strip_prefix(&self.root)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Path not in root"))?
            .to_string_lossy()
            .replace('\\', "/");

        // Check if supported
        if !self.parser.is_supported(path) {
            return Ok(false);
        }

        // Read content
        let content = fs::read_to_string(&abs_path)?;
        let hash = compute_hash(&content);

        // Check if needs update
        let needs_update = self
            .db
            .needs_update(&rel_path, &hash)
            .map_err(|e| io::Error::other(e.to_string()))?;

        if !needs_update {
            return Ok(false);
        }

        // Parse
        let parse_result = self
            .parser
            .parse(&abs_path, &content)
            .ok_or_else(|| io::Error::other("Parse failed"))?;

        // Store
        self.store_file(&rel_path, &content, &hash, &parse_result)?;

        Ok(true)
    }

    /// Store a parsed file in the database.
    fn store_file(
        &self,
        rel_path: &str,
        content: &str,
        hash: &str,
        parse_result: &crate::db::ParseResult,
    ) -> io::Result<()> {
        let compressed = compress_source(content);
        let file_record = FileRecord {
            path: rel_path.to_string(),
            content_hash: hash.to_string(),
            size_bytes: content.len() as i64,
            language: Some(parse_result.language.clone()),
            last_indexed: 0,
        };

        // Store file FIRST (before symbols, due to foreign key constraint)
        self.db
            .upsert_file(&file_record, Some(&compressed))
            .map_err(db_error)?;
        self.db
            .delete_symbols_for_file(rel_path)
            .map_err(db_error)?;

        // Build ID mapping and store symbols
        let id_map = self.store_symbols(rel_path, &parse_result.symbols)?;

        // Store edges with rewritten IDs
        self.store_edges(rel_path, &parse_result.edges, &id_map)?;

        // Store module info
        if let Some(ref module) = parse_result.module {
            let mut m = module.clone();
            m.file_path = rel_path.to_string();
            self.db.upsert_module(&m).map_err(db_error)?;
        }

        Ok(())
    }

    /// Store symbols and build ID mapping from old to new IDs.
    fn store_symbols(
        &self,
        rel_path: &str,
        symbols: &[crate::db::Symbol],
    ) -> io::Result<std::collections::HashMap<String, String>> {
        let mut id_map = std::collections::HashMap::new();

        for symbol in symbols {
            let parent_name = extract_parent_name(symbol.parent_id.as_deref());
            let new_id = crate::db::Symbol::make_id_with_line(
                rel_path,
                &symbol.name,
                parent_name,
                symbol.line_start,
            );
            id_map.insert(symbol.id.clone(), new_id.clone());

            let mut sym = symbol.clone();
            sym.file_path = rel_path.to_string();
            sym.id = new_id;
            if symbol.parent_id.is_some() {
                if let Some(pn) = parent_name {
                    sym.parent_id = Some(crate::db::Symbol::make_id(rel_path, pn, None));
                }
            }
            self.db.insert_symbol(&sym).map_err(db_error)?;
        }

        Ok(id_map)
    }

    /// Store edges with rewritten source/target IDs.
    fn store_edges(
        &self,
        rel_path: &str,
        edges: &[crate::db::Edge],
        id_map: &std::collections::HashMap<String, String>,
    ) -> io::Result<()> {
        for edge in edges {
            let mut e = edge.clone();
            e.source_id = rewrite_id(&e.source_id, rel_path, id_map);
            if let Some(ref target_id) = edge.target_id {
                e.target_id = Some(rewrite_id(target_id, rel_path, id_map));
            }
            self.db.insert_edge(&e).map_err(db_error)?;
        }
        Ok(())
    }

    /// Remove files from database that no longer exist.
    fn cleanup_deleted_files(&self, seen_files: &[String]) -> io::Result<()> {
        let indexed_files = self
            .db
            .get_indexed_files()
            .map_err(|e| io::Error::other(e.to_string()))?;

        for file in indexed_files {
            if !seen_files.contains(&file) {
                if self.verbose {
                    eprintln!("Removing: {}", file);
                }
                self.db
                    .delete_file(&file)
                    .map_err(|e| io::Error::other(e.to_string()))?;
            }
        }

        Ok(())
    }

    /// Get a reference to the database.
    pub fn database(&self) -> &Database {
        &self.db
    }
}

/// Compute SHA256 hash of content.
fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)
}

/// Compress source code using gzip.
fn compress_source(content: &str) -> Vec<u8> {
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(content.as_bytes()).ok();
    encoder.finish().unwrap_or_default()
}

/// Open the database for a project.
pub fn open_database(root: &Path) -> crate::error::Result<Database> {
    let ctx_dir = root.join(CTX_DIR);
    let db_path = ctx_dir.join(DB_FILE);

    if !db_path.exists() {
        return Err(crate::error::CtxError::IndexNotFound(format!(
            "run 'ctx index' first (expected {})",
            db_path.display()
        )));
    }

    Database::open(&db_path)
}

/// Watch mode for automatic reindexing.
pub mod watch {
    use std::path::Path;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use notify::RecursiveMode;
    use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};

    use super::Indexer;
    use crate::parser::Language;
    use crate::walker::{FileFilter, WalkerConfig};

    /// Start watching the codebase for changes and reindex automatically.
    pub fn watch_and_index(
        root: &Path,
        verbose: bool,
        walker_config: WalkerConfig,
    ) -> std::io::Result<()> {
        let root = root.canonicalize()?;

        // Build file filter once for efficient watch-mode filtering
        // This handles .gitignore, .contextignore, default ignores, custom ignores, and include patterns
        let file_filter = FileFilter::new(&root, &walker_config)?;

        // Do initial index
        eprintln!("Performing initial index...");
        let mut indexer = Indexer::with_config(&root, verbose, walker_config)?;
        let result = indexer.index()?;
        eprintln!(
            "Initial index complete: {} files, {} symbols",
            result.files_indexed + result.files_skipped,
            result.symbols_extracted
        );

        // Set up file watcher with debouncing
        let (tx, rx) = channel();

        let mut debouncer = new_debouncer(Duration::from_millis(500), tx)
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        debouncer
            .watcher()
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        eprintln!("\nWatching for changes... (press Ctrl+C to stop)");

        // Process file change events
        loop {
            match rx.recv() {
                Ok(Ok(events)) => {
                    let mut reindex_needed = false;

                    for event in events {
                        // Handle both Any and AnyContinuous events
                        // AnyContinuous signals ongoing/rapid changes that should also trigger reindex
                        if matches!(
                            event.kind,
                            DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous
                        ) {
                            let path = &event.path;

                            // Skip .ctx directory
                            if path.starts_with(root.join(super::CTX_DIR)) {
                                continue;
                            }

                            // Check if it's a supported source file
                            let lang = Language::from_path(path);
                            if lang == Language::Unknown {
                                continue;
                            }

                            // Check if file should be included based on walker config
                            // (respects .gitignore, .contextignore, --ignore, --pattern, etc.)
                            if !file_filter.should_include(path) {
                                continue;
                            }

                            // Check if file exists (handle deletions)
                            if !path.exists() {
                                let rel_path = path
                                    .strip_prefix(&root)
                                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                                    .unwrap_or_default();

                                if verbose {
                                    eprintln!("Removed: {}", rel_path);
                                }

                                // Delete from database
                                if let Err(e) = indexer.db.delete_file(&rel_path) {
                                    eprintln!("Warning: failed to remove {}: {}", rel_path, e);
                                }
                                continue;
                            }

                            // Index the changed file
                            match indexer.index_file(path) {
                                Ok(true) => {
                                    let rel_path = path
                                        .strip_prefix(&root)
                                        .map(|p| p.to_string_lossy().to_string())
                                        .unwrap_or_else(|_| path.display().to_string());

                                    // Resolve edge targets after reindexing to maintain accurate analytics
                                    if let Err(e) = indexer.db.resolve_edge_targets() {
                                        if verbose {
                                            eprintln!("Warning: edge resolution failed: {}", e);
                                        }
                                    }

                                    if verbose {
                                        eprintln!("Reindexed: {}", rel_path);
                                    } else {
                                        eprint!(".");
                                    }
                                    reindex_needed = true;
                                }
                                Ok(false) => {
                                    // File unchanged
                                }
                                Err(e) => {
                                    let rel_path = path
                                        .strip_prefix(&root)
                                        .map(|p| p.to_string_lossy().to_string())
                                        .unwrap_or_else(|_| path.display().to_string());
                                    eprintln!("\nWarning: failed to index {}: {}", rel_path, e);
                                }
                            }
                        }
                    }

                    if reindex_needed && !verbose {
                        eprintln!(); // Newline after dots
                    }
                }
                Ok(Err(error)) => {
                    eprintln!("Watch error: {:?}", error);
                }
                Err(e) => {
                    eprintln!("Channel error: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_compute_hash() {
        let hash1 = compute_hash("hello");
        let hash2 = compute_hash("hello");
        let hash3 = compute_hash("world");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_compress_source() {
        let content = "fn main() { println!(\"Hello, world!\"); }";
        let compressed = compress_source(content);
        assert!(!compressed.is_empty());
        assert!(compressed.len() < content.len() * 2); // Reasonable compression
    }

    #[test]
    fn test_index_simple_project() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Create a simple Rust file
        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            src_dir.join("main.rs"),
            r#"
/// Main entry point
fn main() {
    println!("Hello, world!");
}

/// A helper function
fn helper() -> i32 {
    42
}
"#,
        )
        .unwrap();

        // Create indexer
        let mut indexer = Indexer::new_in_memory(root).unwrap();
        let result = indexer.index().unwrap();

        assert_eq!(result.files_indexed, 1);
        assert!(result.symbols_extracted >= 2); // main and helper

        // Check database
        let stats = indexer.database().get_stats().unwrap();
        assert_eq!(stats.files, 1);
        assert!(stats.symbols >= 2);
    }
}
