//! Gate-evaluation logging.
//!
//! When the `CTX_GATE_LOG` environment variable is set, `ctx score` appends
//! one JSON line per gate evaluation to a local log file (default
//! `.ctx/gate-log.jsonl`). Opt-in, local-only; ctx ships no telemetry.
//!
//! `CTX_GATE_LOG` values:
//!
//! - unset, empty, or `0` -- logging disabled
//! - `1` or `true` -- log to [`DEFAULT_GATE_LOG_PATH`] under the repo root
//! - anything else -- treated as the log path (joined to the repo root when
//!   relative)

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::error::Result;

/// Version of the [`GateRecord`] line format.
pub const GATE_LOG_SCHEMA_VERSION: u32 = 1;

/// Default log location (relative to the repo root) for `CTX_GATE_LOG=1`.
pub const DEFAULT_GATE_LOG_PATH: &str = ".ctx/gate-log.jsonl";

/// One gate evaluation, serialized as a single JSONL line.
#[derive(Debug, Serialize)]
pub struct GateRecord {
    /// Line format version ([`GATE_LOG_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Evaluation time, RFC3339 UTC (same shape as the JSON envelope's
    /// `generated_at`; see [`crate::json`]).
    pub ts: String,
    /// The ctx version that evaluated the gate.
    pub ctx_version: String,
    /// The command that evaluated the gate (`"score"`).
    pub source: String,
    /// The git reference the score was computed against.
    pub against: String,
    /// The raw `--fail-on` expression, if any.
    pub fail_on: Option<String>,
    /// The scorecard metrics: the same seven-key object the `--json`
    /// payload emits under `metrics`.
    pub metrics: serde_json::Value,
    /// Rendered `--fail-on` conditions that fired (empty on pass).
    pub failed_conditions: Vec<String>,
    /// `"pass"` or `"fail"`.
    pub outcome: String,
    /// Whether blocking mode was requested (`CTX_GATE_BLOCKING=1`).
    pub blocking: bool,
    /// Claude Code session id (`CLAUDE_SESSION_ID`), if present.
    pub session_id: Option<String>,
}

/// The gate log path selected by `CTX_GATE_LOG`, or `None` when logging is
/// disabled.
pub fn gate_log_target(root: &Path) -> Option<PathBuf> {
    resolve(std::env::var("CTX_GATE_LOG").ok().as_deref(), root)
}

/// Core `CTX_GATE_LOG` value resolution (env-free, so tests can drive it
/// without mutating process env).
fn resolve(value: Option<&str>, root: &Path) -> Option<PathBuf> {
    match value? {
        "" | "0" => None,
        "1" | "true" => Some(root.join(DEFAULT_GATE_LOG_PATH)),
        path => {
            let path = Path::new(path);
            if path.is_absolute() {
                Some(path.to_path_buf())
            } else {
                Some(root.join(path))
            }
        }
    }
}

/// Whether blocking mode is requested (`CTX_GATE_BLOCKING=1`, exactly).
pub fn blocking_enabled() -> bool {
    std::env::var("CTX_GATE_BLOCKING").as_deref() == Ok("1")
}

/// The Claude Code session id (`CLAUDE_SESSION_ID`), if present.
pub fn session_id() -> Option<String> {
    std::env::var("CLAUDE_SESSION_ID").ok()
}

/// Current UTC time as an RFC3339 string.
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}

/// Append one record to the log at `path` as a single JSONL line.
///
/// Creates parent directories as needed. The record plus trailing newline
/// go out in one write call, so concurrent appenders on the same local log
/// do not interleave within a line.
pub fn append(path: &Path, record: &GateRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_string(record)?;
    line.push('\n');
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_record(outcome: &str) -> GateRecord {
        GateRecord {
            schema_version: GATE_LOG_SCHEMA_VERSION,
            ts: now_rfc3339(),
            ctx_version: env!("CARGO_PKG_VERSION").to_string(),
            source: "score".to_string(),
            against: "main".to_string(),
            fail_on: Some("new_duplication>0".to_string()),
            metrics: serde_json::json!({"new_duplication": 1}),
            failed_conditions: vec!["new_duplication > 0".to_string()],
            outcome: outcome.to_string(),
            blocking: false,
            session_id: None,
        }
    }

    #[test]
    fn test_resolve_disabled_values() {
        let root = Path::new("/repo");
        assert_eq!(resolve(None, root), None);
        assert_eq!(resolve(Some(""), root), None);
        assert_eq!(resolve(Some("0"), root), None);
    }

    #[test]
    fn test_resolve_default_path_values() {
        let root = Path::new("/repo");
        let default = root.join(DEFAULT_GATE_LOG_PATH);
        assert_eq!(resolve(Some("1"), root), Some(default.clone()));
        assert_eq!(resolve(Some("true"), root), Some(default));
    }

    #[test]
    fn test_resolve_custom_paths() {
        let root = Path::new("/repo");
        // Relative values are joined to the root.
        assert_eq!(
            resolve(Some("logs/gates.jsonl"), root),
            Some(root.join("logs/gates.jsonl"))
        );
        // Absolute values are used as-is.
        assert_eq!(
            resolve(Some("/var/log/gates.jsonl"), root),
            Some(PathBuf::from("/var/log/gates.jsonl"))
        );
    }

    #[test]
    fn test_append_creates_dirs_and_accumulates_lines() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("nested/dir/gate-log.jsonl");

        append(&path, &sample_record("fail")).unwrap();
        append(&path, &sample_record("pass")).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.ends_with('\n'), "content: {content:?}");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "content: {content:?}");

        // Every line is one valid JSON object with the expected shape.
        for (line, outcome) in lines.iter().zip(["fail", "pass"]) {
            let doc: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(doc["schema_version"], GATE_LOG_SCHEMA_VERSION);
            assert_eq!(doc["source"], "score");
            assert_eq!(doc["outcome"], outcome);
            // ts round-trips as RFC3339.
            let ts = doc["ts"].as_str().unwrap();
            assert!(OffsetDateTime::parse(ts, &Rfc3339).is_ok(), "ts: {ts}");
        }
    }
}
