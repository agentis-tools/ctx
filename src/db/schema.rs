//! SQLite schema and database operations.

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use rusqlite::ffi::sqlite3_auto_extension;
use rusqlite::{params, Connection, Result, Transaction};
use sqlite_vec::sqlite3_vec_init;

use super::models::*;

/// Current index schema version, stamped into SQLite's `PRAGMA user_version`.
///
/// Bump this whenever the schema changes in a way that is incompatible with
/// existing databases. Databases with `user_version = 0` are pre-versioning
/// databases whose schema is identical to v1; they are silently stamped.
/// Any other mismatch is reported as [`crate::error::CtxError::SchemaVersionMismatch`].
pub const SCHEMA_VERSION: i64 = 1;

/// Default embedding dimension for vector search.
/// This matches OpenAI text-embedding-ada-002 (1536) and text-embedding-3-small (1536).
/// For local embeddings with fastembed (384 dims), a separate table or dynamic dimension is needed.
///
/// TODO: Make this configurable per-provider or support multiple dimension tables.
pub const DEFAULT_EMBEDDING_DIM: usize = 1536;

/// Track whether sqlite-vec extension loaded successfully.
static VEC_EXTENSION_AVAILABLE: AtomicBool = AtomicBool::new(false);

/// Initialize the sqlite-vec extension for vector search.
///
/// This must be called before any Database connections are opened.
/// It is safe to call multiple times - initialization only happens once.
/// Returns true if the extension was loaded successfully.
#[allow(clippy::missing_transmute_annotations)]
pub fn init_vec_extension() -> bool {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let result = unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite3_vec_init as *const (),
            )))
        };
        // sqlite3_auto_extension returns SQLITE_OK (0) on success
        if result == 0 {
            VEC_EXTENSION_AVAILABLE.store(true, Ordering::SeqCst);
        } else {
            eprintln!(
                "Warning: Failed to register sqlite-vec extension (error code: {}). Vector search will be unavailable.",
                result
            );
        }
    });
    VEC_EXTENSION_AVAILABLE.load(Ordering::SeqCst)
}

/// Check if sqlite-vec extension is available for vector search.
pub fn is_vec_extension_available() -> bool {
    VEC_EXTENSION_AVAILABLE.load(Ordering::SeqCst)
}

/// SQLite database for code intelligence.
#[derive(Debug)]
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create a database at the given path.
    ///
    /// Verifies the schema version stored in `PRAGMA user_version`:
    /// - `0` (fresh or pre-versioning database): the schema is initialized and
    ///   the version is stamped to [`SCHEMA_VERSION`].
    /// - [`SCHEMA_VERSION`]: opened normally.
    /// - anything else: returns [`crate::error::CtxError::SchemaVersionMismatch`].
    pub fn open(path: &Path) -> crate::error::Result<Self> {
        // Initialize sqlite-vec extension before opening connection
        init_vec_extension();
        let conn = Connection::open(path)?;
        Self::configure_connection(&conn)?;
        let db = Self { conn };
        db.check_schema_version()?;
        db.init_schema()?;
        Ok(db)
    }

    /// Create an in-memory database (for testing).
    #[allow(dead_code)]
    pub fn open_in_memory() -> crate::error::Result<Self> {
        // Initialize sqlite-vec extension before opening connection
        init_vec_extension();
        let conn = Connection::open_in_memory()?;
        Self::configure_connection(&conn)?;
        let db = Self { conn };
        db.check_schema_version()?;
        db.init_schema()?;
        Ok(db)
    }

    /// Validate `PRAGMA user_version` against [`SCHEMA_VERSION`].
    ///
    /// Version `0` means a fresh database or a pre-versioning (v0) database
    /// whose schema is identical to v1; it is silently stamped to the current
    /// version. Any other mismatch is an error.
    fn check_schema_version(&self) -> crate::error::Result<()> {
        let found: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;

        if found == 0 {
            self.conn
                .pragma_update(None, "user_version", SCHEMA_VERSION)?;
            Ok(())
        } else if found == SCHEMA_VERSION {
            Ok(())
        } else {
            Err(crate::error::CtxError::SchemaVersionMismatch {
                found,
                expected: SCHEMA_VERSION,
            })
        }
    }

    /// Configure the SQLite connection for optimal performance and concurrency.
    fn configure_connection(conn: &Connection) -> Result<()> {
        // Enable WAL mode for better concurrent access
        // This allows multiple readers and one writer simultaneously
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA busy_timeout = 5000;
            PRAGMA cache_size = -64000;
            "#,
        )?;
        Ok(())
    }

    /// Initialize the database schema.
    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            -- Enable foreign keys
            PRAGMA foreign_keys = ON;

            -- File tracking for incremental updates
            CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                content_hash TEXT NOT NULL,
                size_bytes INTEGER,
                language TEXT,
                last_indexed INTEGER DEFAULT (unixepoch()),
                source BLOB
            );

            -- All symbols (functions, structs, etc.)
            CREATE TABLE IF NOT EXISTS symbols (
                id TEXT PRIMARY KEY,
                file_path TEXT NOT NULL,
                name TEXT NOT NULL,
                qualified_name TEXT,
                kind TEXT NOT NULL,
                visibility TEXT DEFAULT 'private',
                signature TEXT,
                brief TEXT,
                docstring TEXT,
                line_start INTEGER,
                line_end INTEGER,
                col_start INTEGER,
                col_end INTEGER,
                parent_id TEXT,
                source TEXT,
                FOREIGN KEY (file_path) REFERENCES files(path) ON DELETE CASCADE
            );

            -- Relationships between symbols
            CREATE TABLE IF NOT EXISTS edges (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_id TEXT NOT NULL,
                target_id TEXT,
                target_name TEXT NOT NULL,
                kind TEXT NOT NULL,
                line INTEGER,
                col INTEGER,
                context TEXT,
                FOREIGN KEY (source_id) REFERENCES symbols(id) ON DELETE CASCADE
            );

            -- Module-level information
            CREATE TABLE IF NOT EXISTS modules (
                file_path TEXT PRIMARY KEY,
                module_name TEXT,
                exports TEXT,
                imports TEXT,
                FOREIGN KEY (file_path) REFERENCES files(path) ON DELETE CASCADE
            );

            -- Indexes for fast lookups
            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_path);
            CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);
            CREATE INDEX IF NOT EXISTS idx_symbols_parent ON symbols(parent_id);
            CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source_id);
            CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target_name);
            CREATE INDEX IF NOT EXISTS idx_edges_kind ON edges(kind);
            CREATE INDEX IF NOT EXISTS idx_files_hash ON files(content_hash);

            -- Embeddings for semantic search
            CREATE TABLE IF NOT EXISTS embeddings (
                symbol_id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                dimension INTEGER NOT NULL,
                vector TEXT NOT NULL,  -- JSON array of floats
                created_at INTEGER DEFAULT (unixepoch()),
                FOREIGN KEY (symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_embeddings_provider ON embeddings(provider);

            -- Full-text search index for semantic search
            CREATE VIRTUAL TABLE IF NOT EXISTS symbol_fts USING fts5(
                id,
                name,
                kind,
                signature,
                brief,
                docstring,
                content='symbols',
                content_rowid='rowid'
            );

            -- Triggers to keep FTS index in sync
            CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
                INSERT INTO symbol_fts(rowid, id, name, kind, signature, brief, docstring)
                VALUES (NEW.rowid, NEW.id, NEW.name, NEW.kind, NEW.signature, NEW.brief, NEW.docstring);
            END;

            CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
                INSERT INTO symbol_fts(symbol_fts, rowid, id, name, kind, signature, brief, docstring)
                VALUES ('delete', OLD.rowid, OLD.id, OLD.name, OLD.kind, OLD.signature, OLD.brief, OLD.docstring);
            END;

            CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
                INSERT INTO symbol_fts(symbol_fts, rowid, id, name, kind, signature, brief, docstring)
                VALUES ('delete', OLD.rowid, OLD.id, OLD.name, OLD.kind, OLD.signature, OLD.brief, OLD.docstring);
                INSERT INTO symbol_fts(rowid, id, name, kind, signature, brief, docstring)
                VALUES (NEW.rowid, NEW.id, NEW.name, NEW.kind, NEW.signature, NEW.brief, NEW.docstring);
            END;
            "#,
        )?;

        // Try to create the symbol_vectors virtual table for fast KNN search
        // Uses sqlite-vec vec0 extension for indexed vector similarity search
        // This is optional - if it fails, we fall back to the JSON embeddings table
        if is_vec_extension_available() {
            match self.conn.execute(
                &format!(
                    r#"
                    CREATE VIRTUAL TABLE IF NOT EXISTS symbol_vectors USING vec0(
                        embedding float[{}],
                        +symbol_id TEXT
                    )
                    "#,
                    DEFAULT_EMBEDDING_DIM
                ),
                [],
            ) {
                Ok(_) => {}
                Err(e) => {
                    // Log warning but don't fail - vector search is optional
                    eprintln!(
                        "Warning: Failed to create symbol_vectors table: {}. Vector search will be unavailable.",
                        e
                    );
                }
            }
        }

        Ok(())
    }

    /// Check if vector search is available (sqlite-vec extension loaded and table exists).
    pub fn has_vector_search(&self) -> bool {
        if !is_vec_extension_available() {
            return false;
        }
        // Check if the table exists and is queryable
        self.conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='symbol_vectors'",
                [],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// Begin a transaction.
    #[allow(dead_code)]
    pub fn transaction(&mut self) -> Result<Transaction<'_>> {
        self.conn.transaction()
    }

    /// Get the content hash for a file.
    pub fn get_file_hash(&self, path: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT content_hash FROM files WHERE path = ?",
                [path],
                |row| row.get(0),
            )
            .optional()
    }

    /// Check if a file needs reindexing based on hash.
    pub fn needs_update(&self, path: &str, new_hash: &str) -> Result<bool> {
        match self.get_file_hash(path)? {
            Some(stored_hash) => Ok(stored_hash != new_hash),
            None => Ok(true),
        }
    }

    /// Insert or update a file record.
    pub fn upsert_file(&self, file: &FileRecord, source: Option<&[u8]>) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO files (path, content_hash, size_bytes, language, last_indexed, source)
            VALUES (?, ?, ?, ?, unixepoch(), ?)
            "#,
            params![
                file.path,
                file.content_hash,
                file.size_bytes,
                file.language,
                source
            ],
        )?;
        Ok(())
    }

    /// Delete a file and all associated data.
    pub fn delete_file(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE path = ?", [path])?;
        Ok(())
    }

    /// Delete all symbols for a file.
    pub fn delete_symbols_for_file(&self, file_path: &str) -> Result<()> {
        // Delete edges first (foreign key constraint)
        self.conn.execute(
            "DELETE FROM edges WHERE source_id IN (SELECT id FROM symbols WHERE file_path = ?)",
            [file_path],
        )?;
        self.conn
            .execute("DELETE FROM symbols WHERE file_path = ?", [file_path])?;
        Ok(())
    }

    /// Insert a symbol.
    pub fn insert_symbol(&self, symbol: &Symbol) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO symbols (
                id, file_path, name, qualified_name, kind, visibility,
                signature, brief, docstring, line_start, line_end,
                col_start, col_end, parent_id, source
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            params![
                symbol.id,
                symbol.file_path,
                symbol.name,
                symbol.qualified_name,
                symbol.kind.as_str(),
                symbol.visibility.as_str(),
                symbol.signature,
                symbol.brief,
                symbol.docstring,
                symbol.line_start,
                symbol.line_end,
                symbol.col_start,
                symbol.col_end,
                symbol.parent_id,
                symbol.source,
            ],
        )?;
        Ok(())
    }

    /// Insert an edge.
    pub fn insert_edge(&self, edge: &Edge) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO edges (source_id, target_id, target_name, kind, line, col, context)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
            params![
                edge.source_id,
                edge.target_id,
                edge.target_name,
                edge.kind.as_str(),
                edge.line,
                edge.col,
                edge.context,
            ],
        )?;
        Ok(())
    }

    /// Insert module information.
    pub fn upsert_module(&self, module: &ModuleInfo) -> Result<()> {
        let exports_json = serde_json::to_string(&module.exports).unwrap_or_default();
        let imports_json = serde_json::to_string(&module.imports).unwrap_or_default();

        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO modules (file_path, module_name, exports, imports)
            VALUES (?, ?, ?, ?)
            "#,
            params![
                module.file_path,
                module.module_name,
                exports_json,
                imports_json,
            ],
        )?;
        Ok(())
    }

    /// Insert multiple symbols in a transaction (batch insert for parallel indexing).
    #[allow(dead_code)] // Useful for future batch operations
    pub fn insert_symbols_batch(&self, symbols: &[Symbol]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut count = 0;

        for symbol in symbols {
            tx.execute(
                r#"
                INSERT INTO symbols (
                    id, file_path, name, qualified_name, kind, visibility,
                    signature, brief, docstring, line_start, line_end,
                    col_start, col_end, parent_id, source
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
                params![
                    symbol.id,
                    symbol.file_path,
                    symbol.name,
                    symbol.qualified_name,
                    symbol.kind.as_str(),
                    symbol.visibility.as_str(),
                    symbol.signature,
                    symbol.brief,
                    symbol.docstring,
                    symbol.line_start,
                    symbol.line_end,
                    symbol.col_start,
                    symbol.col_end,
                    symbol.parent_id,
                    symbol.source,
                ],
            )?;
            count += 1;
        }

        tx.commit()?;
        Ok(count)
    }

    /// Insert multiple edges in a transaction (batch insert for parallel indexing).
    #[allow(dead_code)] // Useful for future batch operations
    pub fn insert_edges_batch(&self, edges: &[Edge]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut count = 0;

        for edge in edges {
            tx.execute(
                r#"
                INSERT INTO edges (source_id, target_id, target_name, kind, line, col, context)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
                params![
                    edge.source_id,
                    edge.target_id,
                    edge.target_name,
                    edge.kind.as_str(),
                    edge.line,
                    edge.col,
                    edge.context,
                ],
            )?;
            count += 1;
        }

        tx.commit()?;
        Ok(count)
    }

    /// Find a symbol by ID.
    pub fn get_symbol(&self, id: &str) -> Result<Option<Symbol>> {
        self.conn
            .query_row(
                r#"
                SELECT id, file_path, name, qualified_name, kind, visibility,
                       signature, brief, docstring, line_start, line_end,
                       col_start, col_end, parent_id, source
                FROM symbols WHERE id = ?
                "#,
                [id],
                |row| Ok(symbol_from_row(row)),
            )
            .optional()
    }

    /// Find symbols by name (exact or pattern).
    pub fn find_symbols(&self, pattern: &str, limit: i32) -> Result<Vec<Symbol>> {
        self.find_symbols_filtered(pattern, limit, None, None)
    }

    /// Find symbols by name with optional file path and kind filters.
    ///
    /// - `pattern`: Name pattern to search for
    /// - `limit`: Maximum number of results
    /// - `file_pattern`: Optional file path filter (supports glob syntax: `*`, `**`)
    /// - `kind_filter`: Optional symbol kind filter (function, method, struct, etc.)
    ///
    /// Results are ordered by match quality: exact name match first, then prefix match,
    /// then substring match.
    pub fn find_symbols_filtered(
        &self,
        pattern: &str,
        limit: i32,
        file_pattern: Option<&str>,
        kind_filter: Option<&str>,
    ) -> Result<Vec<Symbol>> {
        // Escape SQL LIKE special characters in the pattern
        let escaped_pattern = escape_like_pattern(pattern);
        let like_pattern = format!("%{}%", escaped_pattern);
        let starts_with_pattern = format!("{}%", escaped_pattern);

        // Convert glob-style file pattern to SQL LIKE pattern
        let file_like = file_pattern.map(glob_to_like_pattern);

        // Build dynamic SQL based on filters
        // We use separate parameters for exact match (?1), like match (?2), and starts_with (?3)
        let mut sql = String::from(
            r#"
            SELECT id, file_path, name, qualified_name, kind, visibility,
                   signature, brief, docstring, line_start, line_end,
                   col_start, col_end, parent_id, source
            FROM symbols
            WHERE (name LIKE ?2 OR qualified_name LIKE ?2)
            "#,
        );

        // Track next parameter position
        let mut next_param = 4; // ?1=pattern, ?2=like_pattern, ?3=starts_with

        // Add file pattern filter if provided
        let file_param_pos = if file_pattern.is_some() {
            let pos = next_param;
            sql.push_str(&format!(" AND file_path LIKE ?{}", pos));
            next_param += 1;
            Some(pos)
        } else {
            None
        };

        // Add kind filter if provided
        let kind_param_pos = if kind_filter.is_some() {
            let pos = next_param;
            sql.push_str(&format!(" AND kind = ?{}", pos));
            next_param += 1;
            Some(pos)
        } else {
            None
        };

        // Add ORDER BY with proper exact match detection
        // ?1 is the original pattern (for exact match)
        // ?3 is the starts_with pattern (for prefix match)
        sql.push_str(&format!(
            r#"
            ORDER BY
                CASE WHEN name = ?1 THEN 0
                     WHEN name LIKE ?3 THEN 1
                     ELSE 2 END,
                name
            LIMIT ?{}
            "#,
            next_param
        ));

        let mut stmt = self.conn.prepare(&sql)?;

        // Execute with appropriate parameters based on which filters are active
        let rows: Vec<Result<Symbol>> = match (
            file_like.as_deref(),
            kind_filter,
            file_param_pos,
            kind_param_pos,
        ) {
            (Some(fp), Some(kf), Some(_), Some(_)) => stmt
                .query_map(
                    params![pattern, like_pattern, starts_with_pattern, fp, kf, limit],
                    |row| Ok(symbol_from_row(row)),
                )?
                .collect(),
            (Some(fp), None, Some(_), None) => stmt
                .query_map(
                    params![pattern, like_pattern, starts_with_pattern, fp, limit],
                    |row| Ok(symbol_from_row(row)),
                )?
                .collect(),
            (None, Some(kf), None, Some(_)) => stmt
                .query_map(
                    params![pattern, like_pattern, starts_with_pattern, kf, limit],
                    |row| Ok(symbol_from_row(row)),
                )?
                .collect(),
            (None, None, None, None) => stmt
                .query_map(
                    params![pattern, like_pattern, starts_with_pattern, limit],
                    |row| Ok(symbol_from_row(row)),
                )?
                .collect(),
            // Handle impossible cases (filter provided but no param pos)
            _ => unreachable!("Filter and param position should match"),
        };

        rows.into_iter().collect()
    }

    /// Get the source code for a symbol.
    pub fn get_source(&self, symbol_id: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT source FROM symbols WHERE id = ?",
                [symbol_id],
                |row| row.get(0),
            )
            .optional()
    }

    /// Get all symbols in a file.
    pub fn get_file_symbols(&self, file_path: &str) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, file_path, name, qualified_name, kind, visibility,
                   signature, brief, docstring, line_start, line_end,
                   col_start, col_end, parent_id, source
            FROM symbols
            WHERE file_path = ?
            ORDER BY line_start
            "#,
        )?;

        let rows = stmt.query_map([file_path], |row| Ok(symbol_from_row(row)))?;
        rows.collect()
    }

    /// Find symbols in a specific file (alias for get_file_symbols).
    pub fn find_symbols_in_file(&self, file_path: &str) -> Result<Vec<Symbol>> {
        self.get_file_symbols(file_path)
    }

    /// Get edges from a symbol.
    pub fn get_outgoing_edges(&self, symbol_id: &str) -> Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT source_id, target_id, target_name, kind, line, col, context
            FROM edges
            WHERE source_id = ?
            ORDER BY line
            "#,
        )?;

        let rows = stmt.query_map([symbol_id], |row| Ok(edge_from_row(row)))?;
        rows.collect()
    }

    /// Get edges to a symbol (callers).
    pub fn get_incoming_edges(&self, target_name: &str) -> Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT source_id, target_id, target_name, kind, line, col, context
            FROM edges
            WHERE target_name = ? OR target_id = ?
            ORDER BY source_id
            "#,
        )?;

        let rows = stmt.query_map([target_name, target_name], |row| Ok(edge_from_row(row)))?;
        rows.collect()
    }

    /// Get codebase statistics.
    pub fn get_stats(&self) -> Result<CodebaseStats> {
        let files: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        let symbols: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))?;
        let edges: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))?;
        let functions: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM symbols WHERE kind IN ('function', 'method')",
            [],
            |row| row.get(0),
        )?;
        let structs: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM symbols WHERE kind IN ('struct', 'class')",
            [],
            |row| row.get(0),
        )?;
        let enums: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM symbols WHERE kind = 'enum'",
            [],
            |row| row.get(0),
        )?;
        let traits: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM symbols WHERE kind IN ('trait', 'interface')",
            [],
            |row| row.get(0),
        )?;

        Ok(CodebaseStats {
            files,
            symbols,
            edges,
            functions,
            structs,
            enums,
            traits,
        })
    }

    // ========== Shared Complexity Metrics ==========
    //
    // These mirror the DuckDB `complexity_analysis` formula exactly:
    // fan_out = COUNT(*) of 'calls' edges grouped by source_id,
    // fan_in  = COUNT(*) of 'calls' edges grouped by target_id (resolved only),
    // complexity = fan_out * 2 + fan_in.

    /// Per-symbol fan-in/fan-out/complexity metrics for functions and methods.
    ///
    /// Results are ordered by complexity (highest first).
    pub fn symbol_metrics(&self) -> Result<Vec<SymbolMetrics>> {
        let mut stmt = self.conn.prepare(
            r#"
            WITH fo AS (
                SELECT source_id AS id, COUNT(*) AS n
                FROM edges
                WHERE kind = 'calls'
                GROUP BY source_id
            ),
            fi AS (
                SELECT target_id AS id, COUNT(*) AS n
                FROM edges
                WHERE kind = 'calls' AND target_id IS NOT NULL
                GROUP BY target_id
            )
            SELECT
                s.id,
                s.name,
                s.qualified_name,
                s.kind,
                s.file_path,
                s.line_start,
                s.line_end,
                COALESCE(fi.n, 0) AS fan_in,
                COALESCE(fo.n, 0) AS fan_out,
                (COALESCE(fo.n, 0) * 2 + COALESCE(fi.n, 0)) AS complexity
            FROM symbols s
            LEFT JOIN fo ON s.id = fo.id
            LEFT JOIN fi ON s.id = fi.id
            WHERE s.kind IN ('function', 'method')
            ORDER BY complexity DESC, s.file_path, s.line_start
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(SymbolMetrics {
                id: row.get(0)?,
                name: row.get(1)?,
                qualified_name: row.get(2)?,
                kind: row.get(3)?,
                file_path: row.get(4)?,
                line_start: row.get(5)?,
                line_end: row.get(6)?,
                fan_in: row.get(7)?,
                fan_out: row.get(8)?,
                complexity: row.get(9)?,
            })
        })?;

        rows.collect()
    }

    /// Per-file aggregated complexity (same formula as [`Self::symbol_metrics`],
    /// summed over all symbols in the file).
    ///
    /// `symbol_count` counts all symbols in the file, not only functions.
    /// Results are ordered by complexity (highest first).
    pub fn file_complexity(&self) -> Result<Vec<FileComplexity>> {
        let mut stmt = self.conn.prepare(
            r#"
            WITH fo AS (
                SELECT source_id AS id, COUNT(*) AS n
                FROM edges
                WHERE kind = 'calls'
                GROUP BY source_id
            ),
            fi AS (
                SELECT target_id AS id, COUNT(*) AS n
                FROM edges
                WHERE kind = 'calls' AND target_id IS NOT NULL
                GROUP BY target_id
            )
            SELECT
                s.file_path,
                SUM(COALESCE(fo.n, 0) * 2 + COALESCE(fi.n, 0)) AS complexity,
                SUM(COALESCE(fo.n, 0)) AS fan_out,
                COUNT(*) AS symbol_count
            FROM symbols s
            LEFT JOIN fo ON s.id = fo.id
            LEFT JOIN fi ON s.id = fi.id
            GROUP BY s.file_path
            ORDER BY complexity DESC, s.file_path
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(FileComplexity {
                file_path: row.get(0)?,
                complexity: row.get(1)?,
                fan_out: row.get(2)?,
                symbol_count: row.get(3)?,
            })
        })?;

        rows.collect()
    }

    /// Count resolved incoming 'calls' edges for the given symbol IDs.
    ///
    /// Symbols with no incoming calls are absent from the returned map.
    pub fn fan_in_counts(&self, ids: &[String]) -> Result<std::collections::HashMap<String, i64>> {
        let mut counts = std::collections::HashMap::new();
        if ids.is_empty() {
            return Ok(counts);
        }

        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!(
            "SELECT target_id, COUNT(*) FROM edges \
             WHERE target_id IN ({}) AND kind = 'calls' \
             GROUP BY target_id",
            placeholders
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;

        for row in rows {
            let (id, n) = row?;
            counts.insert(id, n);
        }

        Ok(counts)
    }

    /// Get all indexed file paths.
    pub fn get_indexed_files(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files ORDER BY path")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    /// All resolved relationship edges whose endpoints live in different files.
    ///
    /// Used by `ctx check` to build the file-level dependency graph. Only
    /// `calls`/`implements`/`extends`/`uses` edges are included (`imports`
    /// edges are file-level and handled separately; `contains` and friends
    /// are structural, not dependencies).
    pub fn get_cross_file_edges(&self) -> Result<Vec<CrossFileEdge>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                s1.name, s1.qualified_name, s1.kind, s1.file_path, s1.line_start, s1.line_end,
                s2.name, s2.qualified_name, s2.kind, s2.file_path, s2.line_start, s2.line_end,
                e.kind, e.line
            FROM edges e
            JOIN symbols s1 ON e.source_id = s1.id
            JOIN symbols s2 ON e.target_id = s2.id
            WHERE e.target_id IS NOT NULL
              AND s1.file_path <> s2.file_path
              AND e.kind IN ('calls', 'implements', 'extends', 'uses')
            ORDER BY s1.file_path, e.line, s2.file_path
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(CrossFileEdge {
                source: EdgeSymbol {
                    name: row.get(0)?,
                    qualified_name: row.get(1)?,
                    kind: row.get(2)?,
                    file_path: row.get(3)?,
                    line_start: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    line_end: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                },
                target: EdgeSymbol {
                    name: row.get(6)?,
                    qualified_name: row.get(7)?,
                    kind: row.get(8)?,
                    file_path: row.get(9)?,
                    line_start: row.get::<_, Option<i64>>(10)?.unwrap_or(0),
                    line_end: row.get::<_, Option<i64>>(11)?.unwrap_or(0),
                },
                kind: row.get(12)?,
                line: row.get(13)?,
            })
        })?;

        rows.collect()
    }

    /// Per-file imports recorded in the `modules` table.
    ///
    /// The `imports` column stores a JSON array of [`ImportInfo`]; rows whose
    /// JSON fails to parse are skipped.
    pub fn get_file_imports(&self) -> Result<Vec<(String, Vec<ImportInfo>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_path, imports FROM modules \
             WHERE imports IS NOT NULL AND imports <> '' \
             ORDER BY file_path",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut result = Vec::new();
        for row in rows {
            let (file_path, json) = row?;
            if let Ok(imports) = serde_json::from_str::<Vec<ImportInfo>>(&json) {
                if !imports.is_empty() {
                    result.push((file_path, imports));
                }
            }
        }
        Ok(result)
    }

    /// File-level `imports` edges from the `edges` table.
    ///
    /// Some parsers (Go) record imports as edges whose `source_id` is the
    /// importing *file path* and whose `target_name` is the import path.
    /// Returns `(source, target_name, line)` tuples.
    pub fn get_import_edges(&self) -> Result<Vec<(String, String, Option<i64>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, target_name, line FROM edges \
             WHERE kind = 'imports' ORDER BY source_id, line",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect()
    }

    /// Normalize an FTS5 bm25 score into a 0-1 relevance value.
    fn bm25_relevance(rank: f64) -> f64 {
        if rank.is_finite() {
            // Use magnitude to normalize regardless of sign: |rank|/(1+|rank|).
            let abs_rank = rank.abs();
            (abs_rank / (1.0 + abs_rank)).clamp(0.0, 1.0)
        } else {
            0.0 // Return 0 for invalid scores
        }
    }

    /// Semantic search using FTS5 full-text search.
    /// Searches across name, signature, brief, and docstring fields.
    pub fn semantic_search(&self, query: &str, limit: i32) -> Result<Vec<(Symbol, f64)>> {
        // Preprocess query: split into keywords, handle natural language
        let keywords = preprocess_search_query(query);

        if keywords.is_empty() {
            return Ok(Vec::new());
        }

        // Build FTS5 query with OR logic for broader matches
        let fts_query = keywords.join(" OR ");

        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                s.id, s.file_path, s.name, s.qualified_name, s.kind, s.visibility,
                s.signature, s.brief, s.docstring, s.line_start, s.line_end,
                s.col_start, s.col_end, s.parent_id, s.source,
                bm25(symbol_fts) as rank
            FROM symbol_fts
            JOIN symbols s ON symbol_fts.id = s.id
            WHERE symbol_fts MATCH ?
            ORDER BY rank
            LIMIT ?
            "#,
        )?;

        let rows = stmt.query_map(params![fts_query, limit], |row| {
            let symbol = symbol_from_row(row);
            let rank: f64 = row.get(15)?;
            let relevance = Self::bm25_relevance(rank);
            Ok((symbol, relevance))
        })?;

        rows.collect()
    }

    /// Hybrid search combining exact match with semantic search.
    pub fn hybrid_search(&self, query: &str, limit: i32) -> Result<Vec<(Symbol, f64, String)>> {
        let mut results: std::collections::HashMap<String, (Symbol, f64, String)> =
            std::collections::HashMap::new();

        // Ensure we get at least 1 result from each source, even for small limits
        let half_limit = (limit / 2).max(1);

        // 1. Exact name matches (highest priority)
        let exact_matches = self.find_symbols(query, half_limit)?;
        for symbol in exact_matches {
            let score = if symbol.name.eq_ignore_ascii_case(query) {
                1.0 // Exact match
            } else if symbol
                .name
                .to_lowercase()
                .starts_with(&query.to_lowercase())
            {
                0.9 // Prefix match
            } else {
                0.7 // Contains match
            };
            results.insert(symbol.id.clone(), (symbol, score, "exact".to_string()));
        }

        // 2. Semantic matches (FTS5)
        if let Ok(semantic_matches) = self.semantic_search(query, half_limit) {
            for (symbol, relevance) in semantic_matches {
                results
                    .entry(symbol.id.clone())
                    .and_modify(|(_, existing_score, _)| {
                        *existing_score = existing_score.max(relevance);
                    })
                    .or_insert((symbol, relevance, "semantic".to_string()));
            }
        }

        // Sort by score and return
        let mut results: Vec<_> = results.into_values().collect();
        // Guard against NaN/Inf by treating non-finite values as lower priority
        results.sort_by(|a, b| {
            match (a.1.is_finite(), b.1.is_finite()) {
                (true, true) => b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal),
                (true, false) => std::cmp::Ordering::Less, // a (finite) is better than b (non-finite)
                (false, true) => std::cmp::Ordering::Greater, // b (finite) is better than a (non-finite)
                (false, false) => std::cmp::Ordering::Equal,
            }
        });
        results.truncate(limit as usize);

        Ok(results)
    }

    /// Rebuild the FTS index (useful after schema changes).
    #[allow(dead_code)]
    pub fn rebuild_fts_index(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            INSERT INTO symbol_fts(symbol_fts) VALUES('rebuild');
            "#,
        )?;
        Ok(())
    }

    // ========== Embedding Operations ==========

    /// Store an embedding for a symbol.
    ///
    /// This stores the embedding in two places:
    /// 1. The `embeddings` table (JSON format, for compatibility)
    /// 2. The `symbol_vectors` table (binary format, for fast KNN search via sqlite-vec)
    pub fn store_embedding(
        &self,
        symbol_id: &str,
        provider: &str,
        model: &str,
        vector: &[f32],
    ) -> Result<()> {
        let vector_json = serde_json::to_string(vector)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

        // Store in JSON embeddings table
        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO embeddings (symbol_id, provider, model, dimension, vector)
            VALUES (?, ?, ?, ?, ?)
            "#,
            params![symbol_id, provider, model, vector.len(), vector_json],
        )?;

        // Also store in vector table for fast KNN search (if available and dimension matches)
        if self.has_vector_search() && vector.len() == DEFAULT_EMBEDDING_DIM {
            // Convert to bytes for sqlite-vec (f32 little-endian)
            let vector_bytes: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();

            // Delete existing entry first (vec0 doesn't support REPLACE)
            let _ = self.conn.execute(
                "DELETE FROM symbol_vectors WHERE symbol_id = ?",
                [symbol_id],
            );

            self.conn.execute(
                "INSERT INTO symbol_vectors (embedding, symbol_id) VALUES (?, ?)",
                params![vector_bytes, symbol_id],
            )?;
        }

        Ok(())
    }

    /// Get the embedding for a symbol.
    #[allow(dead_code)]
    pub fn get_embedding(&self, symbol_id: &str) -> Result<Option<Vec<f32>>> {
        let result = self.conn.query_row(
            "SELECT vector FROM embeddings WHERE symbol_id = ?",
            [symbol_id],
            |row| {
                let json: String = row.get(0)?;
                Ok(json)
            },
        );

        match result {
            Ok(json) => {
                let vector: Vec<f32> = serde_json::from_str(&json).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                Ok(Some(vector))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    #[allow(clippy::type_complexity)]
    /// Get all embeddings with their symbol metadata.
    pub fn get_all_embeddings(
        &self,
    ) -> Result<Vec<(String, String, String, String, u32, Vec<f32>)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT e.symbol_id, s.name, s.kind, s.file_path, s.line_start, e.vector
            FROM embeddings e
            JOIN symbols s ON e.symbol_id = s.id
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            let symbol_id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let file_path: String = row.get(3)?;
            let line: u32 = row.get(4)?;
            let json: String = row.get(5)?;
            Ok((symbol_id, name, kind, file_path, line, json))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (symbol_id, name, kind, file_path, line, json) = row?;
            let vector: Vec<f32> = serde_json::from_str(&json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            results.push((symbol_id, name, kind, file_path, line, vector));
        }

        Ok(results)
    }

    /// Count symbols that have embeddings.
    pub fn count_embeddings(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))
    }

    /// Get metadata about stored embeddings (provider, model, dimension, count).
    ///
    /// Returns a list of (provider, model, dimension, count) tuples for each
    /// unique combination in the embeddings table. This is useful for detecting
    /// dimension mismatches when querying with a different embedding provider.
    pub fn get_embedding_metadata(&self) -> Result<Vec<(String, String, i64, i64)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT provider, model, dimension, COUNT(*) as count
            FROM embeddings
            GROUP BY provider, model, dimension
            ORDER BY count DESC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;

        rows.collect()
    }

    /// Get symbols that don't have embeddings yet.
    pub fn get_symbols_without_embeddings(&self, limit: i64) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT s.id, s.file_path, s.name, s.qualified_name, s.kind, s.visibility,
                   s.signature, s.brief, s.docstring, s.line_start, s.line_end,
                   s.col_start, s.col_end, s.parent_id, s.source
            FROM symbols s
            LEFT JOIN embeddings e ON s.id = e.symbol_id
            WHERE e.symbol_id IS NULL
            LIMIT ?
            "#,
        )?;

        let rows = stmt.query_map([limit], |row| {
            Ok(Symbol {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                qualified_name: row.get(3)?,
                kind: SymbolKind::from_str(&row.get::<_, String>(4)?)
                    .unwrap_or(SymbolKind::Function),
                visibility: Visibility::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
                signature: row.get(6)?,
                brief: row.get(7)?,
                docstring: row.get(8)?,
                line_start: row.get(9)?,
                line_end: row.get(10)?,
                col_start: row.get(11)?,
                col_end: row.get(12)?,
                parent_id: row.get(13)?,
                source: row.get(14)?,
            })
        })?;

        rows.collect()
    }

    /// Migrate existing embeddings from JSON table to vector table for fast KNN search.
    ///
    /// This copies all embeddings with the correct dimension to the symbol_vectors table.
    /// Returns the number of embeddings migrated.
    #[allow(dead_code)] // Migration utility for future use
    pub fn migrate_embeddings_to_vec(&self) -> Result<usize> {
        if !self.has_vector_search() {
            return Ok(0);
        }

        // Get all embeddings with matching dimension
        let mut stmt = self.conn.prepare(
            r#"
            SELECT symbol_id, vector FROM embeddings
            WHERE dimension = ?
            "#,
        )?;

        let rows = stmt.query_map([DEFAULT_EMBEDDING_DIM as i64], |row| {
            let symbol_id: String = row.get(0)?;
            let json: String = row.get(1)?;
            Ok((symbol_id, json))
        })?;

        let mut count = 0;
        for row in rows {
            let (symbol_id, json) = row?;
            let vector: Vec<f32> = match serde_json::from_str(&json) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Convert to bytes for sqlite-vec (f32 little-endian)
            let vector_bytes: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();

            // Skip if already exists
            let exists: bool = self
                .conn
                .query_row(
                    "SELECT 1 FROM symbol_vectors WHERE symbol_id = ?",
                    [&symbol_id],
                    |_| Ok(true),
                )
                .unwrap_or(false);

            if !exists
                && self
                    .conn
                    .execute(
                        "INSERT INTO symbol_vectors (embedding, symbol_id) VALUES (?, ?)",
                        params![vector_bytes, symbol_id],
                    )
                    .is_ok()
            {
                count += 1;
            }
        }

        Ok(count)
    }

    /// Get the count of embeddings in the vector table.
    pub fn count_vector_embeddings(&self) -> Result<i64> {
        if !self.has_vector_search() {
            return Ok(0);
        }
        self.conn
            .query_row("SELECT COUNT(*) FROM symbol_vectors", [], |row| row.get(0))
    }

    /// Fast vector similarity search using sqlite-vec.
    ///
    /// Returns the top-k most similar symbols to the query embedding.
    /// This uses indexed KNN search which is O(log n) instead of O(n).
    ///
    /// Returns (symbol_id, name, kind, file_path, line, distance) tuples.
    #[allow(clippy::type_complexity)]
    /// Distance is L2 distance (lower is more similar).
    pub fn vector_search(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(String, String, String, String, u32, f32)>> {
        if !self.has_vector_search() {
            return Ok(Vec::new());
        }

        if query_embedding.len() != DEFAULT_EMBEDDING_DIM {
            return Ok(Vec::new());
        }

        // Convert query to bytes for sqlite-vec (f32 little-endian)
        let query_bytes: Vec<u8> = query_embedding
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();

        // sqlite-vec KNN query - first get matching rowids, then join with symbols
        // The vec0 virtual table uses MATCH for KNN queries with LIMIT
        let mut stmt = self.conn.prepare(
            r#"
            SELECT knn.symbol_id, s.name, s.kind, s.file_path, s.line_start, knn.distance
            FROM (
                SELECT symbol_id, distance
                FROM symbol_vectors
                WHERE embedding MATCH ?
                ORDER BY distance
                LIMIT ?
            ) knn
            JOIN symbols s ON knn.symbol_id = s.id
            ORDER BY knn.distance
            "#,
        )?;

        let rows = stmt.query_map(params![query_bytes, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, f32>(5)?,
            ))
        })?;

        rows.collect()
    }

    /// Check if the vector table has any embeddings.
    pub fn has_vector_embeddings(&self) -> bool {
        self.count_vector_embeddings().unwrap_or(0) > 0
    }

    /// Delete embeddings for a specific provider/model.
    pub fn delete_embeddings(&self, provider: &str, model: Option<&str>) -> Result<usize> {
        if let Some(model) = model {
            self.conn.execute(
                "DELETE FROM embeddings WHERE provider = ? AND model = ?",
                params![provider, model],
            )
        } else {
            self.conn
                .execute("DELETE FROM embeddings WHERE provider = ?", [provider])
        }
    }

    /// Resolve target_id for edges that only have target_name.
    ///
    /// This performs cross-file symbol resolution by matching target_name to symbols
    /// in the database. Resolution priority:
    /// 1. Context match: the call context contains the type name (e.g., "TypeScriptParser::new()")
    /// 2. Unique: only one symbol with that name exists in the codebase
    /// 3. Same file unique: only one symbol with that name exists in the same file
    ///
    /// We intentionally avoid aggressive same-file matching because calls like
    /// `Vec::new()` would incorrectly match a local `new` function.
    ///
    /// Returns the number of edges that were resolved.
    pub fn resolve_edge_targets(&self) -> Result<usize> {
        // Step 1: Resolve edges where the context contains the qualified name
        // e.g., context "TypeScriptParser::new()" matches symbol with qualified_name "TypeScriptParser::new"
        // Only resolve if exactly one symbol matches to avoid ambiguous resolution
        let context_resolved = self.conn.execute(
            r#"
            UPDATE edges
            SET target_id = (
                SELECT t.id
                FROM symbols t
                WHERE t.name = edges.target_name
                  AND t.kind IN ('function', 'method')
                  AND t.qualified_name IS NOT NULL
                  AND edges.context LIKE '%' || t.qualified_name || '%'
            )
            WHERE target_id IS NULL
              AND context IS NOT NULL
              AND (
                SELECT COUNT(*)
                FROM symbols t
                WHERE t.name = edges.target_name
                  AND t.kind IN ('function', 'method')
                  AND t.qualified_name IS NOT NULL
                  AND edges.context LIKE '%' || t.qualified_name || '%'
              ) = 1
            "#,
            [],
        )?;

        // Step 2: Resolve edges where target name is unique across the codebase
        let unique_resolved = self.conn.execute(
            r#"
            UPDATE edges
            SET target_id = (
                SELECT id FROM symbols
                WHERE name = edges.target_name
                  AND kind IN ('function', 'method')
            )
            WHERE target_id IS NULL
              AND (
                SELECT COUNT(*) FROM symbols
                WHERE name = edges.target_name
                  AND kind IN ('function', 'method')
              ) = 1
            "#,
            [],
        )?;

        // Step 3: Resolve edges where target is unique in the same file
        // Only match if:
        // - There's exactly one function with that name in the same file
        // - The context doesn't suggest an external type (no :: prefix before the name)
        // - The context doesn't suggest an external type/receiver call
        //   (no ::, ., or -> before the function name)
        let same_file_unique = self.conn.execute(
            r#"
            UPDATE edges
            SET target_id = (
                SELECT t.id
                FROM symbols t
                JOIN symbols s ON s.id = edges.source_id
                WHERE t.name = edges.target_name
                  AND t.file_path = s.file_path
                  AND t.kind IN ('function', 'method')
            )
            WHERE target_id IS NULL
              AND (
                SELECT COUNT(*)
                FROM symbols t
                JOIN symbols s ON s.id = edges.source_id
                WHERE t.name = edges.target_name
                  AND t.file_path = s.file_path
                  AND t.kind IN ('function', 'method')
              ) = 1
              -- Exclude if context suggests an external type call (has :: before the function name)
              -- e.g., "Vec::new()" or "Parser::new()" should NOT match local "new" functions
              AND (
                context IS NULL
                OR (
                    context NOT LIKE '%::' || target_name || '(%'
                    AND context NOT LIKE '%.' || target_name || '(%'
                    AND context NOT LIKE '%->' || target_name || '(%'
                )
                OR context LIKE '%' || (
                        SELECT t.qualified_name
                        FROM symbols t
                        JOIN symbols s ON s.id = edges.source_id
                        WHERE t.name = edges.target_name
                          AND t.file_path = s.file_path
                          AND t.kind IN ('function', 'method')
                        LIMIT 1
                    ) || '(%'
              )
            "#,
            [],
        )?;

        Ok(context_resolved + unique_resolved + same_file_unique)
    }

    /// Find duplicate code blocks using content hashing and similarity.
    pub fn find_duplicates(
        &self,
        similarity_threshold: u32,
        min_lines: u32,
    ) -> Result<Vec<DuplicateResult>> {
        // Get all function/method symbols with their source code
        // Use DISTINCT and unique id to avoid duplicate rows
        let mut stmt = self.conn.prepare(
            r#"
            SELECT DISTINCT id, name, file_path, line_start, line_end, source
            FROM symbols
            WHERE kind IN ('function', 'method')
              AND source IS NOT NULL
              AND (line_end - line_start) >= ?
            ORDER BY id
            "#,
        )?;

        let symbols: Vec<(String, String, String, u32, u32, String)> = stmt
            .query_map([min_lines], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        let mut duplicates = Vec::new();
        let threshold = similarity_threshold as f64 / 100.0;

        // Track seen pairs to avoid duplicates
        let mut seen_pairs: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();

        // Compare each pair of symbols
        for i in 0..symbols.len() {
            for j in (i + 1)..symbols.len() {
                let (id1, name1, file1, line1, end1, source1) = &symbols[i];
                let (id2, name2, file2, line2, end2, source2) = &symbols[j];

                // Skip if same symbol (by id or by file+line)
                if id1 == id2 || (file1 == file2 && line1 == line2) {
                    continue;
                }

                // Create canonical pair key (smaller id first) to avoid duplicates
                let pair_key = if id1 < id2 {
                    (id1.clone(), id2.clone())
                } else {
                    (id2.clone(), id1.clone())
                };

                // Skip if we've already seen this pair
                if seen_pairs.contains(&pair_key) {
                    continue;
                }

                // Normalize and compare source code
                let norm1 = normalize_code(source1);
                let norm2 = normalize_code(source2);

                let similarity = calculate_similarity(&norm1, &norm2);

                if similarity >= threshold {
                    // Mark this pair as seen
                    seen_pairs.insert(pair_key);

                    // Create a content hash for grouping
                    let hash = format!("{:x}", md5_hash(&norm1));

                    duplicates.push(DuplicateResult {
                        name1: name1.clone(),
                        file1: file1.clone(),
                        line1: *line1,
                        name2: name2.clone(),
                        file2: file2.clone(),
                        line2: *line2,
                        similarity: similarity * 100.0,
                        lines: ((end1 - line1) + (end2 - line2)) / 2,
                        hash,
                    });
                }
            }
        }

        // Sort by similarity (highest first), guarding against NaN/Inf
        duplicates.sort_by(
            |a, b| match (a.similarity.is_finite(), b.similarity.is_finite()) {
                (true, true) => b
                    .similarity
                    .partial_cmp(&a.similarity)
                    .unwrap_or(std::cmp::Ordering::Equal),
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                (false, false) => std::cmp::Ordering::Equal,
            },
        );

        Ok(duplicates)
    }
}

/// Escape SQL LIKE special characters in a pattern.
///
/// SQLite LIKE uses `%` for any sequence and `_` for single character.
/// This function escapes these so they match literally.
fn escape_like_pattern(pattern: &str) -> String {
    pattern
        .replace('\\', "\\\\") // Escape backslash first
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Convert a glob-style pattern to a SQL LIKE pattern.
///
/// Supports:
/// - `*` -> `%` (match any sequence - note: in SQL LIKE this also matches `/`)
/// - `**` -> `%` (match any sequence including path separators)
/// - `**/` -> consumed, following pattern matches from any depth
/// - `?` -> `_` (match single character)
/// - Escapes literal `%` and `_` in the pattern
///
/// Limitations:
/// - SQL LIKE `%` matches across path separators, so `src/*.rs` will also match
///   `src/foo/bar.rs`. Use substring patterns like `*parser*` for simple filtering,
///   or rely on the prefix to narrow results.
///
/// Examples:
/// - `**/*.rs` -> `%.rs` (any .rs file at any depth)
/// - `src/**/*.rs` -> `src/%.rs` (any .rs file under src/)
/// - `*parser*` -> `%parser%` (any path containing "parser")
fn glob_to_like_pattern(pattern: &str) -> String {
    let mut result = String::with_capacity(pattern.len() * 2);
    let mut chars = pattern.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '*' => {
                // Check for **
                if chars.peek() == Some(&'*') {
                    chars.next();
                    // Check for **/ - this should match zero or more path segments
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        // **/ means "any path prefix including empty"
                        // Don't add % here - the following pattern (likely *.ext) will add it
                        // This allows **/*.rs to become %.rs instead of %%.rs
                        // But if there's nothing after, or it's not a *, we need the %
                        if chars.peek().is_none() || chars.peek() == Some(&'*') {
                            // **/ at end or followed by another * - don't add redundant %
                            continue;
                        }
                        // **/ followed by something else - add % to match the path prefix
                        result.push('%');
                    } else {
                        // ** without trailing / - just match anything
                        result.push('%');
                    }
                } else {
                    // Single * - match anything (in SQL LIKE, same as %)
                    result.push('%');
                }
            }
            '?' => result.push('_'),
            '%' => result.push_str("\\%"),
            '_' => result.push_str("\\_"),
            '\\' => result.push_str("\\\\"),
            _ => result.push(c),
        }
    }

    result
}

/// Normalize code for comparison (remove whitespace, comments, variable names).
fn normalize_code(code: &str) -> String {
    code.lines()
        .map(|line| {
            // Remove leading/trailing whitespace
            let trimmed = line.trim();
            // Remove single-line comments
            let without_comment = if let Some(idx) = trimmed.find("//") {
                &trimmed[..idx]
            } else if let Some(idx) = trimmed.find('#') {
                &trimmed[..idx]
            } else {
                trimmed
            };
            without_comment.trim()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Calculate similarity between two strings using Jaccard similarity on tokens.
fn calculate_similarity(s1: &str, s2: &str) -> f64 {
    let tokens1: std::collections::HashSet<&str> = s1
        .split(|c: char| {
            c.is_whitespace()
                || c == '('
                || c == ')'
                || c == '{'
                || c == '}'
                || c == ';'
                || c == ','
        })
        .filter(|t| !t.is_empty())
        .collect();

    let tokens2: std::collections::HashSet<&str> = s2
        .split(|c: char| {
            c.is_whitespace()
                || c == '('
                || c == ')'
                || c == '{'
                || c == '}'
                || c == ';'
                || c == ','
        })
        .filter(|t| !t.is_empty())
        .collect();

    if tokens1.is_empty() && tokens2.is_empty() {
        return 1.0;
    }
    if tokens1.is_empty() || tokens2.is_empty() {
        return 0.0;
    }

    let intersection = tokens1.intersection(&tokens2).count();
    let union = tokens1.union(&tokens2).count();

    intersection as f64 / union as f64
}

/// Simple MD5-like hash for grouping duplicates (not cryptographically secure).
fn md5_hash(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Preprocess a search query into keywords.
fn preprocess_search_query(query: &str) -> Vec<String> {
    // Common words to filter out
    let stop_words: std::collections::HashSet<&str> = [
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "must", "shall",
        "can", "need", "dare", "and", "or", "but", "if", "then", "else", "when", "where", "why",
        "how", "what", "which", "who", "whom", "this", "that", "these", "those", "i", "you", "he",
        "she", "it", "we", "they", "me", "him", "her", "us", "them", "my", "your", "his", "its",
        "our", "their", "for", "to", "from", "with", "at", "by", "on", "in", "of", "about", "into",
        "through", "during", "before", "after", "above", "below", "find", "get", "search", "look",
        "all", "any", "each", "every",
    ]
    .iter()
    .copied()
    .collect();

    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|word| {
            let word = word.trim();
            !word.is_empty() && word.len() > 1 && !stop_words.contains(word)
        })
        .map(|s| {
            // Add wildcard suffix for prefix matching
            format!("{}*", s)
        })
        .collect()
}

/// Helper to convert a row to a Symbol.
fn symbol_from_row(row: &rusqlite::Row) -> Symbol {
    let kind_str: String = row.get(4).unwrap_or_default();
    let visibility_str: String = row.get(5).unwrap_or_default();

    Symbol {
        id: row.get(0).unwrap_or_default(),
        file_path: row.get(1).unwrap_or_default(),
        name: row.get(2).unwrap_or_default(),
        qualified_name: row.get(3).ok(),
        kind: SymbolKind::from_str(&kind_str).unwrap_or(SymbolKind::Function),
        visibility: Visibility::from_str(&visibility_str).unwrap_or_default(),
        signature: row.get(6).ok(),
        brief: row.get(7).ok(),
        docstring: row.get(8).ok(),
        line_start: row.get(9).unwrap_or(0),
        line_end: row.get(10).unwrap_or(0),
        col_start: row.get(11).unwrap_or(0),
        col_end: row.get(12).unwrap_or(0),
        parent_id: row.get(13).ok(),
        source: row.get(14).ok(),
    }
}

/// Helper to convert a row to an Edge.
fn edge_from_row(row: &rusqlite::Row) -> Edge {
    let kind_str: String = row.get(3).unwrap_or_default();

    Edge {
        source_id: row.get(0).unwrap_or_default(),
        target_id: row.get(1).ok(),
        target_name: row.get(2).unwrap_or_default(),
        kind: EdgeKind::from_str(&kind_str).unwrap_or(EdgeKind::Calls),
        line: row.get(4).ok(),
        col: row.get(5).ok(),
        context: row.get(6).ok(),
    }
}

/// Per-symbol complexity metrics (see [`Database::symbol_metrics`]).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SymbolMetrics {
    pub id: String,
    pub name: String,
    pub qualified_name: Option<String>,
    pub kind: String,
    pub file_path: String,
    pub line_start: i64,
    pub line_end: i64,
    pub fan_in: i64,
    pub fan_out: i64,
    pub complexity: i64,
}

/// A resolved relationship edge whose endpoints live in different files
/// (see [`Database::get_cross_file_edges`]).
#[derive(Debug, Clone)]
pub struct CrossFileEdge {
    pub source: EdgeSymbol,
    pub target: EdgeSymbol,
    /// Edge kind (`calls`, `implements`, `extends`, `uses`).
    pub kind: String,
    /// Line in the source file where the reference occurs.
    pub line: Option<i64>,
}

/// Lightweight symbol info attached to a [`CrossFileEdge`].
#[derive(Debug, Clone)]
pub struct EdgeSymbol {
    pub name: String,
    pub qualified_name: Option<String>,
    pub kind: String,
    pub file_path: String,
    pub line_start: i64,
    pub line_end: i64,
}

/// Per-file aggregated complexity (see [`Database::file_complexity`]).
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileComplexity {
    pub file_path: String,
    pub complexity: i64,
    pub fan_out: i64,
    pub symbol_count: i64,
}

/// Result of duplicate detection.
#[derive(Debug, Clone)]
pub struct DuplicateResult {
    pub name1: String,
    pub file1: String,
    pub line1: u32,
    pub name2: String,
    pub file2: String,
    pub line2: u32,
    pub similarity: f64,
    pub lines: u32,
    pub hash: String,
}

/// Extension trait for optional query results.
trait ResultExt<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> ResultExt<T> for Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_database() {
        let db = Database::open_in_memory().unwrap();
        let stats = db.get_stats().unwrap();
        assert_eq!(stats.files, 0);
        assert_eq!(stats.symbols, 0);
    }

    #[test]
    fn test_fresh_database_is_stamped_with_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("codebase.sqlite");

        let db = Database::open(&db_path).unwrap();
        let version: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        drop(db);

        // Re-opening a stamped database works.
        assert!(Database::open(&db_path).is_ok());
    }

    #[test]
    fn test_v0_database_is_silently_stamped() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("codebase.sqlite");

        // Create a database, then reset it to the pre-versioning state (v0).
        drop(Database::open(&db_path).unwrap());
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.pragma_update(None, "user_version", 0).unwrap();
        }

        // Opening a v0 database succeeds and stamps the current version.
        let db = Database::open(&db_path).unwrap();
        let version: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn test_schema_version_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("codebase.sqlite");

        drop(Database::open(&db_path).unwrap());
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.pragma_update(None, "user_version", 99).unwrap();
        }

        let err = Database::open(&db_path).unwrap_err();
        match &err {
            crate::error::CtxError::SchemaVersionMismatch { found, expected } => {
                assert_eq!(*found, 99);
                assert_eq!(*expected, SCHEMA_VERSION);
            }
            other => panic!("expected SchemaVersionMismatch, got: {}", other),
        }
        assert!(err.to_string().contains("ctx index --force"));
    }

    #[test]
    fn test_sqlite_vec_extension_loaded() {
        // Verify sqlite-vec is properly initialized
        let db = Database::open_in_memory().unwrap();

        // Check that extension initialization was attempted
        // Note: init_vec_extension() is called during Database::open_in_memory()
        // so is_vec_extension_available() reflects whether it succeeded
        if !is_vec_extension_available() {
            eprintln!("Skipping sqlite-vec tests: extension not available on this platform");
            return;
        }

        // Check vec_version() function exists (proves extension loaded)
        let version: Result<String, _> = db
            .conn
            .query_row("SELECT vec_version()", [], |row| row.get(0));

        match version {
            Ok(v) => {
                assert!(
                    v.starts_with('v'),
                    "vec_version should return version string, got: {}",
                    v
                );
            }
            Err(e) => {
                // Extension registered but function not available - this shouldn't happen
                // if is_vec_extension_available() returned true, but handle gracefully
                panic!(
                    "sqlite-vec extension reported available but vec_version() failed: {}",
                    e
                );
            }
        }

        // Verify has_vector_search() works
        assert!(db.has_vector_search(), "vector search should be available");

        // Verify symbol_vectors table exists and is queryable
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM symbol_vectors", [], |row| row.get(0))
            .expect("symbol_vectors table should exist");
        assert_eq!(count, 0, "symbol_vectors should be empty initially");
    }

    /// Build a minimal function symbol for metrics tests.
    fn make_fn_symbol(id: &str, name: &str, file: &str, line: u32) -> Symbol {
        Symbol {
            id: id.to_string(),
            file_path: file.to_string(),
            name: name.to_string(),
            qualified_name: Some(name.to_string()),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: None,
            brief: None,
            docstring: None,
            line_start: line,
            line_end: line + 5,
            col_start: 0,
            col_end: 0,
            parent_id: None,
            source: None,
        }
    }

    /// Build a 'calls' edge for metrics tests.
    fn make_call_edge(source_id: &str, target_id: Option<&str>, target_name: &str) -> Edge {
        Edge {
            source_id: source_id.to_string(),
            target_id: target_id.map(|s| s.to_string()),
            target_name: target_name.to_string(),
            kind: EdgeKind::Calls,
            line: Some(1),
            col: None,
            context: None,
        }
    }

    /// Set up a database with three functions and a small call graph:
    /// alpha -> beta (resolved), alpha -> external (unresolved), beta -> alpha (resolved).
    fn metrics_fixture() -> Database {
        let db = Database::open_in_memory().unwrap();
        db.upsert_file(
            &FileRecord {
                path: "src/a.rs".to_string(),
                content_hash: "h1".to_string(),
                size_bytes: 10,
                language: Some("rust".to_string()),
                last_indexed: 0,
            },
            None,
        )
        .unwrap();

        db.insert_symbol(&make_fn_symbol("src/a.rs::alpha", "alpha", "src/a.rs", 1))
            .unwrap();
        db.insert_symbol(&make_fn_symbol("src/a.rs::beta", "beta", "src/a.rs", 10))
            .unwrap();
        db.insert_symbol(&make_fn_symbol("src/a.rs::gamma", "gamma", "src/a.rs", 20))
            .unwrap();

        db.insert_edge(&make_call_edge(
            "src/a.rs::alpha",
            Some("src/a.rs::beta"),
            "beta",
        ))
        .unwrap();
        db.insert_edge(&make_call_edge("src/a.rs::alpha", None, "external"))
            .unwrap();
        db.insert_edge(&make_call_edge(
            "src/a.rs::beta",
            Some("src/a.rs::alpha"),
            "alpha",
        ))
        .unwrap();

        db
    }

    #[test]
    fn test_symbol_metrics_mirrors_complexity_formula() {
        let db = metrics_fixture();
        let metrics = db.symbol_metrics().unwrap();
        assert_eq!(metrics.len(), 3);

        let get = |name: &str| metrics.iter().find(|m| m.name == name).unwrap();

        // alpha: fan_out 2 (beta + unresolved external), fan_in 1 (from beta)
        let alpha = get("alpha");
        assert_eq!(alpha.fan_out, 2);
        assert_eq!(alpha.fan_in, 1);
        assert_eq!(alpha.complexity, 2 * 2 + 1);
        assert_eq!(alpha.file_path, "src/a.rs");
        assert_eq!(alpha.line_start, 1);

        // beta: fan_out 1, fan_in 1
        let beta = get("beta");
        assert_eq!(beta.fan_out, 1);
        assert_eq!(beta.fan_in, 1);
        assert_eq!(beta.complexity, 3); // fan_out(1) * 2 + fan_in(1)

        // gamma: no edges at all
        let gamma = get("gamma");
        assert_eq!(gamma.fan_out, 0);
        assert_eq!(gamma.fan_in, 0);
        assert_eq!(gamma.complexity, 0);

        // Ordered by complexity, highest first
        assert_eq!(metrics[0].name, "alpha");
    }

    #[test]
    fn test_file_complexity_aggregates_per_file() {
        let db = metrics_fixture();
        let files = db.file_complexity().unwrap();
        assert_eq!(files.len(), 1);

        let f = &files[0];
        assert_eq!(f.file_path, "src/a.rs");
        assert_eq!(f.symbol_count, 3);
        assert_eq!(f.fan_out, 3);
        // alpha (5) + beta (3) + gamma (0)
        assert_eq!(f.complexity, 8);
    }

    #[test]
    fn test_fan_in_counts() {
        let db = metrics_fixture();

        let ids = vec![
            "src/a.rs::alpha".to_string(),
            "src/a.rs::beta".to_string(),
            "src/a.rs::gamma".to_string(),
        ];
        let counts = db.fan_in_counts(&ids).unwrap();
        assert_eq!(counts.get("src/a.rs::alpha"), Some(&1));
        assert_eq!(counts.get("src/a.rs::beta"), Some(&1));
        // gamma has no incoming resolved calls, so it is absent
        assert!(!counts.contains_key("src/a.rs::gamma"));

        // Empty input short-circuits
        assert!(db.fan_in_counts(&[]).unwrap().is_empty());
    }

    #[test]
    fn test_insert_and_find_symbol() {
        let db = Database::open_in_memory().unwrap();

        // Insert a file first (foreign key)
        let file = FileRecord {
            path: "src/main.rs".to_string(),
            content_hash: "abc123".to_string(),
            size_bytes: 100,
            language: Some("rust".to_string()),
            last_indexed: 0,
        };
        db.upsert_file(&file, None).unwrap();

        // Insert a symbol
        let symbol = Symbol {
            id: "src/main.rs::main".to_string(),
            file_path: "src/main.rs".to_string(),
            name: "main".to_string(),
            qualified_name: None,
            kind: SymbolKind::Function,
            visibility: Visibility::Private,
            signature: Some("fn main()".to_string()),
            brief: Some("Entry point".to_string()),
            docstring: None,
            line_start: 1,
            line_end: 5,
            col_start: 0,
            col_end: 1,
            parent_id: None,
            source: Some("fn main() {\n    println!(\"Hello\");\n}".to_string()),
        };
        db.insert_symbol(&symbol).unwrap();

        // Find it
        let found = db.get_symbol("src/main.rs::main").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "main");

        // Search for it
        let results = db.find_symbols("main", 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_find_symbols_exact_match_ordering() {
        let db = Database::open_in_memory().unwrap();

        // Insert a file
        let file = FileRecord {
            path: "src/test.rs".to_string(),
            content_hash: "abc123".to_string(),
            size_bytes: 100,
            language: Some("rust".to_string()),
            last_indexed: 0,
        };
        db.upsert_file(&file, None).unwrap();

        // Insert symbols with similar names - order matters for testing
        // We insert in reverse order to ensure ordering comes from query, not insertion
        let symbols = vec![
            ("new_large", "src/test.rs::new_large"), // substring match
            ("new_item", "src/test.rs::new_item"),   // prefix match
            ("renew", "src/test.rs::renew"),         // substring match (contains "new")
            ("new", "src/test.rs::new"),             // exact match
            ("new_thing", "src/test.rs::new_thing"), // prefix match
        ];

        for (name, id) in &symbols {
            let symbol = Symbol {
                id: id.to_string(),
                file_path: "src/test.rs".to_string(),
                name: name.to_string(),
                qualified_name: None,
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: None,
                brief: None,
                docstring: None,
                line_start: 1,
                line_end: 5,
                col_start: 0,
                col_end: 1,
                parent_id: None,
                source: None,
            };
            db.insert_symbol(&symbol).unwrap();
        }

        // Search for "new" - should get exact match first, then prefix matches, then substring
        let results = db.find_symbols("new", 10).unwrap();

        assert!(results.len() >= 4, "Should find at least 4 symbols");

        // First result should be exact match "new"
        assert_eq!(results[0].name, "new", "Exact match 'new' should be first");

        // Next results should be prefix matches (new_*), alphabetically sorted
        // new_item, new_large, new_thing should come before renew
        let prefix_matches: Vec<&str> = results[1..4].iter().map(|s| s.name.as_str()).collect();
        assert!(
            prefix_matches.iter().all(|n| n.starts_with("new")),
            "Positions 2-4 should be prefix matches, got: {:?}",
            prefix_matches
        );

        // "renew" should be last (substring but not prefix)
        let renew_pos = results.iter().position(|s| s.name == "renew");
        assert!(
            renew_pos.is_some() && renew_pos.unwrap() >= 4,
            "'renew' should be after prefix matches"
        );
    }

    #[test]
    fn test_find_symbols_with_file_filter() {
        let db = Database::open_in_memory().unwrap();

        // Insert two files
        for path in &["src/parser/rust.rs", "src/embeddings/local.rs"] {
            let file = FileRecord {
                path: path.to_string(),
                content_hash: "abc123".to_string(),
                size_bytes: 100,
                language: Some("rust".to_string()),
                last_indexed: 0,
            };
            db.upsert_file(&file, None).unwrap();
        }

        // Insert "new" in both files
        for (path, id) in &[
            ("src/parser/rust.rs", "src/parser/rust.rs::new"),
            ("src/embeddings/local.rs", "src/embeddings/local.rs::new"),
        ] {
            let symbol = Symbol {
                id: id.to_string(),
                file_path: path.to_string(),
                name: "new".to_string(),
                qualified_name: None,
                kind: SymbolKind::Method,
                visibility: Visibility::Public,
                signature: None,
                brief: None,
                docstring: None,
                line_start: 1,
                line_end: 5,
                col_start: 0,
                col_end: 1,
                parent_id: None,
                source: None,
            };
            db.insert_symbol(&symbol).unwrap();
        }

        // Without filter, should find both
        let all_results = db.find_symbols_filtered("new", 10, None, None).unwrap();
        assert_eq!(all_results.len(), 2);

        // With file filter for parser, should find only one
        let filtered = db
            .find_symbols_filtered("new", 10, Some("*parser*"), None)
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].file_path.contains("parser"));

        // With file filter for embeddings
        let filtered = db
            .find_symbols_filtered("new", 10, Some("*embeddings*"), None)
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].file_path.contains("embeddings"));
    }

    #[test]
    fn test_escape_like_pattern() {
        // Normal patterns should pass through
        assert_eq!(escape_like_pattern("new"), "new");
        assert_eq!(escape_like_pattern("foo_bar"), "foo\\_bar");

        // Special SQL LIKE characters should be escaped
        assert_eq!(escape_like_pattern("100%"), "100\\%");
        assert_eq!(escape_like_pattern("a_b"), "a\\_b");
        assert_eq!(escape_like_pattern("a%b_c"), "a\\%b\\_c");
    }

    #[test]
    fn test_glob_to_like_pattern() {
        // * becomes %
        assert_eq!(glob_to_like_pattern("*.rs"), "%.rs");
        assert_eq!(glob_to_like_pattern("src/*"), "src/%");

        // **/ is consumed when followed by *, preventing double %
        // This ensures **/*.rs becomes %.rs (not %%.rs)
        assert_eq!(glob_to_like_pattern("**/*.rs"), "%.rs");
        assert_eq!(glob_to_like_pattern("src/**/*.rs"), "src/%.rs");

        // ** without trailing / just becomes %
        assert_eq!(glob_to_like_pattern("src/**"), "src/%");

        // **/ followed by non-* adds the %
        assert_eq!(glob_to_like_pattern("**/foo.rs"), "%foo.rs");

        // ? becomes _
        assert_eq!(glob_to_like_pattern("file?.txt"), "file_.txt");

        // Literal % and _ are escaped
        assert_eq!(glob_to_like_pattern("100%"), "100\\%");
        assert_eq!(glob_to_like_pattern("a_b"), "a\\_b");
    }

    #[test]
    fn test_glob_pattern_matches_files() {
        let db = Database::open_in_memory().unwrap();

        // Insert files at different depths
        for path in &[
            "main.rs",
            "src/lib.rs",
            "src/parser/mod.rs",
            "src/parser/rust.rs",
            "tests/test.rs",
        ] {
            let file = FileRecord {
                path: path.to_string(),
                content_hash: "abc".to_string(),
                size_bytes: 100,
                language: Some("rust".to_string()),
                last_indexed: 0,
            };
            db.upsert_file(&file, None).unwrap();

            let symbol = Symbol {
                id: format!("{}::main", path),
                file_path: path.to_string(),
                name: "main".to_string(),
                qualified_name: None,
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: None,
                brief: None,
                docstring: None,
                line_start: 1,
                line_end: 5,
                col_start: 0,
                col_end: 1,
                parent_id: None,
                source: None,
            };
            db.insert_symbol(&symbol).unwrap();
        }

        // **/*.rs should match ALL .rs files at any depth
        let results = db
            .find_symbols_filtered("main", 20, Some("**/*.rs"), None)
            .unwrap();
        assert_eq!(results.len(), 5, "**/*.rs should match all .rs files");

        // src/**/*.rs should match all .rs files under src/
        let results = db
            .find_symbols_filtered("main", 20, Some("src/**/*.rs"), None)
            .unwrap();
        assert_eq!(
            results.len(),
            3,
            "src/**/*.rs should match src/lib.rs, src/parser/mod.rs, src/parser/rust.rs"
        );

        // Note: src/*.rs also matches nested files because SQL LIKE % matches /
        // This is a documented limitation. For precise matching, use substring patterns.
        let results = db
            .find_symbols_filtered("main", 20, Some("src/*.rs"), None)
            .unwrap();
        assert_eq!(
            results.len(),
            3,
            "src/*.rs matches all under src/ (SQL LIKE limitation)"
        );

        // *parser* should match files with "parser" in the path
        let results = db
            .find_symbols_filtered("main", 20, Some("*parser*"), None)
            .unwrap();
        assert_eq!(
            results.len(),
            2,
            "*parser* should match parser directory files"
        );
    }

    #[test]
    fn test_hybrid_search_limit_one() {
        // Tests that hybrid_search doesn't panic or return 0 results with limit=1
        // Previously, limit/2 = 0 caused no results to be returned
        let db = Database::open_in_memory().unwrap();

        // Insert a file and symbol
        let file = FileRecord {
            path: "src/main.rs".to_string(),
            content_hash: "abc123".to_string(),
            size_bytes: 100,
            language: Some("rust".to_string()),
            last_indexed: 0,
        };
        db.upsert_file(&file, None).unwrap();

        let symbol = Symbol {
            id: "src/main.rs::authenticate".to_string(),
            file_path: "src/main.rs".to_string(),
            name: "authenticate".to_string(),
            qualified_name: None,
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: Some("fn authenticate(user: &str)".to_string()),
            brief: Some("Authenticate a user".to_string()),
            docstring: None,
            line_start: 1,
            line_end: 5,
            col_start: 0,
            col_end: 1,
            parent_id: None,
            source: None,
        };
        db.insert_symbol(&symbol).unwrap();

        // hybrid_search with limit=1 should return exactly 1 result
        let results = db.hybrid_search("authenticate", 1).unwrap();
        assert_eq!(
            results.len(),
            1,
            "hybrid_search with limit=1 should return 1 result"
        );
        assert_eq!(results[0].0.name, "authenticate");
    }

    #[test]
    fn test_hybrid_search_limit_three() {
        // Tests hybrid_search behavior with limit=3 (where limit/2 = 1)
        let db = Database::open_in_memory().unwrap();

        // Insert a file
        let file = FileRecord {
            path: "src/test.rs".to_string(),
            content_hash: "abc123".to_string(),
            size_bytes: 100,
            language: Some("rust".to_string()),
            last_indexed: 0,
        };
        db.upsert_file(&file, None).unwrap();

        // Insert multiple symbols
        for (name, i) in &[("login", 1), ("logout", 2), ("log_error", 3)] {
            let symbol = Symbol {
                id: format!("src/test.rs::{}", name),
                file_path: "src/test.rs".to_string(),
                name: name.to_string(),
                qualified_name: None,
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: Some(format!("fn {}()", name)),
                brief: Some(format!("{} function", name)),
                docstring: None,
                line_start: *i,
                line_end: *i + 5,
                col_start: 0,
                col_end: 1,
                parent_id: None,
                source: None,
            };
            db.insert_symbol(&symbol).unwrap();
        }

        // hybrid_search with limit=3 should work correctly
        let results = db.hybrid_search("log", 3).unwrap();
        assert!(
            !results.is_empty(),
            "hybrid_search with limit=3 should return results"
        );
        assert!(results.len() <= 3, "hybrid_search should respect limit");
    }

    #[test]
    fn test_semantic_search_score_range() {
        // Tests that semantic search returns scores in valid 0-1 range
        let db = Database::open_in_memory().unwrap();

        // Insert a file and symbol
        let file = FileRecord {
            path: "src/main.rs".to_string(),
            content_hash: "abc123".to_string(),
            size_bytes: 100,
            language: Some("rust".to_string()),
            last_indexed: 0,
        };
        db.upsert_file(&file, None).unwrap();

        let symbol = Symbol {
            id: "src/main.rs::process_data".to_string(),
            file_path: "src/main.rs".to_string(),
            name: "process_data".to_string(),
            qualified_name: None,
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: Some("fn process_data(input: &str) -> Result<()>".to_string()),
            brief: Some("Process incoming data".to_string()),
            docstring: Some("Processes the incoming data and returns a result.".to_string()),
            line_start: 1,
            line_end: 10,
            col_start: 0,
            col_end: 1,
            parent_id: None,
            source: None,
        };
        db.insert_symbol(&symbol).unwrap();

        // Semantic search should return scores in 0-1 range
        let results = db.semantic_search("process data", 10).unwrap();
        for (symbol, score) in &results {
            assert!(
                *score >= 0.0 && *score <= 1.0,
                "Score for '{}' should be in 0-1 range, got {}",
                symbol.name,
                score
            );
            assert!(
                score.is_finite(),
                "Score for '{}' should be finite, got {}",
                symbol.name,
                score
            );
        }
    }

    #[test]
    fn test_bm25_relevance_mapping() {
        let cases = [
            (0.0, 0.0),
            (-1.0, 0.5),
            (1.0, 0.5),
            (-10.0, 10.0 / 11.0),
            (10.0, 10.0 / 11.0),
        ];

        for (rank, expected) in cases {
            let actual = Database::bm25_relevance(rank);
            assert!(
                (actual - expected).abs() < 1e-12,
                "rank {} expected {} got {}",
                rank,
                expected,
                actual
            );
        }

        assert_eq!(Database::bm25_relevance(f64::INFINITY), 0.0);
        assert_eq!(Database::bm25_relevance(f64::NEG_INFINITY), 0.0);
        assert_eq!(Database::bm25_relevance(f64::NAN), 0.0);
    }

    #[test]
    fn test_get_embedding_metadata() {
        // Tests the embedding metadata query used for dimension mismatch detection
        let db = Database::open_in_memory().unwrap();

        // Insert a file first (foreign key)
        let file = FileRecord {
            path: "src/main.rs".to_string(),
            content_hash: "abc123".to_string(),
            size_bytes: 100,
            language: Some("rust".to_string()),
            last_indexed: 0,
        };
        db.upsert_file(&file, None).unwrap();

        // Insert some symbols
        for (name, id) in &[("foo", "src/main.rs::foo"), ("bar", "src/main.rs::bar")] {
            let symbol = Symbol {
                id: id.to_string(),
                file_path: "src/main.rs".to_string(),
                name: name.to_string(),
                qualified_name: None,
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: None,
                brief: None,
                docstring: None,
                line_start: 1,
                line_end: 5,
                col_start: 0,
                col_end: 1,
                parent_id: None,
                source: None,
            };
            db.insert_symbol(&symbol).unwrap();
        }

        // No embeddings yet
        let metadata = db.get_embedding_metadata().unwrap();
        assert!(
            metadata.is_empty(),
            "Should have no embedding metadata initially"
        );

        // Store embeddings with different dimensions (simulating local vs OpenAI)
        let local_vec: Vec<f32> = vec![0.1; 384]; // Local embedding dimension
        let openai_vec: Vec<f32> = vec![0.2; 1536]; // OpenAI embedding dimension

        db.store_embedding("src/main.rs::foo", "local", "all-MiniLM-L6-v2", &local_vec)
            .unwrap();
        db.store_embedding(
            "src/main.rs::bar",
            "openai",
            "text-embedding-3-small",
            &openai_vec,
        )
        .unwrap();

        // Query metadata
        let metadata = db.get_embedding_metadata().unwrap();

        // Should have two distinct provider/dimension combinations
        assert_eq!(
            metadata.len(),
            2,
            "Should have 2 embedding metadata entries"
        );

        // Verify dimensions are recorded correctly
        let dims: Vec<i64> = metadata.iter().map(|(_, _, dim, _)| *dim).collect();
        assert!(
            dims.contains(&384),
            "Should have local embedding dimension 384"
        );
        assert!(
            dims.contains(&1536),
            "Should have OpenAI embedding dimension 1536"
        );

        // Verify providers are recorded correctly
        let providers: Vec<&str> = metadata.iter().map(|(p, _, _, _)| p.as_str()).collect();
        assert!(providers.contains(&"local"), "Should have local provider");
        assert!(providers.contains(&"openai"), "Should have openai provider");
    }

    #[test]
    fn test_vector_search() {
        // Test the fast vector search using sqlite-vec
        let db = Database::open_in_memory().unwrap();

        if !is_vec_extension_available() {
            eprintln!("Skipping vector search test: sqlite-vec not available");
            return;
        }

        // Insert a file first (foreign key)
        let file = FileRecord {
            path: "src/main.rs".to_string(),
            content_hash: "abc123".to_string(),
            size_bytes: 100,
            language: Some("rust".to_string()),
            last_indexed: 0,
        };
        db.upsert_file(&file, None).unwrap();

        // Insert multiple symbols
        for (name, id, line) in &[
            ("foo", "src/main.rs::foo", 1),
            ("bar", "src/main.rs::bar", 10),
            ("baz", "src/main.rs::baz", 20),
        ] {
            let symbol = Symbol {
                id: id.to_string(),
                file_path: "src/main.rs".to_string(),
                name: name.to_string(),
                qualified_name: None,
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: None,
                brief: None,
                docstring: None,
                line_start: *line,
                line_end: *line + 5,
                col_start: 0,
                col_end: 1,
                parent_id: None,
                source: None,
            };
            db.insert_symbol(&symbol).unwrap();
        }

        // Create embeddings with default dimension (1536)
        // Make foo and bar similar, baz different
        let mut foo_vec: Vec<f32> = vec![0.0; DEFAULT_EMBEDDING_DIM];
        foo_vec[0] = 1.0;
        foo_vec[1] = 0.5;

        let mut bar_vec: Vec<f32> = vec![0.0; DEFAULT_EMBEDDING_DIM];
        bar_vec[0] = 0.9;
        bar_vec[1] = 0.6;

        let mut baz_vec: Vec<f32> = vec![0.0; DEFAULT_EMBEDDING_DIM];
        baz_vec[0] = 0.0;
        baz_vec[1] = 0.0;
        baz_vec[2] = 1.0; // Completely different direction

        // Store embeddings (should also insert into symbol_vectors)
        db.store_embedding(
            "src/main.rs::foo",
            "openai",
            "text-embedding-3-small",
            &foo_vec,
        )
        .unwrap();
        db.store_embedding(
            "src/main.rs::bar",
            "openai",
            "text-embedding-3-small",
            &bar_vec,
        )
        .unwrap();
        db.store_embedding(
            "src/main.rs::baz",
            "openai",
            "text-embedding-3-small",
            &baz_vec,
        )
        .unwrap();

        // Verify vector embeddings were stored
        let vec_count = db.count_vector_embeddings().unwrap();
        assert_eq!(vec_count, 3, "Should have 3 vector embeddings");

        // Search with a query similar to foo
        let query: Vec<f32> = foo_vec.clone();
        let results = db.vector_search(&query, 3).unwrap();

        assert_eq!(results.len(), 3, "Should return 3 results");

        // First result should be foo (exact match)
        assert_eq!(
            results[0].0, "src/main.rs::foo",
            "First result should be foo (exact match)"
        );
        assert_eq!(results[0].5, 0.0, "Exact match should have distance 0");

        // Second result should be bar (similar)
        assert_eq!(
            results[1].0, "src/main.rs::bar",
            "Second result should be bar (similar)"
        );

        // Third result should be baz (different)
        assert_eq!(
            results[2].0, "src/main.rs::baz",
            "Third result should be baz (different)"
        );

        // baz should have larger distance than bar
        assert!(
            results[2].5 > results[1].5,
            "baz should have larger distance than bar"
        );
    }

    #[test]
    fn test_migrate_embeddings_to_vec() {
        // Test migrating existing JSON embeddings to vector table
        let db = Database::open_in_memory().unwrap();

        if !is_vec_extension_available() {
            eprintln!("Skipping migration test: sqlite-vec not available");
            return;
        }

        // Insert a file and symbol
        let file = FileRecord {
            path: "src/main.rs".to_string(),
            content_hash: "abc123".to_string(),
            size_bytes: 100,
            language: Some("rust".to_string()),
            last_indexed: 0,
        };
        db.upsert_file(&file, None).unwrap();

        let symbol = Symbol {
            id: "src/main.rs::foo".to_string(),
            file_path: "src/main.rs".to_string(),
            name: "foo".to_string(),
            qualified_name: None,
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: None,
            brief: None,
            docstring: None,
            line_start: 1,
            line_end: 5,
            col_start: 0,
            col_end: 1,
            parent_id: None,
            source: None,
        };
        db.insert_symbol(&symbol).unwrap();

        // Manually insert an embedding only in the JSON table (simulating old data)
        let vec: Vec<f32> = vec![0.1; DEFAULT_EMBEDDING_DIM];
        let vec_json = serde_json::to_string(&vec).unwrap();
        db.conn.execute(
            "INSERT INTO embeddings (symbol_id, provider, model, dimension, vector) VALUES (?, ?, ?, ?, ?)",
            params!["src/main.rs::foo", "openai", "test", DEFAULT_EMBEDDING_DIM as i64, vec_json],
        ).unwrap();

        // Verify not in vector table yet
        let vec_count_before = db.count_vector_embeddings().unwrap();
        assert_eq!(
            vec_count_before, 0,
            "Should have no vector embeddings before migration"
        );

        // Run migration
        let migrated = db.migrate_embeddings_to_vec().unwrap();
        assert_eq!(migrated, 1, "Should migrate 1 embedding");

        // Verify now in vector table
        let vec_count_after = db.count_vector_embeddings().unwrap();
        assert_eq!(
            vec_count_after, 1,
            "Should have 1 vector embedding after migration"
        );

        // Re-running migration should not duplicate
        let migrated_again = db.migrate_embeddings_to_vec().unwrap();
        assert_eq!(
            migrated_again, 0,
            "Should not re-migrate existing embeddings"
        );
    }
}
