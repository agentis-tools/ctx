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
use crate::lsp::{FileBackend, LspManager};
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
    /// LSP extraction backend; `None` unless `.ctx/config.toml` registers at
    /// least one `[lsp.*]` server. Lives on the indexer so servers stay warm
    /// across watch-mode events.
    lsp: Option<LspManager>,
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

        // Optional LSP extraction backend (inert without [lsp.*] config).
        let config = crate::lsp::LspConfig::load(&root);
        let lsp = LspManager::from_config(&root, &config, verbose);

        Ok(Self {
            db,
            parser: CodeParser::new(),
            root,
            verbose,
            walker_config,
            lsp,
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
            lsp: None,
        })
    }

    /// Decide how a file should be extracted: via the LSP manager when one is
    /// configured, otherwise via the builtin tree-sitter language table.
    pub fn backend_for(&self, path: &Path) -> FileBackend {
        match &self.lsp {
            Some(mgr) => mgr.backend_for(path),
            None => {
                if CodeParser::is_supported_static(path) {
                    FileBackend::TreeSitter
                } else {
                    FileBackend::Unsupported
                }
            }
        }
    }

    /// Stage A extraction for a file with an `lsp` backend, with graceful
    /// fallback: builtin languages re-parse with tree-sitter, dynamic
    /// languages store a file record with zero symbols (so incremental
    /// skipping and deletion cleanup keep working). Never fails the run.
    fn lsp_extract_or_fallback(
        &mut self,
        language: &str,
        rel_path: &str,
        abs_path: &Path,
        content: &str,
    ) -> ParseResult {
        if let Some(mgr) = self.lsp.as_mut() {
            if let Some(result) = mgr.extract(language, rel_path, content) {
                return result;
            }
        }

        // Server missing/crashed (the manager already warned once per
        // language): builtin grammars still produce full symbols.
        if CodeParser::is_supported_static(abs_path) {
            if let Some(result) = self.parser.parse(abs_path, content) {
                return result;
            }
        }

        ParseResult {
            file_path: rel_path.to_string(),
            language: language.to_string(),
            symbols: Vec::new(),
            edges: Vec::new(),
            module: None,
        }
    }

    /// Stage B: resolve leftover cross-file references for the given files
    /// via the language server, then refresh the LSP status sidecar.
    /// Best-effort — never affects the exit code.
    fn run_lsp_stage_b(&mut self, changed_files: &std::collections::HashSet<String>) {
        let verbose = self.verbose;
        if let Some(mgr) = self.lsp.as_mut() {
            if !changed_files.is_empty() {
                let resolved = crate::lsp::resolve::resolve_edges_with_lsp(
                    &self.db,
                    mgr,
                    changed_files,
                    verbose,
                );
                if verbose && resolved > 0 {
                    eprintln!("Resolved {} edge targets via LSP", resolved);
                }
            }
            mgr.write_status();
        }
    }

    /// Shut down all LSP servers at the end of a full indexing run. Watch
    /// mode never calls this so servers stay warm across events.
    fn shutdown_lsp(&mut self) {
        if let Some(mgr) = self.lsp.as_mut() {
            mgr.shutdown_all();
        }
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
        // Files indexed this run with an lsp/hybrid backend (Stage B input).
        let mut stage_b_files: std::collections::HashSet<String> = std::collections::HashSet::new();

        for entry in &entries {
            let rel_path = entry.relative_path.to_string_lossy().replace('\\', "/");

            // Only process supported files (builtin grammars plus any
            // language claimed by an [lsp.*] config block)
            let backend = self.backend_for(&entry.relative_path);
            if backend == FileBackend::Unsupported {
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

            // Parse the file (tree-sitter or LSP, per backend)
            let parse_result = match &backend {
                FileBackend::Lsp(language) => {
                    let language = language.clone();
                    self.lsp_extract_or_fallback(
                        &language,
                        &rel_path,
                        &entry.absolute_path,
                        &content,
                    )
                }
                _ => match self.parser.parse(&entry.absolute_path, &content) {
                    Some(r) => r,
                    None => {
                        if self.verbose {
                            eprintln!("Warning: failed to parse {}", rel_path);
                        }
                        result.files_failed += 1;
                        continue;
                    }
                },
            };

            // Store in database
            if let Err(e) = self.store_file(&rel_path, &content, &hash, &parse_result) {
                if self.verbose {
                    eprintln!("Warning: failed to store {}: {}", rel_path, e);
                }
                result.files_failed += 1;
                continue;
            }

            if matches!(backend, FileBackend::Lsp(_) | FileBackend::Hybrid(_)) {
                stage_b_files.insert(rel_path.clone());
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

        // Stage B: LSP-assisted resolution for what the SQL passes left
        // unresolved, then shut the servers down for this run.
        self.run_lsp_stage_b(&stage_b_files);
        self.shutdown_lsp();

        // The graph changed: invalidate the cached PageRank scores
        // (`ctx map` recomputes them lazily).
        if result.files_indexed > 0 {
            if let Err(e) = self.db.clear_symbol_rank() {
                if self.verbose {
                    eprintln!("Warning: failed to clear rank cache: {}", e);
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

                // Only process supported files (builtin grammars plus any
                // language claimed by an [lsp.*] config block)
                let backend = self.backend_for(&entry.relative_path);
                if backend == FileBackend::Unsupported {
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
                    Ok(true) => Some((entry.clone(), rel_path, content, hash, backend)),
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

        // Files indexed this run with an lsp/hybrid backend (Stage B input).
        let stage_b_files: std::collections::HashSet<String> = files_to_index
            .iter()
            .filter(|(_, _, _, _, backend)| {
                matches!(backend, FileBackend::Lsp(_) | FileBackend::Hybrid(_))
            })
            .map(|(_, rel_path, _, _, _)| rel_path.clone())
            .collect();

        // Partition: `lsp`-backend files are extracted serially (Stage A)
        // after the rayon parse phase; everything else (tree-sitter and
        // hybrid) goes through the unchanged parallel parse path.
        let (lsp_files, ts_files): (Vec<_>, Vec<_>) = files_to_index
            .into_iter()
            .partition(|(_, _, _, _, backend)| matches!(backend, FileBackend::Lsp(_)));

        // Parallel parse phase: parse all files that need updating
        let parsed_files: Vec<ParsedFile> = ts_files
            .par_iter()
            .filter_map(|(entry, rel_path, content, hash, _)| {
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

        // Track files we've seen for cleanup (both indexed and skipped);
        // includes LSP-claimed dynamic languages so their deletions are
        // cleaned up like any other file.
        let seen_files: Vec<String> = entries
            .iter()
            .filter_map(|e| {
                let rel = e.relative_path.to_string_lossy().replace('\\', "/");
                if self.backend_for(&e.relative_path) != FileBackend::Unsupported {
                    Some(rel)
                } else {
                    None
                }
            })
            .collect();

        // Store parsed files (single funnel shared with the serial path)
        for parsed in &parsed_files {
            if let Err(e) = self.store_file_impl(
                &parsed.rel_path,
                &parsed.content,
                &parsed.hash,
                &parsed.compressed,
                &parsed.parse_result,
            ) {
                if self.verbose {
                    eprintln!("Warning: failed to store {}: {}", parsed.rel_path, e);
                }
                result.files_failed += 1;
                continue;
            }

            result.files_indexed += 1;
            result.symbols_extracted += parsed.parse_result.symbols.len();
            result.edges_extracted += parsed.parse_result.edges.len();
        }

        // Stage A: serial LSP extraction, grouped per language so each
        // server handles its files consecutively while warm.
        if !lsp_files.is_empty() {
            let mut by_language: std::collections::BTreeMap<String, Vec<_>> =
                std::collections::BTreeMap::new();
            for (entry, rel_path, content, hash, backend) in lsp_files {
                if let FileBackend::Lsp(language) = backend {
                    by_language
                        .entry(language)
                        .or_default()
                        .push((entry, rel_path, content, hash));
                }
            }

            for (language, files) in by_language {
                for (entry, rel_path, content, hash) in files {
                    if verbose {
                        eprintln!("Indexing (lsp:{}): {}", language, rel_path);
                    }
                    let parse_result = self.lsp_extract_or_fallback(
                        &language,
                        &rel_path,
                        &entry.absolute_path,
                        &content,
                    );
                    if let Err(e) = self.store_file(&rel_path, &content, &hash, &parse_result) {
                        if self.verbose {
                            eprintln!("Warning: failed to store {}: {}", rel_path, e);
                        }
                        result.files_failed += 1;
                        continue;
                    }
                    result.files_indexed += 1;
                    result.symbols_extracted += parse_result.symbols.len();
                    result.edges_extracted += parse_result.edges.len();
                }
            }
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

        // Stage B: LSP-assisted resolution for what the SQL passes left
        // unresolved, then shut the servers down for this run.
        self.run_lsp_stage_b(&stage_b_files);
        self.shutdown_lsp();

        // The graph changed: invalidate the cached PageRank scores
        // (`ctx map` recomputes them lazily).
        if result.files_indexed > 0 {
            if let Err(e) = self.db.clear_symbol_rank() {
                if self.verbose {
                    eprintln!("Warning: failed to clear rank cache: {}", e);
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

        // Check if supported (builtin grammars plus LSP-claimed extensions)
        let backend = self.backend_for(&abs_path);
        if backend == FileBackend::Unsupported {
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

        // Parse (tree-sitter or LSP, per backend)
        let parse_result = match &backend {
            FileBackend::Lsp(language) => {
                let language = language.clone();
                self.lsp_extract_or_fallback(&language, &rel_path, &abs_path, &content)
            }
            _ => self
                .parser
                .parse(&abs_path, &content)
                .ok_or_else(|| io::Error::other("Parse failed"))?,
        };

        // Store
        self.store_file(&rel_path, &content, &hash, &parse_result)?;

        // The graph changed: invalidate the cached PageRank scores
        // (`ctx map` recomputes them lazily).
        self.db.clear_symbol_rank().map_err(db_error)?;

        // Stage B for this file (lsp/hybrid backends): run the cheap SQL
        // resolution first so the language server only sees the leftovers.
        // Servers stay warm on the indexer for subsequent watch events.
        if matches!(backend, FileBackend::Lsp(_) | FileBackend::Hybrid(_)) {
            if let Err(e) = self.db.resolve_edge_targets() {
                if self.verbose {
                    eprintln!("Warning: edge resolution failed: {}", e);
                }
            }
            let changed: std::collections::HashSet<String> = std::iter::once(rel_path).collect();
            self.run_lsp_stage_b(&changed);
        }

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
        self.store_file_impl(rel_path, content, hash, &compressed, parse_result)
    }

    /// Store a parsed file, reusing an already-compressed source blob.
    ///
    /// This is the single funnel for the serial, parallel, and watch
    /// indexing paths, so per-file bookkeeping (symbols, edges, modules,
    /// fingerprints) stays consistent between them.
    fn store_file_impl(
        &self,
        rel_path: &str,
        content: &str,
        hash: &str,
        compressed: &[u8],
        parse_result: &crate::db::ParseResult,
    ) -> io::Result<()> {
        let file_record = FileRecord {
            path: rel_path.to_string(),
            content_hash: hash.to_string(),
            size_bytes: content.len() as i64,
            language: Some(parse_result.language.clone()),
            last_indexed: 0,
        };

        // Store file FIRST (before symbols, due to foreign key constraint)
        self.db
            .upsert_file(&file_record, Some(compressed))
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

        // Compute and store MinHash fingerprints for this file's
        // function/method symbols (incremental: only changed files reach
        // store_file, and delete_symbols_for_file cascaded old rows away).
        let lang = crate::parser::Language::from_path(Path::new(rel_path));
        let fingerprints = crate::fingerprint::file_fingerprints(
            lang,
            content,
            rel_path,
            &parse_result.symbols,
            &id_map,
        );
        self.db
            .insert_fingerprints_batch(&fingerprints)
            .map_err(db_error)?;
        if self.verbose {
            eprintln!(
                "Fingerprinted {} functions in {}",
                fingerprints.len(),
                rel_path
            );
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

        let mut deleted_any = false;
        for file in indexed_files {
            if !seen_files.contains(&file) {
                if self.verbose {
                    eprintln!("Removing: {}", file);
                }
                self.db
                    .delete_file(&file)
                    .map_err(|e| io::Error::other(e.to_string()))?;
                deleted_any = true;
            }
        }

        // Deleting files changes the graph: invalidate the cached PageRank
        // scores (`ctx map` recomputes them lazily).
        if deleted_any {
            self.db.clear_symbol_rank().map_err(db_error)?;
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
    result.iter().map(|byte| format!("{byte:02x}")).collect()
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

                            // Check if it's a supported source file (LSP-aware:
                            // dynamic languages registered in [lsp.*] must
                            // reindex too, not just builtin grammars)
                            if indexer.backend_for(path) == crate::lsp::FileBackend::Unsupported {
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

                                // Delete from database and invalidate the
                                // cached PageRank scores
                                if let Err(e) = indexer.db.delete_file(&rel_path) {
                                    eprintln!("Warning: failed to remove {}: {}", rel_path, e);
                                } else if let Err(e) = indexer.db.clear_symbol_rank() {
                                    eprintln!("Warning: failed to clear rank cache: {}", e);
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

    /// A function with > 50 normalized tokens.
    const DUPE_A: &str = r#"
pub fn process_orders(items: &[i64]) -> i64 {
    let mut total = 0;
    for item in items {
        if *item > 10 {
            total += *item * 2;
        } else {
            total += *item + 1;
        }
    }
    println!("processed the batch: {}", total);
    total
}
"#;

    /// A structural copy of [`DUPE_A`] with every identifier renamed and
    /// every literal (numbers and the string) changed.
    const DUPE_B: &str = r#"
pub fn sum_invoices(entries: &[i64]) -> i64 {
    let mut acc = 0;
    for entry in entries {
        if *entry > 99 {
            acc += *entry * 7;
        } else {
            acc += *entry + 3;
        }
    }
    println!("done with invoices: {}", acc);
    acc
}
"#;

    /// A structurally unrelated function, also > 50 tokens.
    const UNRELATED: &str = r#"
pub fn render_table(headers: &[String], widths: &[usize]) -> String {
    let mut out = String::new();
    for (header, width) in headers.iter().zip(widths.iter()) {
        out.push('|');
        out.push_str(header);
        while out.len() < *width {
            out.push(' ');
        }
    }
    out.push('\n');
    for width in widths {
        out.push_str(&"-".repeat(*width));
        out.push('+');
    }
    out
}
"#;

    fn write_fixture(root: &std::path::Path) {
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.rs"), DUPE_A).unwrap();
        fs::write(src.join("b.rs"), DUPE_B).unwrap();
        fs::write(src.join("c.rs"), UNRELATED).unwrap();
    }

    #[test]
    fn test_near_duplicates_end_to_end() {
        let temp = TempDir::new().unwrap();
        write_fixture(temp.path());

        let mut indexer = Indexer::new_in_memory(temp.path()).unwrap();
        indexer.index().unwrap();

        // Renamed variables + different string literals are still detected
        // at the default threshold; the unrelated function is not.
        let pairs =
            crate::fingerprint::find_near_duplicates(indexer.database(), 0.85, 50, None).unwrap();
        assert_eq!(pairs.len(), 1, "expected exactly the renamed-copy pair");
        let pair = &pairs[0];
        let names = [pair.a.name.as_str(), pair.b.name.as_str()];
        assert!(names.contains(&"process_orders"), "names: {:?}", names);
        assert!(names.contains(&"sum_invoices"), "names: {:?}", names);
        assert!(pair.similarity >= 0.85);
        assert!(pair.token_count_a >= 50);
        assert!(pair.token_count_b >= 50);
        // Pairs are canonical: no self-pairs, endpoints ordered by id.
        assert!(pair.a.id < pair.b.id);

        // --against filter: pair survives when one endpoint changed...
        let changed: std::collections::HashSet<String> =
            std::iter::once("src/b.rs".to_string()).collect();
        let filtered =
            crate::fingerprint::find_near_duplicates(indexer.database(), 0.85, 50, Some(&changed))
                .unwrap();
        assert_eq!(filtered.len(), 1);

        // ...and is dropped when neither endpoint changed.
        let unrelated_change: std::collections::HashSet<String> =
            std::iter::once("src/c.rs".to_string()).collect();
        let filtered = crate::fingerprint::find_near_duplicates(
            indexer.database(),
            0.85,
            50,
            Some(&unrelated_change),
        )
        .unwrap();
        assert!(filtered.is_empty());

        // min-tokens above the fixture size filters everything out.
        let none = crate::fingerprint::find_near_duplicates(indexer.database(), 0.85, 10_000, None)
            .unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn test_parallel_indexing_also_fingerprints() {
        let temp = TempDir::new().unwrap();
        write_fixture(temp.path());

        let mut indexer = Indexer::new_in_memory(temp.path()).unwrap();
        indexer.index_parallel().unwrap();

        let fingerprints = indexer.database().get_fingerprints(0).unwrap();
        assert_eq!(fingerprints.len(), 3, "one fingerprint per function");
    }

    #[test]
    fn test_incremental_reindex_preserves_untouched_fingerprints() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_fixture(root);

        let mut indexer = Indexer::with_config(root, false, WalkerConfig::default()).unwrap();
        indexer.index().unwrap();

        let before = indexer.database().get_fingerprints(0).unwrap();
        let b_before: Vec<_> = before
            .iter()
            .filter(|f| f.file_path == "src/b.rs")
            .cloned()
            .collect();
        assert!(!b_before.is_empty());

        // Modify only a.rs; the incremental pass must re-fingerprint just
        // that file and leave b.rs's fingerprint BLOBs byte-identical.
        fs::write(
            root.join("src/a.rs"),
            DUPE_A.replace("process_orders", "process_orders_v2"),
        )
        .unwrap();
        let result = indexer.index().unwrap();
        assert_eq!(result.files_indexed, 1, "only the edited file re-indexes");

        let after = indexer.database().get_fingerprints(0).unwrap();
        let b_after: Vec<_> = after
            .iter()
            .filter(|f| f.file_path == "src/b.rs")
            .cloned()
            .collect();
        assert_eq!(
            b_before, b_after,
            "untouched fingerprints must be byte-identical"
        );
        assert!(after
            .iter()
            .any(|f| f.symbol_id.contains("process_orders_v2")));
        assert!(!after
            .iter()
            .any(|f| f.symbol_id.contains("process_orders@")));
    }

    #[test]
    fn test_solidity_files_produce_fingerprints() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("Token.sol"),
            "pragma solidity ^0.8.0;\ncontract Token {\n    function transfer(address to, uint256 amount) public returns (bool) {\n        return amount > 0;\n    }\n}\n",
        )
        .unwrap();

        let mut indexer = Indexer::new_in_memory(temp.path()).unwrap();
        let result = indexer.index().unwrap();
        assert_eq!(result.files_indexed, 1);

        // Solidity is tokenized with the solang-parser lexer, so its
        // functions are fingerprinted like any other language.
        assert!(!indexer.database().get_fingerprints(0).unwrap().is_empty());
    }

    /// AGE-5: a Solidity qualified call `ChessPureLib.isKingInCheck(...)` must
    /// resolve to the library's method even when the bare name `isKingInCheck`
    /// is ambiguous (also defined in another file). Before the fix the resolver
    /// bailed on the ambiguous bare name and left `target_id` NULL, silently
    /// zeroing the callee's fan-in/complexity/reachability metrics.
    #[test]
    fn test_solidity_qualified_call_resolves_across_ambiguous_bare_name() {
        let temp = TempDir::new().unwrap();

        // Library whose method we expect the qualified call to resolve to.
        fs::write(
            temp.path().join("ChessPureLib.sol"),
            r#"pragma solidity ^0.8.0;
library ChessPureLib {
    function isKingInCheck(uint256 tb, bool forBlack) internal pure returns (bool) {
        return tb > 0 && forBlack;
    }
}
"#,
        )
        .unwrap();

        // A second library defining a colliding bare name, so a plain
        // name lookup for `isKingInCheck` is ambiguous (COUNT > 1).
        fs::write(
            temp.path().join("OtherLib.sol"),
            r#"pragma solidity ^0.8.0;
library OtherLib {
    function isKingInCheck(uint256 tb, bool forBlack) internal pure returns (bool) {
        return tb < 0 || forBlack;
    }
}
"#,
        )
        .unwrap();

        // Caller making the qualified call `ChessPureLib.isKingInCheck(...)`.
        fs::write(
            temp.path().join("Game.sol"),
            r#"pragma solidity ^0.8.0;
contract Game {
    function check(uint256 tb, bool forBlack) public pure returns (bool) {
        return ChessPureLib.isKingInCheck(tb, forBlack);
    }
}
"#,
        )
        .unwrap();

        let mut indexer = Indexer::new_in_memory(temp.path()).unwrap();
        indexer.index().unwrap();

        let db = indexer.database();

        // The bare name is ambiguous: two library methods share it.
        let candidates: Vec<_> = db
            .find_symbols("isKingInCheck", 50)
            .unwrap()
            .into_iter()
            .filter(|s| s.kind == crate::db::SymbolKind::Function)
            .collect();
        assert!(
            candidates.len() >= 2,
            "expected the bare name to be ambiguous, got {} candidate(s)",
            candidates.len()
        );

        // The intended target: ChessPureLib.isKingInCheck.
        let chess_lib_method = candidates
            .iter()
            .find(|s| s.qualified_name.as_deref() == Some("ChessPureLib.isKingInCheck"))
            .expect("expected a ChessPureLib.isKingInCheck symbol");

        // The edge for the qualified call must resolve to the ChessPureLib method,
        // not be left NULL and not point at the OtherLib method.
        let call_edge = db
            .get_incoming_edges("isKingInCheck")
            .unwrap()
            .into_iter()
            .find(|e| e.kind == crate::db::EdgeKind::Calls)
            .expect("expected a calls edge for isKingInCheck");

        assert_eq!(
            call_edge.context.as_deref(),
            Some("ChessPureLib.isKingInCheck"),
            "qualifier should be captured on the edge context"
        );
        assert_eq!(
            call_edge.target_id.as_deref(),
            Some(chess_lib_method.id.as_str()),
            "qualified call must resolve to ChessPureLib.isKingInCheck, not NULL or the OtherLib symbol"
        );
    }
}
