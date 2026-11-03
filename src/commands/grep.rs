//! `ctx grep`: ripgrep-style text search with index annotations.
//!
//! See [`ctx::grep`] for the search itself and the tag vocabulary.

use std::env;
use std::io::{self, Write};

use ctx::error::{CtxError, Result};
use ctx::exit::Outcome;
use ctx::grep::{self, GrepOptions, GrepReport, IndexState, TagKind};
use ctx::index;
use ctx::walker::FilePatternFilter;

/// How many distinct targets the human summary lists.
const SUMMARY_TARGETS: usize = 5;

/// Run `ctx grep`.
///
/// Always exits 0 on success, with or without matches: a match is not a
/// finding. Bad patterns, globs, or types are operational errors (exit 2).
pub fn run_grep(options: GrepOptions, patterns: &[String], json: bool) -> Result<Outcome> {
    let root = env::current_dir()?;
    let scope = FilePatternFilter::new(&root, patterns)
        .map_err(|error| CtxError::Other(format!("Invalid file pattern: {error}")))?;

    let db = if options.annotate {
        match index::open_database(&root) {
            Ok(db) => Some(db),
            Err(CtxError::IndexNotFound(_)) => None,
            Err(other) => return Err(other),
        }
    } else {
        None
    };

    let report = grep::search(&root, &scope, &options, db.as_ref())
        .map_err(|error| CtxError::Other(format!("ctx grep: {error}")))?;

    if json {
        ctx::json::emit("grep", grep_data(&report)?)?;
    } else if let Err(error) = print_report(&report) {
        // `ctx grep ... | head` closes the pipe early; like rg, that is not
        // a failure.
        if error.kind() != io::ErrorKind::BrokenPipe {
            return Err(error.into());
        }
    }
    Ok(Outcome::Clean)
}

/// The `grep` JSON payload: the report plus its summary counts.
fn grep_data(report: &GrepReport) -> Result<serde_json::Value> {
    let mut data = serde_json::to_value(report)?;
    let by_tag: serde_json::Map<String, serde_json::Value> = report
        .tag_counts()
        .into_iter()
        .map(|(kind, n)| (kind.as_str().to_string(), n.into()))
        .collect();
    data["by_tag"] = serde_json::Value::Object(by_tag);
    data["targets"] = report
        .targets()
        .into_iter()
        .map(|(target, count)| serde_json::json!({ "target": target, "matches": count }))
        .collect();
    Ok(data)
}

fn print_report(report: &GrepReport) -> io::Result<()> {
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    if report.index == IndexState::Disabled {
        // Plain mode mirrors `rg -n --no-heading` so it can serve as the
        // un-annotated arm of a comparison.
        for m in &report.matches {
            writeln!(out, "{}:{}:{}", m.file, m.line, m.text)?;
        }
        return out.flush();
    }

    writeln!(out, "{}", summary_line(report))?;
    for m in &report.matches {
        writeln!(
            out,
            "{}:{}:{}  [{}]",
            m.file,
            m.line,
            m.text,
            grep::render_tags(&m.tags)
        )?;
    }
    out.flush()
}

/// e.g. `ctx grep: 14 matches in 6 files · ref 6 · unresolved 2 · unlinked 3 ·
/// unindexed 2 · targets: Conn::close ×5, File::close ×1`
fn summary_line(report: &GrepReport) -> String {
    let mut parts = vec![format!(
        "ctx grep: {} matches in {} files{}",
        report.matches.len(),
        report.files_matched,
        if report.truncated { " (truncated)" } else { "" }
    )];
    if report.index == IndexState::Missing {
        parts.push("no index (run `ctx index`)".to_string());
    }
    let counts = report.tag_counts();
    for kind in TagKind::ALL {
        if let Some(n) = counts.get(&kind) {
            parts.push(format!("{} {n}", kind.as_str()));
        }
    }
    let targets = report.targets();
    if !targets.is_empty() {
        let mut listed: Vec<String> = targets
            .iter()
            .take(SUMMARY_TARGETS)
            .map(|(label, n)| format!("{label} ×{n}"))
            .collect();
        if targets.len() > SUMMARY_TARGETS {
            listed.push(format!("+{} more", targets.len() - SUMMARY_TARGETS));
        }
        parts.push(format!("targets: {}", listed.join(", ")));
    }
    parts.join(" · ")
}
