//! SQLite schema and database operations.

use std::path::Path;

use rusqlite::{params, Connection, Result, Transaction};

use super::models::*;

/// SQLite database for code intelligence.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create a database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Create an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
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
        )
    }

    /// Begin a transaction.
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
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, file_path, name, qualified_name, kind, visibility,
                   signature, brief, docstring, line_start, line_end,
                   col_start, col_end, parent_id, source
            FROM symbols
            WHERE name LIKE ? OR qualified_name LIKE ?
            ORDER BY 
                CASE WHEN name = ? THEN 0 
                     WHEN name LIKE ? THEN 1 
                     ELSE 2 END,
                name
            LIMIT ?
            "#,
        )?;

        let like_pattern = format!("%{}%", pattern);
        let starts_with = format!("{}%", pattern);

        let rows = stmt.query_map(
            params![like_pattern, like_pattern, pattern, starts_with, limit],
            |row| Ok(symbol_from_row(row)),
        )?;

        rows.collect()
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

    /// Get all indexed file paths.
    pub fn get_indexed_files(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files ORDER BY path")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
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
            // Convert BM25 score (negative, lower is better) to a 0-1 relevance score
            let relevance = 1.0 / (1.0 - rank);
            Ok((symbol, relevance))
        })?;

        rows.collect()
    }

    /// Hybrid search combining exact match with semantic search.
    pub fn hybrid_search(&self, query: &str, limit: i32) -> Result<Vec<(Symbol, f64, String)>> {
        let mut results: std::collections::HashMap<String, (Symbol, f64, String)> = 
            std::collections::HashMap::new();

        // 1. Exact name matches (highest priority)
        let exact_matches = self.find_symbols(query, limit / 2)?;
        for symbol in exact_matches {
            let score = if symbol.name.eq_ignore_ascii_case(query) {
                1.0  // Exact match
            } else if symbol.name.to_lowercase().starts_with(&query.to_lowercase()) {
                0.9  // Prefix match
            } else {
                0.7  // Contains match
            };
            results.insert(symbol.id.clone(), (symbol, score, "exact".to_string()));
        }

        // 2. Semantic matches (FTS5)
        if let Ok(semantic_matches) = self.semantic_search(query, limit / 2) {
            for (symbol, relevance) in semantic_matches {
                results.entry(symbol.id.clone())
                    .and_modify(|(_, existing_score, _)| {
                        *existing_score = existing_score.max(relevance);
                    })
                    .or_insert((symbol, relevance, "semantic".to_string()));
            }
        }

        // Sort by score and return
        let mut results: Vec<_> = results.into_values().collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit as usize);

        Ok(results)
    }

    /// Rebuild the FTS index (useful after schema changes).
    pub fn rebuild_fts_index(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            INSERT INTO symbol_fts(symbol_fts) VALUES('rebuild');
            "#,
        )?;
        Ok(())
    }
}

/// Preprocess a search query into keywords.
fn preprocess_search_query(query: &str) -> Vec<String> {
    // Common words to filter out
    let stop_words: std::collections::HashSet<&str> = [
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did", "will", "would", "could",
        "should", "may", "might", "must", "shall", "can", "need", "dare",
        "and", "or", "but", "if", "then", "else", "when", "where", "why",
        "how", "what", "which", "who", "whom", "this", "that", "these",
        "those", "i", "you", "he", "she", "it", "we", "they", "me", "him",
        "her", "us", "them", "my", "your", "his", "its", "our", "their",
        "for", "to", "from", "with", "at", "by", "on", "in", "of", "about",
        "into", "through", "during", "before", "after", "above", "below",
        "find", "get", "search", "look", "all", "any", "each", "every",
    ].iter().copied().collect();

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
        visibility: Visibility::from_str(&visibility_str),
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
}
