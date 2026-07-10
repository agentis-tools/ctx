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

use std::path::Path;

use assert_cmd::Command;
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
