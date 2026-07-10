//! Integration tests for the `ctx sql` subcommand.
//!
//! These run the real `ctx` binary against a real DuckDB-backed index built by
//! `ctx index`. Safety is enforced entirely by engine configuration, so the
//! security tests below assert that dangerous operations fail (exit code 2) and,
//! where relevant, that they leave the filesystem and index untouched.
//!
//! Assertions deliberately use substring matching (`predicates::str::contains`)
//! and exit-code checks rather than parsing JSON, to avoid depending on
//! `serde_json` from an integration-test target.

// The entire suite drives `ctx sql` against a DuckDB-backed index, so it is
// meaningless without the duckdb feature (Windows CI runs --no-default-features,
// where `ctx sql` exits 2 with a "requires the duckdb feature" error).
#![cfg(feature = "duckdb")]

use std::path::Path;

use assert_cmd::Command;
use ctx::testutil::GitRepo;
use predicates::prelude::*;
use tempfile::TempDir;

/// Build a minimal, real Rust project in a fresh `TempDir` and run `ctx index`
/// so the `v1.*` views are populated with symbols and `calls` edges.
///
/// The returned `TempDir` must be kept alive for the duration of a test; its
/// path is the working directory for every `ctx sql` invocation.
fn indexed_fixture() -> TempDir {
    let temp = TempDir::new().expect("create temp dir");
    let src = temp.path().join("src");
    std::fs::create_dir_all(&src).expect("create src dir");

    std::fs::write(
        src.join("main.rs"),
        r#"
struct Point {
    x: i32,
    y: i32,
}

enum Color {
    Red,
    Green,
    Blue,
}

fn helper(n: i32) -> i32 {
    n * 2
}

fn main() {
    let p = Point { x: 1, y: 2 };
    let _c = Color::Red;
    let _sum = helper(p.x) + helper(p.y);
}
"#,
    )
    .expect("write main.rs");

    std::fs::write(
        src.join("lib.rs"),
        r#"
pub fn public_api() -> u32 {
    private_impl() + 1
}

fn private_impl() -> u32 {
    7
}
"#,
    )
    .expect("write lib.rs");

    Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .arg("index")
        .assert()
        .success();

    temp
}

/// Convenience: a `ctx sql` command rooted in `dir`.
fn sql(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("ctx").unwrap();
    cmd.current_dir(dir).arg("sql");
    cmd
}

/// Path to the on-disk index for a fixture project.
fn index_db(dir: &Path) -> std::path::PathBuf {
    dir.join(".ctx").join("codebase.sqlite")
}

// ---------------------------------------------------------------------------
// Security: the engine must reject filesystem access, extension installs,
// configuration changes, and file-based ATTACH.
// ---------------------------------------------------------------------------

#[test]
fn copy_to_file_is_blocked_and_writes_nothing() {
    let temp = indexed_fixture();
    let leak = temp.path().join("leak.csv");
    let query = format!("COPY (SELECT 1) TO '{}'", leak.display());

    sql(temp.path()).arg(&query).assert().code(2);

    assert!(
        !leak.exists(),
        "COPY must not create a file on disk: {}",
        leak.display()
    );
}

#[test]
fn read_csv_from_filesystem_is_blocked() {
    let temp = indexed_fixture();
    sql(temp.path())
        .arg("SELECT * FROM read_csv('/etc/passwd')")
        .assert()
        .code(2);
}

#[test]
fn install_extension_is_blocked() {
    let temp = indexed_fixture();
    sql(temp.path()).arg("INSTALL httpfs").assert().code(2);
}

#[test]
fn changing_locked_configuration_is_blocked() {
    let temp = indexed_fixture();
    sql(temp.path())
        .arg("SET enable_external_access = true")
        .assert()
        .code(2);
}

#[test]
fn attach_file_database_is_blocked() {
    let temp = indexed_fixture();
    // A file-based ATTACH must be rejected. (`:memory:` is intentionally
    // permitted-but-inert and is deliberately NOT tested here.)
    let evil = temp.path().join("ctx_evil_attach.db");
    let query = format!("ATTACH '{}' AS x", evil.display());
    sql(temp.path()).arg(&query).assert().code(2);
    assert!(
        !evil.exists(),
        "file-based ATTACH must not create a database file"
    );
}

#[test]
fn updates_are_rejected_and_index_bytes_are_unchanged() {
    let temp = indexed_fixture();
    let db = index_db(temp.path());

    let before = std::fs::read(&db).expect("read index before");

    // Both the public view layer and the underlying read-only `code` database
    // must reject writes.
    for stmt in [
        "UPDATE v1.symbols SET name='x'",
        "UPDATE code.symbols SET name='x'",
    ] {
        sql(temp.path()).arg(stmt).assert().code(2);
    }

    let after = std::fs::read(&db).expect("read index after");
    assert!(
        before == after,
        "the on-disk index must be byte-for-byte unchanged after rejected UPDATEs"
    );
}

#[test]
fn runaway_query_is_interrupted_by_timeout() {
    let temp = indexed_fixture();
    let start = std::time::Instant::now();
    sql(temp.path())
        .arg("--timeout")
        .arg("2")
        .arg("WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM r) SELECT count(*) FROM r")
        .assert()
        .code(2);
    // Generous upper bound so the test is not flaky under load, but still proves
    // the query did not run unbounded.
    assert!(
        start.elapsed().as_secs() < 30,
        "timeout should abort the query promptly"
    );
}

// ---------------------------------------------------------------------------
// Schema: the versioned `v1` views are queryable and `v1.meta` reports the
// contract version and crate version.
// ---------------------------------------------------------------------------

#[test]
fn meta_reports_schema_and_crate_version() {
    let temp = indexed_fixture();
    sql(temp.path())
        .arg("--json")
        .arg("SELECT * FROM v1.meta")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"command\": \"sql\""))
        .stdout(predicate::str::contains("schema_version"))
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")))
        // schema_version is 1 (single-row meta).
        .stdout(predicate::str::contains("1"));
}

#[test]
fn symbol_analytics_columns_are_selectable() {
    let temp = indexed_fixture();
    sql(temp.path())
        .arg("SELECT name, complexity, fan_in, fan_out FROM v1.symbols LIMIT 1")
        .assert()
        .success();
}

#[test]
fn every_documented_view_is_queryable() {
    let temp = indexed_fixture();
    for view in ["v1.symbols", "v1.edges", "v1.files", "v1.meta"] {
        sql(temp.path())
            .arg(format!("SELECT * FROM {} LIMIT 0", view))
            .assert()
            .success();
    }
}

#[test]
fn schema_flag_prints_reference_without_index() {
    // `--schema` needs neither an index nor the engine.
    let temp = TempDir::new().unwrap();
    sql(temp.path())
        .arg("--schema")
        .assert()
        .success()
        .stdout(predicate::str::contains("v1.symbols"))
        .stdout(predicate::str::contains("Public Schema"));
}

// ---------------------------------------------------------------------------
// Execution: exit-code convention, row caps, JSON envelope, multi-statement.
// ---------------------------------------------------------------------------

#[test]
fn fail_on_rows_exit_codes() {
    let temp = indexed_fixture();

    // >= 1 row -> exit 1.
    sql(temp.path())
        .arg("--fail-on-rows")
        .arg("SELECT 1")
        .assert()
        .code(1);

    // zero rows -> exit 0.
    sql(temp.path())
        .arg("--fail-on-rows")
        .arg("SELECT 1 WHERE false")
        .assert()
        .code(0);

    // error -> exit 2.
    sql(temp.path())
        .arg("--fail-on-rows")
        .arg("SELCT bad syntax")
        .assert()
        .code(2);
}

#[test]
fn row_cap_truncates_in_json_and_warns_in_table() {
    let temp = indexed_fixture();

    // JSON: default cap is 1000 rows; the envelope reports the cap and truncation.
    sql(temp.path())
        .arg("--json")
        .arg("SELECT * FROM range(5000) t(n)")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"row_count\": 1000"))
        .stdout(predicate::str::contains("\"truncated\": true"));

    // Table (default): a cap notice mentioning --max-rows goes to stderr.
    sql(temp.path())
        .arg("SELECT * FROM range(5000) t(n)")
        .assert()
        .success()
        .stderr(predicate::str::contains("--max-rows"));
}

#[test]
fn json_output_is_a_well_formed_envelope() {
    let temp = indexed_fixture();
    sql(temp.path())
        .arg("--json")
        .arg("SELECT 1 AS one")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("{"))
        .stdout(predicate::str::contains("\"command\": \"sql\""))
        .stdout(predicate::str::contains("\"data\""));
}

#[test]
fn multi_statement_semantics() {
    let temp = indexed_fixture();

    // Leading non-result statement, then a final SELECT -> ok, value shown.
    sql(temp.path())
        .arg("CREATE TEMP TABLE t AS SELECT 42 AS x; SELECT * FROM t")
        .assert()
        .success()
        .stdout(predicate::str::contains("42"));

    // Two result-producing statements -> error (only the final may return rows).
    sql(temp.path()).arg("SELECT 1; SELECT 2").assert().code(2);
}

#[test]
fn missing_index_errors_and_creates_nothing() {
    // Fresh temp dir with NO `ctx index` run.
    let temp = TempDir::new().unwrap();
    sql(temp.path())
        .arg("SELECT 1")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("ctx index"));

    assert!(
        !temp.path().join(".ctx").exists(),
        "a failed `ctx sql` must not create a .ctx directory"
    );
}

// ---------------------------------------------------------------------------
// Integration: the gate pattern (`--fail-on-rows --file <gate>.sql`).
// ---------------------------------------------------------------------------

#[test]
fn gate_files_drive_pass_and_fail_exit_codes() {
    let temp = indexed_fixture();
    let gates = temp.path().join(".ctx").join("gates");
    std::fs::create_dir_all(&gates).expect("create gates dir");

    // A passing gate returns no rows.
    std::fs::write(
        gates.join("pass.sql"),
        "SELECT name FROM v1.symbols WHERE 1=0",
    )
    .unwrap();
    sql(temp.path())
        .arg("--fail-on-rows")
        .arg("--file")
        .arg(".ctx/gates/pass.sql")
        .assert()
        .code(0);

    // A failing gate returns at least one row.
    std::fs::write(
        gates.join("fail.sql"),
        "SELECT name FROM v1.symbols LIMIT 1",
    )
    .unwrap();
    sql(temp.path())
        .arg("--fail-on-rows")
        .arg("--file")
        .arg(".ctx/gates/fail.sql")
        .assert()
        .code(1);
}

// ---------------------------------------------------------------------------
// Snapshots: `--snapshots` loads Parquet partitions as `snap.*` tables for
// trend queries, without weakening the sandbox.
// ---------------------------------------------------------------------------

/// A function with > 50 normalized tokens (the snapshot near-duplicate
/// detector's minimum).
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

/// A structural copy of `DUPE_A` with renamed identifiers and different
/// literals, so every snapshot partition contains one near-duplicate pair.
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

/// A file with call edges so its `total_complexity` strictly exceeds the
/// duplicate file's — the hotspot-mass query's `percent_rank() >= 0.9`
/// bucket then contains this file for every commit.
const CALLS_V1: &str = r#"
pub fn alpha() -> i64 {
    beta() + gamma() + beta()
}

pub fn beta() -> i64 {
    1
}

pub fn gamma() -> i64 {
    beta() + 2
}
"#;

const CALLS_V2: &str = r#"
pub fn alpha() -> i64 {
    beta() + gamma() + beta() + delta()
}

pub fn beta() -> i64 {
    1
}

pub fn gamma() -> i64 {
    beta() + 2
}

pub fn delta() -> i64 {
    beta() + gamma()
}
"#;

/// The three canned trend queries documented in the schema reference
/// (`ctx sql --schema`). `canned_queries_match_schema_reference` keeps these
/// strings and the docs in lockstep.
const TREND_DUPLICATION: &str = "SELECT commit_sha, min(committed_at) AS committed_at, count(*) AS dup_pairs FROM snap.dup_pairs GROUP BY commit_sha ORDER BY committed_at;";
const TREND_VIOLATIONS: &str = "SELECT commit_sha, min(committed_at) AS committed_at, sum(violation_count) AS violations FROM snap.files GROUP BY commit_sha ORDER BY committed_at;";
const TREND_HOTSPOT_MASS: &str = "WITH ranked AS (SELECT commit_sha, committed_at, churn_commits * total_complexity AS mass, percent_rank() OVER (PARTITION BY commit_sha ORDER BY total_complexity) AS pr FROM snap.files) SELECT commit_sha, min(committed_at) AS committed_at, sum(mass) AS hotspot_mass FROM ranked WHERE pr >= 0.9 GROUP BY commit_sha ORDER BY committed_at;";

/// Run a git command in `dir` and return its trimmed stdout.
fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A repo with two dated commits, an index, and one snapshot partition per
/// commit (via `ctx snapshot backfill`). Returns the commit shas oldest-first.
fn snapshot_fixture() -> (TempDir, GitRepo, Vec<String>) {
    let temp = TempDir::new().expect("create temp dir");
    let repo = GitRepo::init(temp.path());
    repo.write("src/dupes.rs", &format!("{}\n{}", DUPE_A, DUPE_B));
    repo.write("src/calls.rs", CALLS_V1);
    repo.commit_all_with_date("one", "2024-01-01T12:00:00 +0000");
    repo.write("src/calls.rs", CALLS_V2);
    repo.commit_all_with_date("two", "2024-02-01T12:00:00 +0000");

    Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(&repo.root)
        .arg("index")
        .assert()
        .success();

    let shas: Vec<String> = git_stdout(&repo.root, &["rev-list", "--reverse", "HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(shas.len(), 2, "fixture has two commits");

    Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(&repo.root)
        .args(["snapshot", "backfill", "--since", &shas[0]])
        .assert()
        .success();

    (temp, repo, shas)
}

/// Run `ctx sql --snapshots --json <query>` in `dir`, assert success, and
/// return stdout.
fn snapshots_json(dir: &Path, query: &str) -> String {
    let assert = sql(dir)
        .arg("--snapshots")
        .arg("--json")
        .arg(query)
        .assert()
        .success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is utf-8")
}

/// One trend row per partition, ordered ascending by `committed_at`
/// (fixture commits are dated oldest-first, so the older sha must appear
/// first in the JSON `rows` array).
fn assert_trend_rows(stdout: &str, shas: &[String]) {
    assert!(
        stdout.contains("\"row_count\": 2"),
        "expected one row per partition, got:\n{}",
        stdout
    );
    let first = stdout
        .find(&shas[0])
        .unwrap_or_else(|| panic!("first commit {} missing from:\n{}", shas[0], stdout));
    let second = stdout
        .find(&shas[1])
        .unwrap_or_else(|| panic!("second commit {} missing from:\n{}", shas[1], stdout));
    assert!(
        first < second,
        "rows must be ordered ascending by committed_at (oldest commit first):\n{}",
        stdout
    );
}

/// The canned trend queries below must be exactly the ones documented in the
/// schema reference, so docs and tests cannot drift apart.
#[test]
fn canned_queries_match_schema_reference() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let schema =
        std::fs::read_to_string(Path::new(manifest).join("src/commands/sql_schema.md")).unwrap();
    for query in [TREND_DUPLICATION, TREND_VIOLATIONS, TREND_HOTSPOT_MASS] {
        assert!(
            schema.contains(query),
            "schema reference must contain the canned query:\n{}",
            query
        );
    }
}

#[test]
fn snapshots_duplication_trend() {
    let (_temp, repo, shas) = snapshot_fixture();
    let stdout = snapshots_json(&repo.root, TREND_DUPLICATION);
    assert_trend_rows(&stdout, &shas);
}

#[test]
fn snapshots_violation_trend() {
    let (_temp, repo, shas) = snapshot_fixture();
    let stdout = snapshots_json(&repo.root, TREND_VIOLATIONS);
    assert_trend_rows(&stdout, &shas);
}

#[test]
fn snapshots_hotspot_mass_trend() {
    let (_temp, repo, shas) = snapshot_fixture();
    let stdout = snapshots_json(&repo.root, TREND_HOTSPOT_MASS);
    assert_trend_rows(&stdout, &shas);
}

#[test]
fn snapshots_meta_is_accessible() {
    let (_temp, repo, shas) = snapshot_fixture();
    // Backfilled partitions record their capture mode; also exercise the
    // explicit `--snapshots=DIR` form.
    let assert = sql(&repo.root)
        .arg("--snapshots=.ctx/snapshots")
        .arg("--json")
        .arg("SELECT commit_sha, capture_mode FROM snap.meta ORDER BY committed_at")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert_trend_rows(&stdout, &shas);
    assert!(
        stdout.contains("backfill"),
        "snap.meta must expose capture_mode values:\n{}",
        stdout
    );
}

#[test]
fn snapshots_do_not_weaken_the_sandbox() {
    let (_temp, repo, _shas) = snapshot_fixture();

    // The engine read the Parquet partitions at startup, but user SQL must
    // still be denied filesystem access...
    sql(&repo.root)
        .arg("--snapshots")
        .arg("SELECT * FROM read_parquet('.ctx/snapshots/sha=*/files.parquet')")
        .assert()
        .code(2);

    // ...including writes.
    let leak = repo.root.join("leak.csv");
    let query = format!("COPY (SELECT * FROM snap.files) TO '{}'", leak.display());
    sql(&repo.root)
        .arg("--snapshots")
        .arg(&query)
        .assert()
        .code(2);
    assert!(
        !leak.exists(),
        "COPY must not create a file on disk: {}",
        leak.display()
    );
}

#[test]
fn snapshots_missing_dir_is_an_operational_error() {
    // Indexed project, but `ctx snapshot` never ran: the default dir is absent.
    let temp = indexed_fixture();
    sql(temp.path())
        .arg("--snapshots")
        .arg("SELECT 1")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no snapshots found"))
        .stderr(predicate::str::contains("ctx snapshot"));
}

#[test]
fn snapshots_empty_dir_is_an_operational_error() {
    // The directory exists but holds no `sha=*` partitions.
    let temp = indexed_fixture();
    std::fs::create_dir_all(temp.path().join(".ctx").join("snapshots")).unwrap();
    sql(temp.path())
        .arg("--snapshots")
        .arg("SELECT 1")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no snapshots found"));
}

#[test]
fn snap_tables_require_the_snapshots_flag() {
    let (_temp, repo, _shas) = snapshot_fixture();
    // Without --snapshots the snap schema must not exist, even though the
    // partitions are on disk.
    sql(&repo.root)
        .arg("SELECT * FROM snap.meta")
        .assert()
        .code(2);
}
