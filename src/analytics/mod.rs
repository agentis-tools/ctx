//! Analytics module using DuckDB for complex graph queries.
//!
//! This module provides:
//! - Recursive call graph traversal
//! - Impact analysis (reverse call graph)
//! - Module dependency analysis
//! - Codebase statistics aggregations

use std::path::Path;

use duckdb::{params, Connection, Result};
use serde::{Deserialize, Serialize};

use crate::index::{CTX_DIR, DB_FILE};

/// DuckDB analytics engine for code intelligence.
pub struct Analytics {
    conn: Connection,
}

/// A node in a call graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraphNode {
    pub name: String,
    pub file_path: String,
    pub kind: String,
    pub depth: i32,
}

/// A node in impact analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactNode {
    pub name: String,
    pub file_path: String,
    pub kind: String,
    pub distance: i32,
}

/// Complexity analysis result for a function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityResult {
    pub name: String,
    pub file_path: String,
    pub line: u32,
    pub fan_out: i64,
    pub fan_in: i64,
    pub complexity_score: i64,
    pub severity: String,
}

/// Module dependency information.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ModuleDep {
    pub source_module: String,
    pub target_module: String,
    pub imported_names: Vec<String>,
}

/// File statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStats {
    pub file_path: String,
    pub symbol_count: i64,
    pub functions: i64,
    pub structs: i64,
    pub enums: i64,
    pub public_symbols: i64,
}

impl Analytics {
    /// Open DuckDB and attach the SQLite database.
    pub fn open(root: &Path) -> Result<Self> {
        let ctx_dir = root.join(CTX_DIR);
        let sqlite_path = ctx_dir.join(DB_FILE);

        // Create in-memory DuckDB and attach SQLite
        let conn = Connection::open_in_memory()?;

        // Attach SQLite database with properly escaped path
        // DuckDB uses single quotes for strings, so we need to escape any single quotes in the path
        let path_str = sqlite_path.display().to_string();
        let escaped_path = path_str.replace('\'', "''");
        conn.execute(
            &format!("ATTACH '{}' AS code (TYPE sqlite, READ_ONLY)", escaped_path),
            [],
        )?;

        // Create materialized views for better performance
        let analytics = Self { conn };
        analytics.create_materialized_views()?;

        Ok(analytics)
    }

    /// Create materialized views for common analytical queries.
    fn create_materialized_views(&self) -> Result<()> {
        // Call graph view - joins edges with symbols for fast traversal
        self.conn.execute_batch(
            r#"
            CREATE OR REPLACE VIEW call_graph AS
            SELECT 
                e.source_id,
                e.target_name,
                e.target_id,
                e.kind as edge_kind,
                e.line,
                e.context,
                s.file_path as source_file,
                s.name as source_name,
                s.kind as source_kind,
                t.file_path as target_file,
                t.name as target_name_resolved,
                t.kind as target_kind
            FROM code.edges e
            JOIN code.symbols s ON e.source_id = s.id
            LEFT JOIN code.symbols t ON e.target_id = t.id
            WHERE e.kind = 'calls';

            -- File statistics view
            CREATE OR REPLACE VIEW file_stats AS
            SELECT 
                file_path,
                COUNT(*) as symbol_count,
                COUNT(*) FILTER (WHERE kind IN ('function', 'method')) as functions,
                COUNT(*) FILTER (WHERE kind = 'struct') as structs,
                COUNT(*) FILTER (WHERE kind = 'enum') as enums,
                COUNT(*) FILTER (WHERE visibility = 'public') as public_symbols
            FROM code.symbols
            GROUP BY file_path;

            -- Symbol summary view
            CREATE OR REPLACE VIEW symbol_summary AS
            SELECT 
                kind,
                COUNT(*) as count,
                COUNT(*) FILTER (WHERE visibility = 'public') as public_count
            FROM code.symbols
            GROUP BY kind
            ORDER BY count DESC;
            "#,
        )?;

        Ok(())
    }

    /// Get the full call graph starting from a symbol (forward traversal).
    ///
    /// The `start_name` can be:
    /// - A simple name like "new" (matches first symbol with that name)
    /// - A qualified name like "LocalProvider::new"
    /// - A full ID like "src/embeddings/local.rs::LocalProvider::new@20"
    pub fn call_graph(&self, start_name: &str, max_depth: i32) -> Result<Vec<CallGraphNode>> {
        let mut stmt = self.conn.prepare(
            r#"
            WITH RECURSIVE start_symbol AS (
                -- Resolve the starting symbol by ID, qualified name, or name
                SELECT id, name, file_path, kind
                FROM code.symbols
                WHERE id = ?
                   OR qualified_name = ?
                   OR (name = ? AND NOT EXISTS (
                       SELECT 1 FROM code.symbols s2 
                       WHERE s2.id = ? OR s2.qualified_name = ?
                   ))
                ORDER BY file_path, line_start
                LIMIT 1
            ),
            graph AS (
                -- Base case: start from resolved symbol
                SELECT 
                    s.name,
                    s.file_path,
                    s.kind,
                    1 as depth,
                    s.id as current_id
                FROM code.symbols s
                JOIN start_symbol ss ON s.id = ss.id
                
                UNION ALL
                
                -- Recursive case: follow outgoing edges (only resolved edges)
                SELECT 
                    t.name,
                    t.file_path,
                    t.kind,
                    g.depth + 1 as depth,
                    t.id as current_id
                FROM graph g
                JOIN code.edges e ON e.source_id = g.current_id
                JOIN code.symbols t ON e.target_id = t.id
                WHERE g.depth < ?
                  AND e.kind = 'calls'
                  AND e.target_id IS NOT NULL
            )
            SELECT DISTINCT name, file_path, kind, MIN(depth) as depth
            FROM graph
            WHERE depth > 1  -- Exclude the starting symbol itself
            GROUP BY name, file_path, kind
            ORDER BY depth, name
            "#,
        )?;

        let rows = stmt.query_map(
            params![start_name, start_name, start_name, start_name, start_name, max_depth],
            |row| {
                Ok(CallGraphNode {
                    name: row.get(0)?,
                    file_path: row.get(1)?,
                    kind: row.get(2)?,
                    depth: row.get(3)?,
                })
            },
        )?;

        rows.collect()
    }

    /// Impact analysis: find all symbols that would be affected by changing the target.
    ///
    /// The `target_name` can be:
    /// - A simple name like "new" (matches first symbol with that name)
    /// - A qualified name like "LocalProvider::new"
    /// - A full ID like "src/embeddings/local.rs::LocalProvider::new@20"
    pub fn impact_analysis(&self, target_name: &str, max_depth: i32) -> Result<Vec<ImpactNode>> {
        let mut stmt = self.conn.prepare(
            r#"
            WITH RECURSIVE target_symbol AS (
                -- Resolve the target symbol by ID, qualified name, or name
                SELECT id, name, file_path
                FROM code.symbols
                WHERE id = ?
                   OR qualified_name = ?
                   OR (name = ? AND NOT EXISTS (
                       SELECT 1 FROM code.symbols s2 
                       WHERE s2.id = ? OR s2.qualified_name = ?
                   ))
                ORDER BY file_path, line_start
                LIMIT 1
            ),
            impact AS (
                -- Base case: find direct callers of the specific target symbol
                SELECT 
                    s.name,
                    s.file_path,
                    s.kind,
                    1 as distance,
                    s.id as current_id
                FROM code.edges e
                JOIN code.symbols s ON e.source_id = s.id
                JOIN target_symbol ts ON 
                    -- Only traverse resolved edges to avoid cross-file false positives
                    e.target_id IS NOT NULL AND e.target_id = ts.id
                WHERE e.kind = 'calls'
                
                UNION ALL
                
                -- Recursive case: find callers of callers (reverse traversal)
                SELECT 
                    s.name,
                    s.file_path,
                    s.kind,
                    i.distance + 1 as distance,
                    s.id as current_id
                FROM impact i
                JOIN code.edges e ON 
                    -- Only traverse resolved edges to avoid cross-file false positives
                    e.target_id IS NOT NULL AND e.target_id = i.current_id
                JOIN code.symbols s ON e.source_id = s.id
                WHERE i.distance < ?
                  AND e.kind = 'calls'
            )
            SELECT DISTINCT name, file_path, kind, MIN(distance) as distance
            FROM impact
            GROUP BY name, file_path, kind
            ORDER BY distance, name
            "#,
        )?;

        let rows = stmt.query_map(
            params![
                target_name,
                target_name,
                target_name,
                target_name,
                target_name,
                max_depth
            ],
            |row| {
                Ok(ImpactNode {
                    name: row.get(0)?,
                    file_path: row.get(1)?,
                    kind: row.get(2)?,
                    distance: row.get(3)?,
                })
            },
        )?;

        rows.collect()
    }

    /// Get file statistics for all indexed files.
    pub fn file_statistics(&self) -> Result<Vec<FileStats>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT file_path, symbol_count, functions, structs, enums, public_symbols
            FROM file_stats
            ORDER BY symbol_count DESC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(FileStats {
                file_path: row.get(0)?,
                symbol_count: row.get(1)?,
                functions: row.get(2)?,
                structs: row.get(3)?,
                enums: row.get(4)?,
                public_symbols: row.get(5)?,
            })
        })?;

        rows.collect()
    }

    /// Get symbol counts by kind.
    #[allow(dead_code)]
    pub fn symbol_summary(&self) -> Result<Vec<(String, i64, i64)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT kind, count, public_count
            FROM symbol_summary
            "#,
        )?;

        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;

        rows.collect()
    }

    /// Find if a path exists between two symbols (simplified version).
    ///
    /// Both `from_name` and `to_name` can be symbol IDs, qualified names, or simple names.
    #[allow(dead_code)]
    pub fn has_path(&self, from_name: &str, to_name: &str, max_depth: i32) -> Result<bool> {
        let mut stmt = self.conn.prepare(
            r#"
            WITH RECURSIVE source_symbol AS (
                SELECT id, name FROM code.symbols
                WHERE id = ? OR qualified_name = ? OR name = ?
                ORDER BY file_path
                LIMIT 1
            ),
            target_symbol AS (
                SELECT id, name FROM code.symbols
                WHERE id = ? OR qualified_name = ? OR name = ?
                ORDER BY file_path
                LIMIT 1
            ),
            reachable AS (
                -- Base case: start from source
                SELECT 
                    s.name,
                    s.id as current_id,
                    1 as depth
                FROM code.symbols s
                JOIN source_symbol src ON s.id = src.id
                
                UNION
                
                -- Follow edges using only resolved target_id to avoid cross-file false positives
                SELECT 
                    t.name,
                    t.id,
                    r.depth + 1
                FROM reachable r
                JOIN code.edges e ON e.source_id = r.current_id
                JOIN code.symbols t ON e.target_id = t.id
                WHERE r.depth < ?
                  AND e.kind = 'calls'
                  AND e.target_id IS NOT NULL
            )
            SELECT COUNT(*) > 0
            FROM reachable r
            JOIN target_symbol tgt ON r.current_id = tgt.id
            "#,
        )?;

        let result: bool = stmt.query_row(
            params![from_name, from_name, from_name, to_name, to_name, to_name, max_depth],
            |row| row.get(0),
        )?;
        Ok(result)
    }

    /// Get the most connected symbols (highest in/out degree).
    ///
    /// This correctly counts incoming edges per symbol ID, not by name,
    /// avoiding over-counting for common names like "new".
    pub fn most_connected(&self, limit: i32) -> Result<Vec<(String, String, i64, i64)>> {
        let mut stmt = self.conn.prepare(
            r#"
            WITH outgoing AS (
                SELECT source_id, COUNT(*) as out_degree
                FROM code.edges
                WHERE kind = 'calls'
                GROUP BY source_id
            ),
            incoming AS (
                -- Count incoming edges by target_id when available
                SELECT target_id as id, COUNT(*) as in_degree
                FROM code.edges
                WHERE kind = 'calls' AND target_id IS NOT NULL
                GROUP BY target_id
            )
            SELECT 
                s.name,
                s.file_path,
                COALESCE(o.out_degree, 0) as out_degree,
                COALESCE(i.in_degree, 0) as in_degree
            FROM code.symbols s
            LEFT JOIN outgoing o ON s.id = o.source_id
            LEFT JOIN incoming i ON s.id = i.id
            WHERE s.kind IN ('function', 'method')
            ORDER BY (COALESCE(o.out_degree, 0) + COALESCE(i.in_degree, 0)) DESC
            LIMIT ?
            "#,
        )?;

        let rows = stmt.query_map(params![limit], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;

        rows.collect()
    }

    /// Check if there are any self-recursive functions.
    ///
    /// Uses target_id when available for accurate recursion detection.
    #[allow(dead_code)]
    pub fn find_recursive_functions(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT DISTINCT s.name, s.file_path
            FROM code.edges e
            JOIN code.symbols s ON e.source_id = s.id
            WHERE e.kind = 'calls'
              AND (
                  (e.target_id IS NOT NULL AND e.target_id = s.id)
                  OR (e.target_id IS NULL AND e.target_name = s.name)
              )
            ORDER BY s.name
            "#,
        )?;

        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Get dependency graph between files (module-level).
    ///
    /// Uses target_id when available for accurate file resolution.
    pub fn file_dependencies(&self) -> Result<Vec<(String, String, i64)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT 
                s.file_path as source_file,
                COALESCE(t.file_path, 'external') as target_file,
                COUNT(*) as call_count
            FROM code.edges e
            JOIN code.symbols s ON e.source_id = s.id
            LEFT JOIN code.symbols t ON 
                (e.target_id IS NOT NULL AND e.target_id = t.id)
                OR (e.target_id IS NULL AND e.target_name = t.name)
            WHERE e.kind = 'calls'
              AND s.file_path != COALESCE(t.file_path, '')
            GROUP BY s.file_path, COALESCE(t.file_path, 'external')
            ORDER BY call_count DESC
            "#,
        )?;

        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;

        rows.collect()
    }

    /// Analyze code complexity based on fan-out (outgoing calls) and fan-in (incoming calls).
    ///
    /// This correctly counts incoming edges per symbol ID, not by name,
    /// avoiding over-counting for common names like "new".
    pub fn complexity_analysis(&self, threshold: i64) -> Result<Vec<ComplexityResult>> {
        let mut stmt = self.conn.prepare(
            r#"
            WITH outgoing AS (
                SELECT source_id, COUNT(*) as out_degree
                FROM code.edges
                WHERE kind = 'calls'
                GROUP BY source_id
            ),
            incoming AS (
                -- Count incoming edges by target_id when available
                SELECT target_id as id, COUNT(*) as in_degree
                FROM code.edges
                WHERE kind = 'calls' AND target_id IS NOT NULL
                GROUP BY target_id
            )
            SELECT 
                s.name,
                s.file_path,
                s.line_start,
                COALESCE(o.out_degree, 0) as fan_out,
                COALESCE(i.in_degree, 0) as fan_in,
                -- Complexity score: weighted combination of fan-out (more important) and fan-in
                (COALESCE(o.out_degree, 0) * 2 + COALESCE(i.in_degree, 0)) as complexity_score,
                CASE 
                    WHEN COALESCE(o.out_degree, 0) > 50 THEN 'critical'
                    WHEN COALESCE(o.out_degree, 0) > 30 THEN 'high'
                    WHEN COALESCE(o.out_degree, 0) > ? THEN 'medium'
                    ELSE 'low'
                END as severity
            FROM code.symbols s
            LEFT JOIN outgoing o ON s.id = o.source_id
            LEFT JOIN incoming i ON s.id = i.id
            WHERE s.kind IN ('function', 'method')
            ORDER BY complexity_score DESC
            "#,
        )?;

        let rows = stmt.query_map(params![threshold], |row| {
            Ok(ComplexityResult {
                name: row.get(0)?,
                file_path: row.get(1)?,
                line: row.get(2)?,
                fan_out: row.get(3)?,
                fan_in: row.get(4)?,
                complexity_score: row.get(5)?,
                severity: row.get(6)?,
            })
        })?;

        rows.collect()
    }

    /// Get the full call graph (all edges with resolved symbols).
    ///
    /// Uses target_id when available for accurate symbol resolution.
    pub fn full_call_graph(
        &self,
        _max_depth: i32,
    ) -> Result<Vec<(String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT DISTINCT
                s.file_path as source_file,
                s.name as source_name,
                COALESCE(t.file_path, 'external') as target_file,
                COALESCE(t.name, e.target_name) as target_name
            FROM code.edges e
            JOIN code.symbols s ON e.source_id = s.id
            LEFT JOIN code.symbols t ON 
                (e.target_id IS NOT NULL AND e.target_id = t.id)
                OR (e.target_id IS NULL AND e.target_name = t.name)
            WHERE e.kind = 'calls'
            ORDER BY s.file_path, s.name
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;

        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::Connection;

    fn setup_test_db() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;

        // Create schema
        conn.execute_batch(
            r#"
            CREATE SCHEMA IF NOT EXISTS code;
            
            CREATE TABLE code.symbols (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                qualified_name TEXT,
                kind TEXT NOT NULL,
                file_path TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                visibility TEXT
            );
            
            CREATE TABLE code.edges (
                source_id TEXT NOT NULL,
                target_id TEXT,
                target_name TEXT NOT NULL,
                kind TEXT NOT NULL,
                context TEXT,
                FOREIGN KEY (source_id) REFERENCES code.symbols(id)
            );
            
            -- Insert test data: main -> run -> helper
            INSERT INTO code.symbols VALUES 
                ('main@1', 'main', NULL, 'function', 'test.rs', 1, 10, 'public'),
                ('run@11', 'run', NULL, 'function', 'test.rs', 11, 20, 'private'),
                ('helper@21', 'helper', NULL, 'function', 'test.rs', 21, 30, 'private');
            
            INSERT INTO code.edges VALUES 
                ('main@1', 'run@11', 'run', 'calls', NULL),
                ('run@11', 'helper@21', 'helper', 'calls', NULL);
            "#,
        )?;

        Ok(conn)
    }

    #[test]
    fn test_call_graph_syntax() {
        let conn = setup_test_db().expect("Failed to setup test db");
        let analytics = Analytics { conn };

        let result = analytics.call_graph("main", 5);
        assert!(
            result.is_ok(),
            "call_graph query failed: {:?}",
            result.err()
        );

        let nodes = result.unwrap();
        assert_eq!(nodes.len(), 2, "Expected 2 nodes (run, helper)");
        assert_eq!(nodes[0].name, "run");
        assert_eq!(nodes[1].name, "helper");
    }

    #[test]
    fn test_impact_analysis_syntax() {
        let conn = setup_test_db().expect("Failed to setup test db");
        let analytics = Analytics { conn };

        let result = analytics.impact_analysis("helper", 5);
        assert!(
            result.is_ok(),
            "impact_analysis query failed: {:?}",
            result.err()
        );

        let nodes = result.unwrap();
        assert_eq!(nodes.len(), 2, "Expected 2 nodes (run, main)");
        // run calls helper directly (distance 1)
        // main calls run which calls helper (distance 2)
        assert!(nodes.iter().any(|n| n.name == "run" && n.distance == 1));
        assert!(nodes.iter().any(|n| n.name == "main" && n.distance == 2));
    }

    #[test]
    fn test_has_path_syntax() {
        let conn = setup_test_db().expect("Failed to setup test db");
        let analytics = Analytics { conn };

        // main -> run -> helper (path exists)
        let result = analytics.has_path("main", "helper", 5);
        assert!(result.is_ok(), "has_path query failed: {:?}", result.err());
        assert!(result.unwrap(), "Expected path from main to helper");

        // helper -> main (no path in reverse direction)
        let result = analytics.has_path("helper", "main", 5);
        assert!(result.is_ok(), "has_path query failed: {:?}", result.err());
        assert!(!result.unwrap(), "Expected no path from helper to main");
    }

    #[test]
    fn test_sql_injection_in_path_escaping() {
        // Test that paths with special SQL characters are properly escaped
        // This is a unit test for the escaping logic - the actual open() function
        // requires a real SQLite file, so we test the escaping directly

        // Test paths that would cause SQL injection if not escaped
        let test_paths = [
            "normal/path.db",
            "path with spaces/file.db",
            "path'with'quotes/file.db",
            "path''with''double/file.db",
            "path;DROP TABLE code;/file.db",
            "path' OR '1'='1/file.db",
        ];

        for path in &test_paths {
            // Simulate the escaping done in Analytics::open()
            let escaped = path.replace('\'', "''");
            let sql = format!("ATTACH '{}' AS code", escaped);

            // The escaped SQL should have balanced quotes
            let quote_count = sql.chars().filter(|c| *c == '\'').count();
            assert_eq!(
                quote_count % 2,
                0,
                "SQL for path '{}' has unbalanced quotes: {}",
                path,
                sql
            );

            // Single quotes in the path should be doubled
            if path.contains('\'') {
                assert!(
                    escaped.contains("''"),
                    "Path with quote should have doubled quotes: {}",
                    escaped
                );
            }
        }
    }
}
