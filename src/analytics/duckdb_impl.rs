//! Analytics module using DuckDB for complex graph queries.
//!
//! This module provides:
//! - Recursive call graph traversal
//! - Impact analysis (reverse call graph)
//! - Module dependency analysis
//! - Codebase statistics aggregations

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use duckdb::types::ValueRef;
use duckdb::{params, Connection, InterruptHandle, Result};

use super::{CallGraphNode, ComplexityResult, FileStats, ImpactNode, LocatedImpactNode};
use crate::index::{CTX_DIR, DB_FILE};

/// DuckDB analytics engine for code intelligence.
pub struct Analytics {
    conn: Connection,
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
        let mut seen = HashSet::new();
        Ok(self
            .impact_analysis_located(target_name, max_depth)?
            .into_iter()
            .filter(|node| {
                seen.insert((node.name.clone(), node.file_path.clone(), node.kind.clone()))
            })
            .map(|node| ImpactNode {
                name: node.name,
                file_path: node.file_path,
                kind: node.kind,
                distance: node.distance,
            })
            .collect())
    }

    /// Impact analysis with stable symbol identities and indexed source locations.
    ///
    /// The `target_name` accepts the same ID, qualified-name, and simple-name forms
    /// as [`Self::impact_analysis`]. Legacy indexes with missing line values report
    /// `0` for the corresponding bound.
    pub fn impact_analysis_located(
        &self,
        target_name: &str,
        max_depth: i32,
    ) -> Result<Vec<LocatedImpactNode>> {
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
                    s.id,
                    s.name,
                    s.qualified_name,
                    s.file_path,
                    s.kind,
                    s.line_start,
                    s.line_end,
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
                    s.id,
                    s.name,
                    s.qualified_name,
                    s.file_path,
                    s.kind,
                    s.line_start,
                    s.line_end,
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
            SELECT current_id, name, qualified_name, file_path, kind,
                   line_start, line_end, MIN(distance) as distance
            FROM impact
            GROUP BY current_id, name, qualified_name, file_path, kind, line_start, line_end
            ORDER BY distance, name, current_id
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
                Ok(LocatedImpactNode {
                    symbol_id: row.get(0)?,
                    name: row.get(1)?,
                    qualified_name: row.get(2)?,
                    file_path: row.get(3)?,
                    kind: row.get(4)?,
                    line_start: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    line_end: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    distance: row.get(7)?,
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

    // ========================================================================
    // Raw SQL surface (`ctx sql`) — hardened, read-only query sandbox.
    // ========================================================================

    /// Open DuckDB for the `ctx sql` command: attach the SQLite index read-only,
    /// build the public `v1` view layer, optionally materialize snapshot
    /// partitions as `snap.*` tables, then lock the engine down so untrusted
    /// user SQL cannot touch the filesystem, load extensions, or re-enable any
    /// of that. Safety is enforced entirely by engine configuration — never by
    /// inspecting the SQL text.
    ///
    /// `snapshots`, when set, is a directory of `sha=<sha>/` Parquet
    /// partitions written by `ctx snapshot`; each partition's four files are
    /// loaded into in-memory tables `snap.files`, `snap.symbols`,
    /// `snap.dup_pairs`, and `snap.meta`.
    pub fn open_sql_sandbox(root: &Path, snapshots: Option<&Path>) -> crate::error::Result<Self> {
        // Attach the index and build the public `v1` contract views BEFORE
        // hardening — creating views and reading the attached DB must happen
        // while access is still allowed.
        let analytics = Self::open_with_public_schema(root)?;

        // Load snapshot partitions BEFORE hardening, as MATERIALIZED tables
        // (CREATE TABLE ... AS), not views. This ordering is load-bearing:
        // `enable_external_access = false` below disables all filesystem reads
        // at query time, so a lazy view over read_parquet() would fail on its
        // first use — the Parquet data must be fully read into memory now.
        if let Some(dir) = snapshots {
            if !Self::has_snapshot_partitions(dir) {
                return Err(crate::error::CtxError::Other(format!(
                    "no snapshots found under {}; run 'ctx snapshot' first",
                    dir.display()
                )));
            }
            // Single-quote-escape the glob path, like the ATTACH path above.
            let escaped_dir = dir.display().to_string().replace('\'', "''");
            analytics.conn.execute_batch("CREATE SCHEMA snap;")?;
            for table in ["files", "symbols", "dup_pairs", "meta"] {
                // `hive_partitioning = false`: every row already carries
                // `commit_sha`; don't add a duplicate `sha` column from the
                // partition directory name.
                analytics.conn.execute_batch(&format!(
                    "CREATE TABLE snap.{table} AS \
                     SELECT * FROM read_parquet('{escaped_dir}/sha=*/{table}.parquet', \
                     union_by_name = true, hive_partitioning = false);"
                ))?;
            }
        }

        // Engine-level hardening (order matters: memory + external-access first,
        // then lock configuration, which blocks any further `SET`).
        //
        // `enable_external_access = false` disables the filesystem (COPY,
        // read_csv, file-based ATTACH), extension installation, and config
        // changes; `lock_configuration = true` prevents user SQL from re-enabling
        // any of it. The SQLite index is attached READ_ONLY, so it cannot be
        // mutated. Note: DuckDB 1.4 has no engine toggle to reject a bare
        // in-memory `ATTACH ':memory:'`; that is permitted but inert (no
        // filesystem access, the index stays read-only, and the one-result-set
        // rule blocks chaining a write onto it). We do not add SQL-text filtering
        // for it — safety is enforced by engine configuration, not by parsing.
        let mem_limit =
            std::env::var("CTX_SQL_MEMORY_LIMIT").unwrap_or_else(|_| "512MB".to_string());
        let mem_escaped = mem_limit.replace('\'', "''");
        analytics.conn.execute_batch(&format!(
            "SET memory_limit = '{}';\n\
             SET enable_external_access = false;\n\
             SET lock_configuration = true;",
            mem_escaped
        ))?;

        Ok(analytics)
    }

    /// Whether `dir` contains at least one `sha=<sha>/` snapshot partition.
    fn has_snapshot_partitions(dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries.filter_map(|e| e.ok()).any(|e| {
                    e.file_name().to_string_lossy().starts_with("sha=") && e.path().is_dir()
                })
            })
            .unwrap_or(false)
    }

    /// Open DuckDB for snapshot export (`ctx snapshot`): attach the SQLite
    /// index read-only and build the same public `v1` view layer as
    /// [`Analytics::open_sql_sandbox`], but with **no** engine hardening
    /// (`enable_external_access` stays on so `COPY ... TO ... (FORMAT
    /// PARQUET)` can write partition files).
    ///
    /// This is a trusted internal path: it only ever executes SQL composed by
    /// ctx itself and is never fed user SQL. Anything user-facing must go
    /// through the hardened [`Analytics::open_sql_sandbox`] instead.
    pub fn open_export(root: &Path) -> Result<Self> {
        Self::open_with_public_schema(root)
    }

    /// Shared constructor for [`Analytics::open_sql_sandbox`] and
    /// [`Analytics::open_export`]: in-memory DuckDB with the SQLite index
    /// attached read-only as `code` and the public `v1` views created.
    /// Performs no hardening — callers decide the trust level.
    fn open_with_public_schema(root: &Path) -> Result<Self> {
        let ctx_dir = root.join(CTX_DIR);
        let sqlite_path = ctx_dir.join(DB_FILE);

        let conn = Connection::open_in_memory()?;

        // Attach the SQLite index read-only (single-quote-escape the path).
        let path_str = sqlite_path.display().to_string();
        let escaped_path = path_str.replace('\'', "''");
        conn.execute(
            &format!("ATTACH '{}' AS code (TYPE sqlite, READ_ONLY)", escaped_path),
            [],
        )?;

        let analytics = Self { conn };

        let index_root = root.display().to_string();
        analytics.create_public_schema_v1(env!("CARGO_PKG_VERSION"), &index_root)?;

        Ok(analytics)
    }

    /// Raw connection access for trusted internal callers (snapshot export
    /// runs ctx-composed DDL/COPY and uses `duckdb::Appender` directly).
    /// Never expose this to user SQL.
    pub(crate) fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Create the versioned public schema `v1` — the stable query surface.
    ///
    /// These views (not the physical `code.*` tables) are the contract. Column
    /// lists here are the documented `v1` schema; derived columns (fan-in/out,
    /// complexity) are computed in-view.
    fn create_public_schema_v1(&self, ctx_version: &str, index_root: &str) -> Result<()> {
        // Literals injected into `v1.meta`; single-quote-escape defensively.
        let ctx_version_lit = ctx_version.replace('\'', "''");
        let index_root_lit = index_root.replace('\'', "''");

        self.conn.execute_batch(&format!(
            r#"
            CREATE SCHEMA v1;

            CREATE VIEW v1.symbols AS
            WITH fan_out_counts AS (
                SELECT source_id AS id, COUNT(*) AS fan_out
                FROM code.edges
                WHERE kind = 'calls'
                GROUP BY source_id
            ),
            fan_in_counts AS (
                SELECT target_id AS id, COUNT(*) AS fan_in
                FROM code.edges
                WHERE kind = 'calls' AND target_id IS NOT NULL
                GROUP BY target_id
            )
            SELECT
                s.id,
                s.name,
                s.qualified_name,
                s.kind,
                s.file_path AS file,
                s.line_start,
                s.line_end,
                (s.visibility = 'public') AS is_public,
                (COALESCE(fo.fan_out, 0) * 2 + COALESCE(fi.fan_in, 0)) AS complexity,
                COALESCE(fi.fan_in, 0) AS fan_in,
                COALESCE(fo.fan_out, 0) AS fan_out,
                COALESCE(s.docstring, s.brief) AS doc
            FROM code.symbols s
            LEFT JOIN fan_out_counts fo ON fo.id = s.id
            LEFT JOIN fan_in_counts fi ON fi.id = s.id;

            CREATE VIEW v1.edges AS
            SELECT
                e.source_id,
                s.name AS source_name,
                s.file_path AS source_file,
                e.target_id,
                e.target_name,
                t.file_path AS target_file,
                e.kind,
                e.line
            FROM code.edges e
            JOIN code.symbols s ON e.source_id = s.id
            LEFT JOIN code.symbols t ON e.target_id = t.id;

            CREATE VIEW v1.files AS
            SELECT
                f.path,
                f.language,
                COALESCE(agg.symbol_count, 0) AS symbol_count,
                COALESCE(agg.total_complexity, 0) AS total_complexity,
                f.last_indexed AS indexed_at
            FROM code.files f
            LEFT JOIN (
                SELECT file, COUNT(*) AS symbol_count, SUM(complexity) AS total_complexity
                FROM v1.symbols
                GROUP BY file
            ) agg ON agg.file = f.path;

            CREATE VIEW v1.meta AS
            SELECT
                1 AS schema_version,
                '{ctx_version}' AS ctx_version,
                (SELECT MIN(last_indexed) FROM code.files) AS index_created_at,
                '{index_root}' AS index_root;
            "#,
            ctx_version = ctx_version_lit,
            index_root = index_root_lit,
        ))?;

        Ok(())
    }

    /// A handle that can interrupt an in-flight query from another thread
    /// (used to enforce `--timeout`). `InterruptHandle` is `Send + Sync`.
    pub fn interrupt_handle(&self) -> Arc<InterruptHandle> {
        self.conn.interrupt_handle()
    }

    /// Execute a non-final statement in a multi-statement submission.
    ///
    /// Returns `Ok(true)` if the statement produces a result set — the caller
    /// rejects that (only the final statement may return rows, per the one
    /// result-set rule). Statements that produce no result set (e.g.
    /// `CREATE TEMP TABLE …`, `SET …`) are executed for their side effects and
    /// return `Ok(false)`.
    ///
    /// (DuckDB only knows a statement's column count *after* execution, so the
    /// statement is always run — harmless here, since the index is read-only
    /// and the in-memory DuckDB is throwaway.)
    pub fn exec_non_final_statement(&self, sql: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query([])?;
        let produced = rows
            .as_ref()
            .map(|s| {
                let n = s.column_count();
                if n == 0 {
                    return false;
                }
                // DuckDB reports DDL/DML — CREATE/INSERT/UPDATE/DELETE, including
                // `CREATE TABLE … AS SELECT` — as a single BIGINT column named
                // "Count" (rows affected). That is not a user-facing result set,
                // so such statements are allowed to precede the final one.
                !(n == 1 && s.column_name(0).map(|c| c == "Count").unwrap_or(false))
            })
            .unwrap_or(false);
        Ok(produced)
    }

    /// Run the final (result-producing) statement, streaming rows and stopping
    /// once `max_rows` is reached (`0` = unlimited). One extra row is fetched to
    /// detect truncation, so the full result is never materialized in Rust.
    pub fn run_final_statement(&self, sql: &str, max_rows: usize) -> Result<SqlResult> {
        let mut stmt = self.conn.prepare(sql)?;
        let mut rows = stmt.query([])?;

        // Column metadata is only available after the statement has executed.
        let columns: Vec<SqlColumn> = {
            let executed = rows.as_ref().expect("statement is available after query()");
            let col_count = executed.column_count();
            let names = executed.column_names();
            (0..col_count)
                .map(|i| SqlColumn {
                    name: names.get(i).cloned().unwrap_or_default(),
                    type_name: duckdb_type_name(&executed.column_type(i)),
                })
                .collect()
        };
        let col_count = columns.len();

        let cap = if max_rows == 0 { usize::MAX } else { max_rows };
        let mut rows_out: Vec<Vec<serde_json::Value>> = Vec::new();
        let mut truncated = false;

        while let Some(row) = rows.next()? {
            if rows_out.len() >= cap {
                truncated = true;
                break;
            }
            let mut record = Vec::with_capacity(col_count);
            for i in 0..col_count {
                record.push(value_ref_to_json(row.get_ref(i)?));
            }
            rows_out.push(record);
        }

        Ok(SqlResult {
            columns,
            rows: rows_out,
            truncated,
        })
    }
}

/// A column in a `ctx sql` result: its name and DuckDB type name.
#[derive(Debug, Clone)]
pub struct SqlColumn {
    pub name: String,
    pub type_name: String,
}

/// The result of a `ctx sql` query: columns plus row-capped data.
#[derive(Debug, Clone)]
pub struct SqlResult {
    pub columns: Vec<SqlColumn>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub truncated: bool,
}

/// Map an Arrow `DataType` (what duckdb-rs reports for a column via
/// `column_type`) to a DuckDB type name for the JSON envelope (e.g. `VARCHAR`,
/// `BIGINT`).
fn duckdb_type_name(dt: &duckdb::arrow::datatypes::DataType) -> String {
    use duckdb::arrow::datatypes::DataType as D;
    match dt {
        D::Null => "NULL",
        D::Boolean => "BOOLEAN",
        D::Int8 => "TINYINT",
        D::Int16 => "SMALLINT",
        D::Int32 => "INTEGER",
        D::Int64 => "BIGINT",
        D::UInt8 => "UTINYINT",
        D::UInt16 => "USMALLINT",
        D::UInt32 => "UINTEGER",
        D::UInt64 => "UBIGINT",
        D::Float16 | D::Float32 => "FLOAT",
        D::Float64 => "DOUBLE",
        D::Utf8 | D::LargeUtf8 | D::Utf8View => "VARCHAR",
        D::Binary | D::LargeBinary | D::BinaryView => "BLOB",
        D::Date32 | D::Date64 => "DATE",
        D::Timestamp(_, _) => "TIMESTAMP",
        D::Time32(_) | D::Time64(_) => "TIME",
        D::Decimal128(_, _) | D::Decimal256(_, _) => "DECIMAL",
        other => return format!("{:?}", other),
    }
    .to_string()
}

/// Convert a DuckDB `ValueRef` into a `serde_json::Value` for output. Exotic
/// types (temporal, decimal, nested) fall back to a debug string so rendering
/// never panics.
fn value_ref_to_json(v: ValueRef<'_>) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        ValueRef::Null => J::Null,
        ValueRef::Boolean(b) => J::Bool(b),
        ValueRef::TinyInt(n) => J::from(n),
        ValueRef::SmallInt(n) => J::from(n),
        ValueRef::Int(n) => J::from(n),
        ValueRef::BigInt(n) => J::from(n),
        ValueRef::UTinyInt(n) => J::from(n),
        ValueRef::USmallInt(n) => J::from(n),
        ValueRef::UInt(n) => J::from(n),
        ValueRef::UBigInt(n) => J::from(n),
        ValueRef::HugeInt(n) => J::String(n.to_string()),
        ValueRef::Float(f) => serde_json::Number::from_f64(f as f64)
            .map(J::Number)
            .unwrap_or(J::Null),
        ValueRef::Double(f) => serde_json::Number::from_f64(f)
            .map(J::Number)
            .unwrap_or(J::Null),
        ValueRef::Text(bytes) => J::String(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => J::String(format!("<blob: {} bytes>", bytes.len())),
        ValueRef::Timestamp(unit, raw) => {
            use duckdb::types::TimeUnit;
            let nanos = match unit {
                TimeUnit::Second => (raw as i128) * 1_000_000_000,
                TimeUnit::Millisecond => (raw as i128) * 1_000_000,
                TimeUnit::Microsecond => (raw as i128) * 1_000,
                TimeUnit::Nanosecond => raw as i128,
            };
            let formatted = time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
                .ok()
                .and_then(|dt| {
                    dt.format(&time::format_description::well_known::Rfc3339)
                        .ok()
                });
            J::String(formatted.unwrap_or_else(|| format!("Timestamp({unit:?}, {raw})")))
        }
        other => J::String(format!("{:?}", other)),
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
                line_start INTEGER,
                line_end INTEGER,
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
    fn test_located_impact_preserves_locations_and_deduplicates_by_symbol_id() {
        let conn = setup_test_db().expect("Failed to setup test db");
        conn.execute_batch(
            r#"
            INSERT INTO code.symbols VALUES
                ('other.rs::run@40', 'run', 'Worker::run', 'function', 'other.rs', 40, 52, 'private'),
                ('test.rs::run@40', 'run', 'Duplicate::run', 'function', 'test.rs', 40, 52, 'private');
            INSERT INTO code.edges VALUES
                ('other.rs::run@40', 'helper@21', 'helper', 'calls', NULL),
                ('other.rs::run@40', 'helper@21', 'helper', 'calls', NULL),
                ('test.rs::run@40', 'helper@21', 'helper', 'calls', NULL);
            "#,
        )
        .unwrap();
        let analytics = Analytics { conn };

        let nodes = analytics.impact_analysis_located("helper", 1).unwrap();

        assert_eq!(nodes.len(), 3);
        let located = nodes
            .iter()
            .find(|node| node.symbol_id == "other.rs::run@40")
            .unwrap();
        assert_eq!(located.name, "run");
        assert_eq!(located.qualified_name.as_deref(), Some("Worker::run"));
        assert_eq!(located.file_path, "other.rs");
        assert_eq!(located.line_start, 40);
        assert_eq!(located.line_end, 52);
        assert_eq!(located.distance, 1);

        let legacy = analytics.impact_analysis("helper", 1).unwrap();
        assert_eq!(legacy.len(), 2);
        assert_eq!(
            legacy
                .iter()
                .filter(|node| node.name == "run" && node.file_path == "test.rs")
                .count(),
            1
        );
    }

    #[test]
    fn test_located_impact_depth_preserves_locations() {
        let conn = setup_test_db().expect("Failed to setup test db");
        let analytics = Analytics { conn };

        let depth_one = analytics.impact_analysis_located("helper", 1).unwrap();
        assert_eq!(depth_one.len(), 1);
        assert_eq!(depth_one[0].symbol_id, "run@11");
        assert_eq!((depth_one[0].line_start, depth_one[0].line_end), (11, 20));
        assert_eq!(depth_one[0].distance, 1);

        let depth_two = analytics.impact_analysis_located("helper", 2).unwrap();
        assert_eq!(depth_two.len(), 2);
        let run = depth_two
            .iter()
            .find(|node| node.symbol_id == "run@11")
            .unwrap();
        assert_eq!((run.line_start, run.line_end), (11, 20));
        assert_eq!(run.distance, 1);
        let main = depth_two
            .iter()
            .find(|node| node.symbol_id == "main@1")
            .unwrap();
        assert_eq!((main.line_start, main.line_end), (1, 10));
        assert_eq!(main.distance, 2);
    }

    #[test]
    fn test_located_impact_missing_legacy_lines_fall_back_to_zero() {
        let conn = setup_test_db().expect("Failed to setup test db");
        conn.execute_batch(
            r#"
            INSERT INTO code.symbols VALUES
                ('legacy@31', 'legacy_caller', NULL, 'function', 'legacy.rs', NULL, NULL, 'private');
            INSERT INTO code.edges VALUES
                ('legacy@31', 'helper@21', 'helper', 'calls', NULL);
            "#,
        )
        .unwrap();
        let analytics = Analytics { conn };

        let nodes = analytics.impact_analysis_located("helper", 1).unwrap();
        let legacy = nodes
            .iter()
            .find(|node| node.symbol_id == "legacy@31")
            .unwrap();
        assert_eq!(legacy.line_start, 0);
        assert_eq!(legacy.line_end, 0);
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
