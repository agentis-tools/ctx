//! `ctx sql` — raw, read-only SQL access to the code index.
//!
//! Queries run through DuckDB against the stable `v1.*` view layer. Safety is
//! enforced entirely by engine configuration (see
//! [`ctx::analytics::Analytics::open_sql_sandbox`]) — this command never
//! inspects, filters, or blocklists SQL text.

use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use ctx::error::{CtxError, Result};
use ctx::exit::Outcome;

/// The reference for the public `v1` schema, printed by `ctx sql --schema`.
///
/// This constant is the single source of truth; the docs site page
/// `docs/website/docs/sql-schema.md` mirrors it verbatim (a drift-guard test
/// keeps them byte-identical).
pub const SQL_SCHEMA_REFERENCE: &str = include_str!("sql_schema.md");

/// Arguments for `ctx sql`, mirrored from the CLI variant.
// Several fields are only read on the `duckdb` execution path.
#[cfg_attr(not(feature = "duckdb"), allow(dead_code))]
pub struct SqlArgs {
    pub query: Option<String>,
    pub file: Option<PathBuf>,
    pub format: String,
    pub json: bool,
    pub max_rows: usize,
    pub timeout: u64,
    pub fail_on_rows: bool,
    pub schema: bool,
}

/// Run `ctx sql`.
///
/// Exit convention (via [`Outcome`] / `Err`):
/// - `Ok(Outcome::Clean)` → 0 (ran; with `--fail-on-rows`: zero rows)
/// - `Ok(Outcome::Findings)` → 1 (only with `--fail-on-rows`: ≥ 1 row)
/// - `Err(_)` → 2 (SQL error, timeout, missing index, invalid flags, stub build)
pub fn run_sql(args: SqlArgs) -> Result<Outcome> {
    // `--schema` needs neither an index nor the engine; works in every build.
    if args.schema {
        print!("{}", SQL_SCHEMA_REFERENCE);
        return Ok(Outcome::Clean);
    }

    // Resolve the effective output format (`--json` is an alias for json).
    let format = if args.json {
        "json"
    } else {
        args.format.as_str()
    };
    if !matches!(format, "table" | "csv" | "json") {
        return Err(CtxError::Other(format!(
            "unknown --format '{}'; expected table, csv, or json",
            format
        )));
    }

    let sql_text = resolve_query_text(&args)?;
    if sql_text.trim().is_empty() {
        return Err(CtxError::Other(
            "no query provided; pass SQL as an argument, via --file, or on stdin".into(),
        ));
    }

    #[cfg(not(feature = "duckdb"))]
    {
        let _ = format;
        Err(CtxError::Other(
            "ctx sql requires the duckdb feature; reinstall with default features".into(),
        ))
    }

    #[cfg(feature = "duckdb")]
    {
        run_sql_duckdb(&args, format, &sql_text)
    }
}

/// Resolve the query text from the positional arg, `--file`, or stdin.
fn resolve_query_text(args: &SqlArgs) -> Result<String> {
    match (&args.query, &args.file) {
        (Some(q), Some(_)) if q != "-" => Err(CtxError::Other(
            "provide the query as an argument OR via --file, not both".into(),
        )),
        (_, Some(path)) => Ok(std::fs::read_to_string(path)?),
        (Some(q), None) if q != "-" => Ok(q.clone()),
        _ => {
            // Omitted or "-": read stdin, but don't hang an interactive terminal.
            if std::io::stdin().is_terminal() {
                return Err(CtxError::Other(
                    "no query provided; pass SQL as an argument, via --file, or on stdin".into(),
                ));
            }
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
    }
}

/// Split a SQL submission into individual statements on top-level `;`,
/// respecting single-quoted strings, double-quoted identifiers, line comments
/// (`--`), and block comments (`/* */`). This governs execution semantics
/// (the one-result-set rule) only — it is never a safety filter.
#[cfg_attr(not(feature = "duckdb"), allow(dead_code))]
fn split_sql_statements(sql: &str) -> Vec<String> {
    #[derive(PartialEq)]
    enum State {
        Normal,
        Single,
        Double,
        Line,
        Block,
    }

    let mut statements = Vec::new();
    let mut current = String::new();
    let mut state = State::Normal;
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        match state {
            State::Normal => match c {
                '\'' => {
                    current.push(c);
                    state = State::Single;
                }
                '"' => {
                    current.push(c);
                    state = State::Double;
                }
                '-' if chars.peek() == Some(&'-') => {
                    current.push(c);
                    current.push(chars.next().unwrap());
                    state = State::Line;
                }
                '/' if chars.peek() == Some(&'*') => {
                    current.push(c);
                    current.push(chars.next().unwrap());
                    state = State::Block;
                }
                ';' => {
                    let trimmed = current.trim();
                    if !trimmed.is_empty() {
                        statements.push(trimmed.to_string());
                    }
                    current.clear();
                }
                _ => current.push(c),
            },
            State::Single => {
                current.push(c);
                if c == '\'' {
                    if chars.peek() == Some(&'\'') {
                        current.push(chars.next().unwrap());
                    } else {
                        state = State::Normal;
                    }
                }
            }
            State::Double => {
                current.push(c);
                if c == '"' {
                    if chars.peek() == Some(&'"') {
                        current.push(chars.next().unwrap());
                    } else {
                        state = State::Normal;
                    }
                }
            }
            State::Line => {
                current.push(c);
                if c == '\n' {
                    state = State::Normal;
                }
            }
            State::Block => {
                current.push(c);
                if c == '*' && chars.peek() == Some(&'/') {
                    current.push(chars.next().unwrap());
                    state = State::Normal;
                }
            }
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        statements.push(trimmed.to_string());
    }
    statements
}

#[cfg(feature = "duckdb")]
mod engine {
    use super::*;
    use ctx::analytics::{Analytics, SqlResult};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// Internal execution error, kept separate from `CtxError` so the caller can
    /// distinguish a timeout and the one-result-set rule from a plain SQL error.
    enum EngineError {
        MultipleResultSets,
        Duck(duckdb::Error),
    }

    pub fn run_sql_duckdb(args: &SqlArgs, format: &str, sql_text: &str) -> Result<Outcome> {
        use ctx::index::{CTX_DIR, DB_FILE};

        let root = std::env::current_dir()?;
        let db_path = root.join(CTX_DIR).join(DB_FILE);
        if !db_path.exists() {
            return Err(CtxError::Other(format!(
                "no index found at {}; run `ctx index` first",
                db_path.display()
            )));
        }

        let statements = split_sql_statements(sql_text);
        if statements.is_empty() {
            return Err(CtxError::Other("no SQL statement found".into()));
        }

        let analytics = Analytics::open_sql_sandbox(&root)?;

        // Timeout watchdog: interrupt the in-flight query if it overruns.
        let timed_out = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let watchdog = if args.timeout > 0 {
            let handle = analytics.interrupt_handle();
            let timed_out = timed_out.clone();
            let done = done.clone();
            let timeout = args.timeout;
            Some(std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(timeout);
                while Instant::now() < deadline {
                    if done.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                if !done.load(Ordering::SeqCst) {
                    timed_out.store(true, Ordering::SeqCst);
                    handle.interrupt();
                }
            }))
        } else {
            None
        };

        let start = Instant::now();
        let result = execute_statements(&analytics, &statements, args.max_rows);
        let elapsed_ms = start.elapsed().as_millis();

        // Retire the watchdog before we touch the result.
        done.store(true, Ordering::SeqCst);
        if let Some(w) = watchdog {
            let _ = w.join();
        }

        let result = match result {
            Ok(r) => r,
            Err(EngineError::MultipleResultSets) => {
                return Err(CtxError::Other(
                    "only the final statement may return rows; earlier statements must not produce a result set"
                        .into(),
                ));
            }
            Err(EngineError::Duck(e)) => {
                if timed_out.load(Ordering::SeqCst) {
                    return Err(CtxError::Other(format!(
                        "query exceeded the {}s timeout; raise it with --timeout",
                        args.timeout
                    )));
                }
                return Err(CtxError::Other(format!("sql error: {}", e)));
            }
        };

        render(&result, format, elapsed_ms);

        if args.fail_on_rows && !result.rows.is_empty() {
            Ok(Outcome::Findings)
        } else {
            Ok(Outcome::Clean)
        }
    }

    /// Execute leading statements (which must not produce result sets), then run
    /// the final statement and return its rows.
    fn execute_statements(
        analytics: &Analytics,
        statements: &[String],
        max_rows: usize,
    ) -> std::result::Result<SqlResult, EngineError> {
        let (last, leading) = statements
            .split_last()
            .expect("statements is non-empty (checked by caller)");
        for stmt in leading {
            let produced_result = analytics
                .exec_non_final_statement(stmt)
                .map_err(EngineError::Duck)?;
            if produced_result {
                return Err(EngineError::MultipleResultSets);
            }
        }
        analytics
            .run_final_statement(last, max_rows)
            .map_err(EngineError::Duck)
    }

    fn render(result: &SqlResult, format: &str, elapsed_ms: u128) {
        match format {
            "json" => render_json(result, elapsed_ms),
            "csv" => render_csv(result),
            _ => render_table(result),
        }
    }

    fn render_json(result: &SqlResult, elapsed_ms: u128) {
        let columns: Vec<serde_json::Value> = result
            .columns
            .iter()
            .map(|c| serde_json::json!({ "name": c.name, "type": c.type_name }))
            .collect();

        // Reuse the shared ctx JSON envelope (ctx_version / command /
        // generated_at / data) so `ctx sql --json` matches every other command.
        let data = serde_json::json!({
            "columns": columns,
            "rows": result.rows,
            "row_count": result.rows.len(),
            "truncated": result.truncated,
            "elapsed_ms": elapsed_ms,
        });

        if let Err(e) = ctx::json::emit("sql", data) {
            eprintln!("failed to serialize result: {}", e);
        }
    }

    const MAX_CELL: usize = 200;

    /// Render a JSON value to a display string, returning whether it was
    /// truncated at [`MAX_CELL`] characters.
    fn cell_string(value: &serde_json::Value) -> (String, bool) {
        let raw = match value {
            serde_json::Value::Null => return ("∅".to_string(), false),
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            other => other.to_string(),
        };
        if raw.chars().count() > MAX_CELL {
            let truncated: String = raw.chars().take(MAX_CELL).collect();
            (format!("{}…", truncated), true)
        } else {
            (raw, false)
        }
    }

    fn render_table(result: &SqlResult) {
        let headers: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
        let mut any_cell_truncated = false;

        // Pre-render every cell so widths and truncation are computed once.
        let rendered: Vec<Vec<String>> = result
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|v| {
                        let (s, truncated) = cell_string(v);
                        any_cell_truncated |= truncated;
                        s
                    })
                    .collect()
            })
            .collect();

        let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
        for row in &rendered {
            for (i, cell) in row.iter().enumerate() {
                if i < widths.len() {
                    widths[i] = widths[i].max(cell.chars().count());
                }
            }
        }

        let mut out = String::new();
        push_row(
            &mut out,
            &headers.iter().map(|h| h.to_string()).collect::<Vec<_>>(),
            &widths,
        );
        push_separator(&mut out, &widths);
        for row in &rendered {
            push_row(&mut out, row, &widths);
        }
        print!("{}", out);

        if result.truncated {
            eprintln!(
                "note: output capped at {} rows; raise the limit with --max-rows",
                result.rows.len()
            );
        }
        if any_cell_truncated {
            eprintln!(
                "note: some cells were truncated at {} characters; use --format json or csv for full values",
                MAX_CELL
            );
        }
    }

    fn push_row(out: &mut String, cells: &[String], widths: &[usize]) {
        let mut parts = Vec::with_capacity(cells.len());
        for (i, cell) in cells.iter().enumerate() {
            let width = widths.get(i).copied().unwrap_or(0);
            let pad = width.saturating_sub(cell.chars().count());
            parts.push(format!("{}{}", cell, " ".repeat(pad)));
        }
        out.push_str(parts.join("  ").trim_end());
        out.push('\n');
    }

    fn push_separator(out: &mut String, widths: &[usize]) {
        let parts: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
        out.push_str(parts.join("  ").trim_end());
        out.push('\n');
    }

    fn render_csv(result: &SqlResult) {
        let mut out = String::new();
        let headers: Vec<String> = result.columns.iter().map(|c| csv_field(&c.name)).collect();
        out.push_str(&headers.join(","));
        out.push_str("\r\n");

        for row in &result.rows {
            let fields: Vec<String> = row
                .iter()
                .map(|v| match v {
                    serde_json::Value::Null => String::new(),
                    serde_json::Value::String(s) => csv_field(s),
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Number(n) => n.to_string(),
                    other => csv_field(&other.to_string()),
                })
                .collect();
            out.push_str(&fields.join(","));
            out.push_str("\r\n");
        }
        print!("{}", out);

        if result.truncated {
            eprintln!(
                "note: output capped at {} rows; raise the limit with --max-rows",
                result.rows.len()
            );
        }
    }

    /// Quote a CSV field per RFC-4180 when it contains a comma, quote, CR, or LF.
    fn csv_field(s: &str) -> String {
        if s.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    }
}

#[cfg(feature = "duckdb")]
use engine::run_sql_duckdb;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_simple_statements() {
        let stmts = split_sql_statements("SELECT 1; SELECT 2");
        assert_eq!(stmts, vec!["SELECT 1", "SELECT 2"]);
    }

    #[test]
    fn ignores_semicolons_in_strings_and_comments() {
        let stmts = split_sql_statements(
            "SELECT ';' AS a; -- comment; not a split\nSELECT /* also; not */ 2",
        );
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("';'"));
        assert!(stmts[1].contains("2"));
    }

    #[test]
    fn single_statement_yields_one() {
        assert_eq!(split_sql_statements("SELECT * FROM v1.symbols").len(), 1);
    }

    #[test]
    fn trailing_semicolon_is_not_an_empty_statement() {
        assert_eq!(split_sql_statements("SELECT 1;").len(), 1);
    }

    /// The `--schema` output and the published docs page must be byte-identical
    /// in their content sections (generated from one source).
    #[test]
    fn schema_reference_matches_docs_page() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let doc_path = std::path::Path::new(manifest).join("docs/website/docs/sql-schema.md");
        let doc = std::fs::read_to_string(&doc_path)
            .unwrap_or_else(|e| panic!("read {}: {}", doc_path.display(), e));

        // Strip a leading YAML front-matter block (--- … ---) if present.
        let body = strip_front_matter(&doc);
        assert_eq!(
            body.trim(),
            SQL_SCHEMA_REFERENCE.trim(),
            "docs/website/docs/sql-schema.md content must match SQL_SCHEMA_REFERENCE"
        );
    }

    fn strip_front_matter(doc: &str) -> &str {
        if let Some(rest) = doc.strip_prefix("---") {
            if let Some(end) = rest.find("\n---") {
                // Skip past the closing delimiter line.
                let after = &rest[end + 4..];
                return after.trim_start_matches(['\n', '\r']);
            }
        }
        doc
    }
}
