//! Machine-readable JSON output.
//!
//! All commands that support `--json` print exactly one JSON document to
//! stdout, wrapped in a common envelope:
//!
//! ```json
//! {
//!   "ctx_version": "0.2.1",
//!   "command": "query.find",
//!   "generated_at": "2026-07-09T12:00:00Z",
//!   "data": { ... }
//! }
//! ```
//!
//! Field names are snake_case throughout, and symbols are always emitted as
//! [`SymbolRef`] objects (never bare strings). See `docs/json-output.md` for
//! the full contract.

use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::db::Symbol;
use crate::error::Result;

/// A reference to a symbol in JSON output.
///
/// This is the canonical shape used everywhere a symbol appears in `--json`
/// payloads.
#[derive(Debug, Clone, Serialize)]
pub struct SymbolRef {
    pub name: String,
    pub qualified_name: Option<String>,
    pub kind: String,
    pub file: String,
    pub line_start: i64,
    pub line_end: i64,
}

impl From<&Symbol> for SymbolRef {
    fn from(s: &Symbol) -> Self {
        SymbolRef {
            name: s.name.clone(),
            qualified_name: s.qualified_name.clone(),
            kind: s.kind.as_str().to_string(),
            file: s.file_path.clone(),
            line_start: s.line_start as i64,
            line_end: s.line_end as i64,
        }
    }
}

impl SymbolRef {
    /// Serialize into a `serde_json::Value`.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// Wrap a command payload in the standard ctx JSON envelope.
pub fn envelope(command: &str, data: serde_json::Value) -> serde_json::Value {
    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default();
    serde_json::json!({
        "ctx_version": env!("CARGO_PKG_VERSION"),
        "command": command,
        "generated_at": generated_at,
        "data": data,
    })
}

/// Pretty-print the envelope for `command` to stdout.
///
/// In JSON mode this must be the only stdout output the command produces.
pub fn emit(command: &str, data: serde_json::Value) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&envelope(command, data))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{SymbolKind, Visibility};

    fn sample_symbol() -> Symbol {
        Symbol {
            id: "src/main.rs::main".to_string(),
            file_path: "src/main.rs".to_string(),
            name: "main".to_string(),
            qualified_name: Some("crate::main".to_string()),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: Some("fn main()".to_string()),
            brief: None,
            docstring: None,
            line_start: 3,
            line_end: 10,
            col_start: 0,
            col_end: 1,
            parent_id: None,
            source: None,
        }
    }

    #[test]
    fn test_envelope_shape() {
        let value = envelope("query.find", serde_json::json!({"symbols": []}));
        let obj = value.as_object().unwrap();

        assert_eq!(obj.len(), 4);
        assert_eq!(
            obj.get("ctx_version").unwrap().as_str().unwrap(),
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(obj.get("command").unwrap().as_str().unwrap(), "query.find");
        assert_eq!(value["data"]["symbols"], serde_json::json!([]));

        // generated_at must be a valid RFC3339 UTC timestamp.
        let ts = obj.get("generated_at").unwrap().as_str().unwrap();
        let parsed = OffsetDateTime::parse(ts, &Rfc3339);
        assert!(parsed.is_ok(), "generated_at is not RFC3339: {}", ts);
    }

    #[test]
    fn test_symbol_ref_snake_case_serialization() {
        let symbol = sample_symbol();
        let value = SymbolRef::from(&symbol).to_value();
        let obj = value.as_object().unwrap();

        // serde_json stores object keys sorted; compare as a sorted set.
        let keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "file",
                "kind",
                "line_end",
                "line_start",
                "name",
                "qualified_name"
            ]
        );
        assert_eq!(value["name"], "main");
        assert_eq!(value["qualified_name"], "crate::main");
        assert_eq!(value["kind"], "function");
        assert_eq!(value["file"], "src/main.rs");
        assert_eq!(value["line_start"], 3);
        assert_eq!(value["line_end"], 10);
    }

    #[test]
    fn test_symbol_ref_null_qualified_name() {
        let mut symbol = sample_symbol();
        symbol.qualified_name = None;
        let value = SymbolRef::from(&symbol).to_value();
        assert!(value["qualified_name"].is_null());
    }

    #[test]
    fn test_emitted_output_is_exactly_one_json_document() {
        // Mirror what emit() prints and verify the stream contains exactly
        // one JSON document.
        let printed = format!(
            "{}\n",
            serde_json::to_string_pretty(&envelope(
                "search",
                serde_json::json!({"query": "q", "results": []})
            ))
            .unwrap()
        );

        let mut stream =
            serde_json::Deserializer::from_str(&printed).into_iter::<serde_json::Value>();
        let first = stream.next().expect("expected one document").unwrap();
        assert_eq!(first["command"], "search");
        assert!(stream.next().is_none(), "expected exactly one document");
    }
}
