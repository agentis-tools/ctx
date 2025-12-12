//! Code indexing module.
//!
//! This module provides functionality to:
//! - Walk the codebase and discover source files
//! - Parse files and extract symbols/edges
//! - Store extracted data in SQLite
//! - Support incremental updates

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::db::{Database, FileRecord};
use crate::parser::CodeParser;
use crate::walker::{discover_files, WalkerConfig};

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

/// Indexer for building the code intelligence database.
pub struct Indexer {
    db: Database,
    parser: CodeParser,
    root: PathBuf,
    verbose: bool,
}

impl Indexer {
    /// Create a new indexer for a project root.
    pub fn new(root: &Path, verbose: bool) -> io::Result<Self> {
        let root = root.canonicalize()?;

        // Create .ctx directory if needed
        let ctx_dir = root.join(CTX_DIR);
        if !ctx_dir.exists() {
            fs::create_dir_all(&ctx_dir)?;
        }

        // Open database
        let db_path = ctx_dir.join(DB_FILE);
        let db = Database::open(&db_path)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        Ok(Self {
            db,
            parser: CodeParser::new(),
            root,
            verbose,
        })
    }

    /// Create an indexer with an in-memory database (for testing).
    pub fn new_in_memory(root: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        let db = Database::open_in_memory()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        Ok(Self {
            db,
            parser: CodeParser::new(),
            root,
            verbose: false,
        })
    }

    /// Index the codebase.
    pub fn index(&mut self) -> io::Result<IndexResult> {
        let start = Instant::now();

        // Discover files
        let config = WalkerConfig::default();
        let entries = discover_files(&self.root, &config)?;

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
            let rel_path = entry
                .relative_path
                .to_string_lossy()
                .replace('\\', "/");

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
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

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
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        if !needs_update {
            return Ok(false);
        }

        // Parse
        let parse_result = self
            .parser
            .parse(&abs_path, &content)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Parse failed"))?;

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
        // Compress source
        let compressed = compress_source(content);

        // Create file record
        let file_record = FileRecord {
            path: rel_path.to_string(),
            content_hash: hash.to_string(),
            size_bytes: content.len() as i64,
            language: Some(parse_result.language.clone()),
            last_indexed: 0, // Will be set by database
        };

        // Store file FIRST (before symbols, due to foreign key constraint)
        self.db
            .upsert_file(&file_record, Some(&compressed))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        // Delete existing symbols for this file (after file exists)
        self.db
            .delete_symbols_for_file(rel_path)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        // Store symbols (rewrite file_path and id to use relative path)
        for symbol in &parse_result.symbols {
            let mut sym = symbol.clone();
            // Rewrite file_path to use relative path
            sym.file_path = rel_path.to_string();
            // Rebuild ID with relative path (include line for uniqueness)
            let parent_name = sym.parent_id.as_ref().and_then(|p| {
                // Extract parent name from the old ID format
                let parts: Vec<&str> = p.split("::").collect();
                if parts.len() >= 2 {
                    Some(parts[parts.len() - 1])
                } else {
                    None
                }
            });
            sym.id = crate::db::Symbol::make_id_with_line(
                rel_path,
                &sym.name,
                parent_name,
                sym.line_start,
            );
            // Also update parent_id if present
            if let Some(ref parent) = symbol.parent_id {
                let parts: Vec<&str> = parent.split("::").collect();
                if parts.len() >= 2 {
                    let parent_name = parts[parts.len() - 1];
                    // We don't have the parent's line number easily, so just use the name
                    sym.parent_id = Some(crate::db::Symbol::make_id(rel_path, parent_name, None));
                }
            }
            self.db
                .insert_symbol(&sym)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        }

        // Build a map from old symbol IDs to new symbol IDs
        let mut id_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for symbol in &parse_result.symbols {
            let parent_name = symbol.parent_id.as_ref().and_then(|p| {
                let parts: Vec<&str> = p.split("::").collect();
                if parts.len() >= 2 {
                    Some(parts[parts.len() - 1])
                } else {
                    None
                }
            });
            let new_id = crate::db::Symbol::make_id_with_line(
                rel_path,
                &symbol.name,
                parent_name,
                symbol.line_start,
            );
            id_map.insert(symbol.id.clone(), new_id);
        }

        // Store edges (rewrite source_id to use the new symbol IDs)
        for edge in &parse_result.edges {
            let mut e = edge.clone();
            // Look up the new source_id from the map
            if let Some(new_id) = id_map.get(&e.source_id) {
                e.source_id = new_id.clone();
            } else {
                // Fallback: just rewrite the file path part
                if let Some((_, rest)) = e.source_id.split_once("::") {
                    e.source_id = format!("{}::{}", rel_path, rest);
                }
            }
            // Rewrite target_id if present
            if let Some(ref target_id) = edge.target_id {
                if let Some(new_id) = id_map.get(target_id) {
                    e.target_id = Some(new_id.clone());
                } else if let Some((_, rest)) = target_id.split_once("::") {
                    e.target_id = Some(format!("{}::{}", rel_path, rest));
                }
            }
            self.db
                .insert_edge(&e)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        }

        // Store module info (rewrite file_path)
        if let Some(ref module) = parse_result.module {
            let mut m = module.clone();
            m.file_path = rel_path.to_string();
            self.db
                .upsert_module(&m)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        }

        Ok(())
    }

    /// Remove files from database that no longer exist.
    fn cleanup_deleted_files(&self, seen_files: &[String]) -> io::Result<()> {
        let indexed_files = self
            .db
            .get_indexed_files()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        for file in indexed_files {
            if !seen_files.contains(&file) {
                if self.verbose {
                    eprintln!("Removing: {}", file);
                }
                self.db
                    .delete_file(&file)
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
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
pub fn open_database(root: &Path) -> io::Result<Database> {
    let ctx_dir = root.join(CTX_DIR);
    let db_path = ctx_dir.join(DB_FILE);

    if !db_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Database not found. Run 'ctx index' first.\nExpected: {}",
                db_path.display()
            ),
        ));
    }

    Database::open(&db_path).map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
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
