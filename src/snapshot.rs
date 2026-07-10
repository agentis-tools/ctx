//! Per-commit metric snapshots (`ctx snapshot`).
//!
//! Exports one Parquet partition per commit (`.ctx/snapshots/sha=<sha>/`)
//! with per-file and per-symbol metrics, near-duplicate pairs, and metadata,
//! for longitudinal quality analysis via `ctx sql --snapshots`.
//!
//! Each partition contains four files — `symbols.parquet`, `files.parquet`,
//! `dup_pairs.parquet`, and `meta.parquet` — every row denormalized with
//! `commit_sha` and `committed_at` so partitions can be unioned with a single
//! `read_parquet('.ctx/snapshots/*/*.parquet')` glob per table.
//!
//! Partitions are written to a `sha=<sha>.tmp` staging directory and moved
//! into place with an atomic rename, so readers never observe a half-written
//! snapshot. The capture core requires the `duckdb` feature; option and
//! report types are available in every build so the CLI surface stays stable.

use std::path::PathBuf;

use serde::Serialize;

/// Version of the snapshot Parquet schema, recorded in `meta.parquet`.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Default output directory for snapshot partitions, relative to the
/// project root.
pub const SNAPSHOTS_DIR: &str = ".ctx/snapshots";

/// How a snapshot was captured, recorded as `capture_mode` in `meta.parquet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Captured from the working tree at HEAD (`ctx snapshot`). Churn is
    /// measured up to now.
    Live,
    /// Captured from a historical commit checked out in a temporary worktree
    /// (`ctx snapshot backfill`). Churn is anchored with
    /// `--until=<commit date>` so it reflects that commit's point in time.
    Backfill,
}

impl CaptureMode {
    /// The string recorded in `meta.parquet` (`"live"` / `"backfill"`).
    pub fn as_str(self) -> &'static str {
        match self {
            CaptureMode::Live => "live",
            CaptureMode::Backfill => "backfill",
        }
    }
}

/// Options for a single snapshot capture.
#[derive(Debug, Clone)]
pub struct CaptureOptions {
    /// Directory that holds the `sha=<sha>/` partitions.
    pub out_dir: PathBuf,
    /// Churn lower bound (a `git log --since` date spec, e.g. `"90 days
    /// ago"`). Relative specs are resolved against wall-clock now by git,
    /// even in backfill mode — only the *upper* bound is anchored to the
    /// commit date there.
    pub churn_window: String,
    /// Live (HEAD + working tree) or backfill (historical worktree).
    pub capture_mode: CaptureMode,
    /// Overwrite an existing partition for the same commit.
    pub force: bool,
}

/// The result of one snapshot capture.
///
/// When `skipped_existing` is true the partition was left untouched and the
/// row counts (`files`, `symbols`, `dup_pairs`, `violations`) are reported
/// as zero — they are not re-read from the existing Parquet files.
#[derive(Debug, Clone, Serialize)]
pub struct SnapshotReport {
    /// Full sha of the snapshotted commit.
    pub commit_sha: String,
    /// Committer date of the commit (strict ISO 8601, as `git log --format=%cI`).
    pub committed_at: String,
    /// The partition directory the snapshot lives in.
    pub partition_dir: String,
    /// Rows written to `files.parquet`.
    pub files: usize,
    /// Rows written to `symbols.parquet`.
    pub symbols: usize,
    /// Rows written to `dup_pairs.parquet`.
    pub dup_pairs: usize,
    /// Total architecture-rule violations (0 when `.ctx/rules.toml` is absent).
    pub violations: i64,
    /// True when the partition already existed and was not rewritten.
    pub skipped_existing: bool,
}

/// Options for `ctx snapshot backfill`.
#[derive(Debug, Clone)]
pub struct BackfillOptions {
    /// Starting commit/ref. The walk covers the first-parent range
    /// `since..HEAD` **plus `since` itself** when it resolves to a commit,
    /// so `--since <first sha>` snapshots that commit too.
    pub since: String,
    /// Sample every Nth commit (1 = every commit). Sampling counts backwards
    /// from HEAD so the newest commit is always included.
    pub every: usize,
    /// Churn lower bound, passed through to [`CaptureOptions::churn_window`].
    pub churn_window: String,
    /// Snapshot directory of the *original* repository (partitions for
    /// historical commits land there, not inside the temporary worktrees).
    pub out_dir: PathBuf,
}

/// The partition directory name for a commit (`sha=<sha>`).
pub fn partition_dir_name(sha: &str) -> String {
    format!("sha={}", sha)
}

#[cfg(feature = "duckdb")]
pub use engine::{backfill, capture};

#[cfg(feature = "duckdb")]
mod engine {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    use super::*;
    use crate::analytics::Analytics;
    use crate::check;
    use crate::error::{CtxError, Result};
    use crate::fingerprint::{self, DuplicatePair};
    use crate::gitutil;
    use crate::index::Indexer;
    use crate::rules;
    use crate::score::{DUP_MIN_TOKENS, DUP_THRESHOLD};
    use crate::walker::WalkerConfig;

    /// Capture a snapshot of the repository at `root` (labeled with HEAD's
    /// sha) into `opts.out_dir/sha=<sha>/`.
    ///
    /// The index is refreshed incrementally first (same as `ctx score`), so
    /// the snapshot reflects the working tree — for a live capture of a
    /// dirty tree the caller should warn that the label and the content can
    /// disagree.
    pub fn capture(root: &Path, opts: &CaptureOptions) -> Result<SnapshotReport> {
        if !gitutil::is_git_repo_in(root) {
            return Err(CtxError::NotGitRepo);
        }
        let (sha, committed_at) = gitutil::head_commit_in(root)?;

        let partition = opts.out_dir.join(partition_dir_name(&sha));
        if partition.exists() && !opts.force {
            return Ok(skipped_report(&sha, &committed_at, &partition));
        }

        // Incremental index refresh: only changed files are re-parsed; the
        // existing database is never cleared (mirrors score::compute_score).
        let mut indexer = Indexer::with_config(root, false, WalkerConfig::default())?;
        indexer.index()?;
        let db = indexer.db;

        // Rust-side metrics: near-duplicates, rule violations, git churn.
        let pairs = fingerprint::find_near_duplicates(&db, DUP_THRESHOLD, DUP_MIN_TOKENS, None)?;
        let (violations_total, violations_by_file) = violation_counts(root)?;
        let until = match opts.capture_mode {
            CaptureMode::Backfill => Some(committed_at.as_str()),
            CaptureMode::Live => None,
        };
        let churn = gitutil::churn_between_in(root, &opts.churn_window, until)?;

        // Trusted, unhardened DuckDB connection: attaches the index and the
        // v1 views, then COPYs to Parquet. Never fed user SQL.
        let analytics = Analytics::open_export(root)?;
        let conn = analytics.connection();
        load_side_tables(conn, &churn, &violations_by_file, &pairs)?;

        // Stage into `sha=<sha>.tmp`, then atomically rename into place.
        let staging = opts
            .out_dir
            .join(format!("{}.tmp", partition_dir_name(&sha)));
        let guard = StagingGuard::new(&staging)?;
        write_parquet_files(conn, &staging, &sha, &committed_at, opts.capture_mode)?;

        if partition.exists() {
            // Only reachable with --force; replace the old partition.
            fs::remove_dir_all(&partition)?;
        }
        fs::rename(&staging, &partition)?;
        guard.defuse();

        let count = |sql: &str| -> Result<usize> {
            let n: i64 = conn.query_row(sql, [], |row| row.get(0))?;
            Ok(n.max(0) as usize)
        };
        Ok(SnapshotReport {
            commit_sha: sha,
            committed_at,
            partition_dir: partition.display().to_string(),
            files: count("SELECT COUNT(*) FROM v1.files")?,
            symbols: count("SELECT COUNT(*) FROM v1.symbols")?,
            dup_pairs: pairs.len(),
            violations: violations_total,
            skipped_existing: false,
        })
    }

    /// Backfill snapshots for historical commits.
    ///
    /// Walks the first-parent range `since..HEAD` oldest-first, **including
    /// `since` itself** when it resolves to a commit (so `--since <sha>`
    /// covers that commit). After `--every N` sampling (newest always kept),
    /// each missing commit is checked out into a temporary `git worktree`,
    /// captured with [`CaptureMode::Backfill`] into the original repo's
    /// snapshot directory, and the worktree is removed again (an RAII guard
    /// cleans up even on panics or per-commit errors).
    ///
    /// Per-commit failures are logged to stderr and skipped; the returned
    /// reports cover the captured and already-existing partitions only.
    pub fn backfill(root: &Path, opts: &BackfillOptions) -> Result<Vec<SnapshotReport>> {
        if !gitutil::is_git_repo_in(root) {
            return Err(CtxError::NotGitRepo);
        }

        let mut shas = gitutil::rev_list_first_parent_in(root, &format!("{}..HEAD", opts.since))?;
        if let Some(since_sha) = resolve_commit(root, &opts.since) {
            if !shas.contains(&since_sha) {
                shas.insert(0, since_sha);
            }
        }
        let sampled = sample_every(shas, opts.every.max(1));

        // Parent directory for the temporary worktrees.
        let worktrees_root =
            std::env::temp_dir().join(format!("ctx-backfill-{}", std::process::id()));
        fs::create_dir_all(&worktrees_root)?;

        let total = sampled.len();
        let mut reports = Vec::new();
        let mut failed = 0usize;
        for (i, sha) in sampled.iter().enumerate() {
            let short = &sha[..12.min(sha.len())];
            let partition = opts.out_dir.join(partition_dir_name(sha));
            if partition.exists() {
                eprintln!(
                    "[{}/{}] {}: partition exists, skipping",
                    i + 1,
                    total,
                    short
                );
                let committed_at = commit_date(root, sha).unwrap_or_default();
                reports.push(skipped_report(sha, &committed_at, &partition));
                continue;
            }

            eprintln!("[{}/{}] {}: capturing…", i + 1, total, short);
            match capture_commit(root, sha, &worktrees_root, opts) {
                Ok(report) => reports.push(report),
                Err(e) => {
                    failed += 1;
                    eprintln!("[{}/{}] {}: failed: {}", i + 1, total, short, e);
                }
            }
        }
        // Best-effort: the per-sha guards removed their worktrees already.
        let _ = fs::remove_dir(&worktrees_root);

        if failed > 0 {
            eprintln!(
                "backfill: {} succeeded, {} failed of {} commits",
                reports.len(),
                failed,
                total
            );
        }
        Ok(reports)
    }

    /// Capture one historical commit via a temporary detached worktree.
    fn capture_commit(
        root: &Path,
        sha: &str,
        worktrees_root: &Path,
        opts: &BackfillOptions,
    ) -> Result<SnapshotReport> {
        let wt_path = worktrees_root.join(sha);
        let output = Command::new("git")
            .args(["worktree", "add", "--detach"])
            .arg(&wt_path)
            .arg(sha)
            .current_dir(root)
            .output()?;
        if !output.status.success() {
            return Err(CtxError::git(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        let _guard = WorktreeGuard {
            repo: root.to_path_buf(),
            path: wt_path.clone(),
        };

        capture(
            &wt_path,
            &CaptureOptions {
                out_dir: opts.out_dir.clone(),
                churn_window: opts.churn_window.clone(),
                capture_mode: CaptureMode::Backfill,
                force: false,
            },
        )
    }

    // ========================================================================
    // Parquet export
    // ========================================================================

    /// Load churn, per-file violation counts, and duplicate pairs into
    /// side tables in the in-memory DuckDB so the COPY queries can join them.
    fn load_side_tables(
        conn: &duckdb::Connection,
        churn: &HashMap<String, u32>,
        violations_by_file: &HashMap<String, i64>,
        pairs: &[DuplicatePair],
    ) -> Result<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE churn (path VARCHAR, churn_commits INTEGER);
            CREATE TABLE violations (path VARCHAR, violation_count INTEGER);
            CREATE TABLE dup_pairs (
                file_a VARCHAR,
                symbol_a VARCHAR,
                file_b VARCHAR,
                symbol_b VARCHAR,
                similarity DOUBLE,
                token_count_a BIGINT,
                token_count_b BIGINT
            );
            "#,
        )?;

        let mut app = conn.appender("churn")?;
        for (path, commits) in churn {
            app.append_row(duckdb::params![path, *commits])?;
        }
        app.flush()?;

        let mut app = conn.appender("violations")?;
        for (path, count) in violations_by_file {
            app.append_row(duckdb::params![path, *count])?;
        }
        app.flush()?;

        let mut app = conn.appender("dup_pairs")?;
        for pair in pairs {
            app.append_row(duckdb::params![
                pair.a.file_path,
                pair.a.name,
                pair.b.file_path,
                pair.b.name,
                pair.similarity,
                pair.token_count_a,
                pair.token_count_b,
            ])?;
        }
        app.flush()?;

        Ok(())
    }

    /// COPY the four snapshot tables into `staging` as Parquet, every row
    /// denormalized with `commit_sha` and `committed_at`.
    fn write_parquet_files(
        conn: &duckdb::Connection,
        staging: &Path,
        sha: &str,
        committed_at: &str,
        mode: CaptureMode,
    ) -> Result<()> {
        // `committed_at` becomes a Parquet TIMESTAMP; normalize the ISO 8601
        // committer date (which carries a UTC offset) to a naive UTC literal.
        let committed_utc = to_utc_timestamp_literal(committed_at)?;
        let captured_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default();

        // Columns shared by every row of every table. `sha` is git-produced
        // hex and the timestamps are generated above, but escape defensively
        // like the rest of this module does for paths.
        let stamp = format!(
            "'{}' AS commit_sha, TIMESTAMP '{}' AS committed_at",
            sql_escape(sha),
            sql_escape(&committed_utc)
        );

        copy_to_parquet(
            conn,
            &format!(
                "SELECT {stamp},
                        id, name, qualified_name, kind, file,
                        line_start, line_end, is_public,
                        complexity, fan_in, fan_out
                 FROM v1.symbols"
            ),
            &staging.join("symbols.parquet"),
        )?;

        copy_to_parquet(
            conn,
            &format!(
                "SELECT {stamp},
                        f.path, f.language, f.symbol_count, f.total_complexity,
                        COALESCE(mx.max_complexity, 0) AS max_complexity,
                        COALESCE(c.churn_commits, 0) AS churn_commits,
                        COALESCE(v.violation_count, 0) AS violation_count
                 FROM v1.files f
                 LEFT JOIN (
                     SELECT file, MAX(complexity) AS max_complexity
                     FROM v1.symbols GROUP BY file
                 ) mx ON mx.file = f.path
                 LEFT JOIN churn c ON c.path = f.path
                 LEFT JOIN violations v ON v.path = f.path"
            ),
            &staging.join("files.parquet"),
        )?;

        copy_to_parquet(
            conn,
            &format!(
                "SELECT {stamp},
                        file_a, symbol_a, file_b, symbol_b,
                        similarity, token_count_a, token_count_b
                 FROM dup_pairs"
            ),
            &staging.join("dup_pairs.parquet"),
        )?;

        copy_to_parquet(
            conn,
            &format!(
                "SELECT {stamp},
                        '{captured_at}' AS captured_at,
                        '{version}' AS ctx_version,
                        CAST({schema} AS INTEGER) AS snapshot_schema_version,
                        '{mode}' AS capture_mode",
                captured_at = sql_escape(&captured_at),
                version = sql_escape(env!("CARGO_PKG_VERSION")),
                schema = SNAPSHOT_SCHEMA_VERSION,
                mode = mode.as_str(),
            ),
            &staging.join("meta.parquet"),
        )?;

        Ok(())
    }

    /// Run `COPY (<select>) TO '<path>' (FORMAT PARQUET)`.
    fn copy_to_parquet(conn: &duckdb::Connection, select: &str, path: &Path) -> Result<()> {
        let escaped = sql_escape(&path.display().to_string());
        conn.execute_batch(&format!("COPY ({select}) TO '{escaped}' (FORMAT PARQUET);"))?;
        Ok(())
    }

    /// Single-quote-escape a string for embedding in a DuckDB SQL literal.
    fn sql_escape(s: &str) -> String {
        s.replace('\'', "''")
    }

    /// Convert a strict ISO 8601 date with offset (git's `%cI`) into a naive
    /// UTC `YYYY-MM-DD HH:MM:SS` literal for a DuckDB TIMESTAMP.
    fn to_utc_timestamp_literal(iso: &str) -> Result<String> {
        let parsed = OffsetDateTime::parse(iso, &Rfc3339)
            .map_err(|e| CtxError::Other(format!("invalid commit date {:?}: {}", iso, e)))?;
        let utc = parsed.to_offset(time::UtcOffset::UTC);
        Ok(format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            utc.year(),
            u8::from(utc.month()),
            utc.day(),
            utc.hour(),
            utc.minute(),
            utc.second()
        ))
    }

    // ========================================================================
    // Metrics helpers
    // ========================================================================

    /// Full-repo rule violations: `(total, per-file counts)`. Absent rules
    /// file -> zero counts (same convention as `ctx score`).
    fn violation_counts(root: &Path) -> Result<(i64, HashMap<String, i64>)> {
        let rules_path = root.join(rules::DEFAULT_RULES_PATH);
        if !rules_path.exists() {
            return Ok((0, HashMap::new()));
        }
        let context = check::load_context(root, None)?;
        let violations = check::collect_violations(root, &context, None)?;
        let mut by_file: HashMap<String, i64> = HashMap::new();
        for violation in &violations {
            *by_file.entry(violation.file.clone()).or_insert(0) += 1;
        }
        Ok((violations.len() as i64, by_file))
    }

    /// A report for a partition that already existed (row counts zeroed;
    /// see [`SnapshotReport::skipped_existing`]).
    fn skipped_report(sha: &str, committed_at: &str, partition: &Path) -> SnapshotReport {
        SnapshotReport {
            commit_sha: sha.to_string(),
            committed_at: committed_at.to_string(),
            partition_dir: partition.display().to_string(),
            files: 0,
            symbols: 0,
            dup_pairs: 0,
            violations: 0,
            skipped_existing: true,
        }
    }

    // ========================================================================
    // Git plumbing local to backfill
    // ========================================================================

    /// Run git in `root` and return its trimmed stdout when the command
    /// succeeds with non-empty output; `None` otherwise.
    fn git_trimmed(root: &Path, args: &[&str]) -> Option<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (!out.is_empty()).then_some(out)
    }

    /// Resolve `rev` to a full commit sha, or `None` if it doesn't resolve.
    fn resolve_commit(root: &Path, rev: &str) -> Option<String> {
        let spec = format!("{}^{{commit}}", rev);
        git_trimmed(root, &["rev-parse", "--verify", "--quiet", &spec])
    }

    /// Committer date (`%cI`) of a commit, for skipped-partition reports.
    fn commit_date(root: &Path, sha: &str) -> Option<String> {
        git_trimmed(root, &["log", "-1", "--format=%cI", sha])
    }

    /// Keep every Nth sha counting backwards from the newest (the last
    /// element), so HEAD-most commits are always included. Input and output
    /// are oldest-first.
    fn sample_every(shas: Vec<String>, every: usize) -> Vec<String> {
        let n = shas.len();
        shas.into_iter()
            .enumerate()
            .filter(|(i, _)| (n - 1 - i).is_multiple_of(every))
            .map(|(_, sha)| sha)
            .collect()
    }

    // ========================================================================
    // RAII guards
    // ========================================================================

    /// Removes the staging directory on drop unless defused (i.e. on any
    /// error path between creation and the atomic rename).
    struct StagingGuard {
        path: PathBuf,
        defused: bool,
    }

    impl StagingGuard {
        fn new(path: &Path) -> Result<StagingGuard> {
            // A stale .tmp dir from a crashed run is safe to replace.
            if path.exists() {
                fs::remove_dir_all(path)?;
            }
            fs::create_dir_all(path)?;
            Ok(StagingGuard {
                path: path.to_path_buf(),
                defused: false,
            })
        }

        fn defuse(mut self) {
            self.defused = true;
        }
    }

    impl Drop for StagingGuard {
        fn drop(&mut self) {
            if !self.defused {
                let _ = fs::remove_dir_all(&self.path);
            }
        }
    }

    /// Removes a temporary backfill worktree on drop (including panics and
    /// per-commit error paths), so failed backfills never leak worktrees.
    struct WorktreeGuard {
        repo: PathBuf,
        path: PathBuf,
    }

    impl Drop for WorktreeGuard {
        fn drop(&mut self) {
            let _ = Command::new("git")
                .args(["worktree", "remove", "--force"])
                .arg(&self.path)
                .current_dir(&self.repo)
                .output();
            if self.path.exists() {
                // `--force` can still refuse (e.g. nested untracked dirs on
                // some git versions); fall back to rm + prune.
                let _ = fs::remove_dir_all(&self.path);
                let _ = Command::new("git")
                    .args(["worktree", "prune"])
                    .current_dir(&self.repo)
                    .output();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::testutil::GitRepo;
        use tempfile::TempDir;

        /// Permanent regression test for the Step-0 spike: the bundled
        /// DuckDB build must support `COPY ... (FORMAT PARQUET)` and
        /// `read_parquet` on an unhardened in-memory connection.
        #[test]
        fn parquet_copy_round_trip() {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("t.parquet");
            let escaped = sql_escape(&path.display().to_string());

            let conn = duckdb::Connection::open_in_memory().unwrap();
            conn.execute_batch(&format!(
                "COPY (SELECT 1 AS x) TO '{}' (FORMAT PARQUET);",
                escaped
            ))
            .unwrap();
            assert!(path.exists(), "COPY must write the parquet file");

            let x: i64 = conn
                .query_row(
                    &format!("SELECT x FROM read_parquet('{}')", escaped),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(x, 1);
        }

        const MAIN_RS: &str = r#"
pub fn alpha() -> i64 {
    beta()
}

pub fn beta() -> i64 {
    1
}
"#;

        fn snapshot_fixture() -> (TempDir, GitRepo) {
            let temp = TempDir::new().unwrap();
            let repo = GitRepo::init(temp.path());
            repo.write("src/a.rs", MAIN_RS);
            repo.commit_all_with_date("v1", "2024-03-04T05:06:07 +0200");
            let mut indexer =
                Indexer::with_config(&repo.root, false, WalkerConfig::default()).unwrap();
            indexer.index().unwrap();
            (temp, repo)
        }

        /// End-to-end round trip: capture a partition, then read every
        /// Parquet file back through an unhardened in-memory connection.
        #[test]
        fn capture_writes_readable_partition() {
            let (_temp, repo) = snapshot_fixture();
            let out_dir = repo.root.join(SNAPSHOTS_DIR);

            let report = capture(
                &repo.root,
                &CaptureOptions {
                    out_dir: out_dir.clone(),
                    churn_window: "90 days ago".to_string(),
                    capture_mode: CaptureMode::Live,
                    force: false,
                },
            )
            .unwrap();

            assert!(!report.skipped_existing);
            assert!(report.symbols >= 2, "symbols: {}", report.symbols);
            assert!(report.files >= 1, "files: {}", report.files);

            let partition = out_dir.join(partition_dir_name(&report.commit_sha));
            assert_eq!(report.partition_dir, partition.display().to_string());
            let conn = duckdb::Connection::open_in_memory().unwrap();
            for name in ["symbols", "files", "dup_pairs", "meta"] {
                let path = partition.join(format!("{}.parquet", name));
                assert!(path.exists(), "missing {}", path.display());
                let escaped = sql_escape(&path.display().to_string());
                let sha: String = conn
                    .query_row(
                        &format!("SELECT commit_sha FROM read_parquet('{}') LIMIT 1", escaped),
                        [],
                        |row| row.get(0),
                    )
                    .unwrap_or_default();
                if name != "dup_pairs" {
                    // dup_pairs may legitimately be empty for this fixture.
                    assert_eq!(sha, report.commit_sha, "table {}", name);
                }
            }

            // committed_at is a real TIMESTAMP normalized to UTC
            // (05:06:07 +0200 -> 03:06:07).
            let meta = partition.join("meta.parquet");
            let escaped = sql_escape(&meta.display().to_string());
            let (committed, mode, schema): (String, String, i32) = conn
                .query_row(
                    &format!(
                        "SELECT CAST(committed_at AS VARCHAR), capture_mode, \
                         snapshot_schema_version FROM read_parquet('{}')",
                        escaped
                    ),
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(committed, "2024-03-04 03:06:07");
            assert_eq!(mode, "live");
            assert_eq!(schema as u32, SNAPSHOT_SCHEMA_VERSION);

            // No staging leftovers.
            assert!(!out_dir
                .join(format!("{}.tmp", partition_dir_name(&report.commit_sha)))
                .exists());

            // Second capture skips; --force rewrites.
            let skipped = capture(
                &repo.root,
                &CaptureOptions {
                    out_dir: out_dir.clone(),
                    churn_window: "90 days ago".to_string(),
                    capture_mode: CaptureMode::Live,
                    force: false,
                },
            )
            .unwrap();
            assert!(skipped.skipped_existing);
            assert_eq!(skipped.symbols, 0, "skipped reports zero counts");

            let forced = capture(
                &repo.root,
                &CaptureOptions {
                    out_dir,
                    churn_window: "90 days ago".to_string(),
                    capture_mode: CaptureMode::Live,
                    force: true,
                },
            )
            .unwrap();
            assert!(!forced.skipped_existing);
            assert_eq!(forced.symbols, report.symbols);
        }

        #[test]
        fn sample_every_always_keeps_newest() {
            let shas: Vec<String> = (0..7).map(|i| format!("c{}", i)).collect();
            let sampled = sample_every(shas.clone(), 3);
            assert_eq!(sampled, vec!["c0", "c3", "c6"]);
            let sampled = sample_every(shas.clone(), 2);
            assert_eq!(sampled, vec!["c0", "c2", "c4", "c6"]);
            let sampled = sample_every(shas, 1);
            assert_eq!(sampled.len(), 7);
        }
    }
}
