//! Scripted mock language server for tests.
//!
//! The ctx binary itself doubles as the mock: when the environment variable
//! `CTX_INTERNAL_MOCK_LSP` points at a scenario JSON file, `main()` enters
//! this stdio loop *before* clap parsing and never runs a real command. Tests
//! register the ctx test binary as the `command` of an `[lsp.*]` block with
//! that variable in its `env` map.
//!
//! The mock speaks real `Content-Length` framing, answers
//! `initialize`/`shutdown` properly, and serves scripted
//! `textDocument/documentSymbol` and `textDocument/definition` responses.
//!
//! Scenario format (all fields optional):
//!
//! ```json
//! {
//!   "server_name": "ctx-mock-lsp",
//!   "exit_after_initialize": false,
//!   "never_respond": false,
//!   "hits_file": "/tmp/hits.log",
//!   "capabilities": { "documentSymbolProvider": true },
//!   "document_symbols": {
//!     "src/main.kt": [ { "name": "...", "kind": 12, ... } ]
//!   },
//!   "definitions": {
//!     "app.py": { "path": "util.py", "line": 0 },
//!     "app.py:3:8": { "path": "util.py", "line": 0, "character": 4 }
//!   }
//! }
//! ```
//!
//! `document_symbols` keys match by URI suffix. `definitions` keys are either
//! a bare suffix (matches any position in that file) or `suffix:line:col`
//! with the 0-based request position. Definition target `path`s are resolved
//! against the `rootUri` received in `initialize`.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use super::transport::read_message;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Scenario {
    server_name: Option<String>,
    server_version: Option<String>,
    /// Respond to `initialize`, then exit immediately (simulated crash).
    exit_after_initialize: bool,
    /// Complete the `initialize` handshake, then read but never answer any
    /// further message (simulated hang; exercises the client's
    /// consecutive-timeout failure logic).
    never_respond: bool,
    /// Append one line per received message (`method<TAB>uri`) to this file.
    hits_file: Option<PathBuf>,
    /// Override the advertised server capabilities.
    capabilities: Option<Value>,
    /// URI-suffix -> `textDocument/documentSymbol` result.
    document_symbols: BTreeMap<String, Value>,
    /// `suffix[:line:col]` -> definition target.
    definitions: BTreeMap<String, MockDefinition>,
}

#[derive(Debug, Deserialize)]
struct MockDefinition {
    /// Target file, relative to the workspace root from `initialize`.
    path: String,
    /// 0-based target line.
    line: u32,
    /// 0-based target character.
    #[serde(default)]
    character: u32,
}

/// Run the mock server loop over stdin/stdout. Returns when the client sends
/// `exit` or closes stdin.
pub fn run_stdio_mock(scenario_path: &Path) {
    let scenario: Scenario = std::fs::read_to_string(scenario_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();

    // rootUri from initialize; definition targets resolve against it.
    let mut root_uri = String::new();

    while let Some(message) = read_message(&mut reader) {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let id = message.get("id").cloned();

        record_hit(&scenario, &method, &message);

        // Simulated hang: the handshake succeeds so the server looks healthy,
        // then every later request times out (the process exits when the
        // client kills it / closes stdin).
        if scenario.never_respond && method != "initialize" {
            continue;
        }

        match method.as_str() {
            "initialize" => {
                if let Some(uri) = message.pointer("/params/rootUri").and_then(Value::as_str) {
                    root_uri = uri.trim_end_matches('/').to_string();
                }
                let capabilities = scenario.capabilities.clone().unwrap_or_else(|| {
                    json!({
                        "textDocumentSync": 1,
                        "documentSymbolProvider": true,
                        "definitionProvider": true,
                    })
                });
                respond(
                    &mut writer,
                    id,
                    json!({
                        "capabilities": capabilities,
                        "serverInfo": {
                            "name": scenario.server_name.as_deref().unwrap_or("ctx-mock-lsp"),
                            "version": scenario.server_version.as_deref().unwrap_or("1.0.0"),
                        },
                    }),
                );
                if scenario.exit_after_initialize {
                    return; // simulated crash right after the handshake
                }
            }
            "shutdown" => respond(&mut writer, id, Value::Null),
            "exit" => return,
            "textDocument/documentSymbol" => {
                let uri = request_uri(&message);
                let result = scenario
                    .document_symbols
                    .iter()
                    .find(|(suffix, _)| uri.ends_with(suffix.as_str()))
                    .map(|(_, symbols)| symbols.clone())
                    .unwrap_or(Value::Null);
                respond(&mut writer, id, result);
            }
            "textDocument/definition" => {
                let uri = request_uri(&message);
                let line = message
                    .pointer("/params/position/line")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let col = message
                    .pointer("/params/position/character")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);

                let target = lookup_definition(&scenario, &uri, line, col);
                let result = match target {
                    Some(def) => json!({
                        "uri": format!("{root_uri}/{}", def.path),
                        "range": {
                            "start": { "line": def.line, "character": def.character },
                            "end": { "line": def.line, "character": def.character },
                        },
                    }),
                    None => Value::Null,
                };
                respond(&mut writer, id, result);
            }
            _ => {
                // Unknown request: answer null so the client never stalls.
                // Notifications (didOpen/didClose/initialized/...) have no id
                // and get no reply.
                if id.is_some() {
                    respond(&mut writer, id, Value::Null);
                }
            }
        }
    }
}

/// Match a definition request against the scenario table: exact
/// `suffix:line:col` entries first, then bare-suffix wildcard entries.
fn lookup_definition<'s>(
    scenario: &'s Scenario,
    uri: &str,
    line: u64,
    col: u64,
) -> Option<&'s MockDefinition> {
    let mut wildcard: Option<&MockDefinition> = None;
    for (key, def) in &scenario.definitions {
        match key.rsplitn(3, ':').collect::<Vec<_>>()[..] {
            // rsplitn yields reversed order: [col, line, suffix]
            [col_s, line_s, suffix] => {
                if let (Ok(l), Ok(c)) = (line_s.parse::<u64>(), col_s.parse::<u64>()) {
                    if uri.ends_with(suffix) && l == line && c == col {
                        return Some(def);
                    }
                    continue;
                }
                // Not numeric: treat the whole key as a suffix.
                if uri.ends_with(key.as_str()) {
                    wildcard = Some(def);
                }
            }
            _ => {
                if uri.ends_with(key.as_str()) {
                    wildcard = Some(def);
                }
            }
        }
    }
    wildcard
}

fn request_uri(message: &Value) -> String {
    message
        .pointer("/params/textDocument/uri")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn respond<W: Write>(writer: &mut W, id: Option<Value>, result: Value) {
    let reply = json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "result": result,
    });
    let body = serde_json::to_vec(&reply).unwrap_or_default();
    let _ = writer.write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    let _ = writer.write_all(&body);
    let _ = writer.flush();
}

fn record_hit(scenario: &Scenario, method: &str, message: &Value) {
    let Some(hits_file) = &scenario.hits_file else {
        return;
    };
    let uri = message
        .pointer("/params/textDocument/uri")
        .and_then(Value::as_str)
        .unwrap_or("");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(hits_file)
    {
        let _ = writeln!(file, "{method}\t{uri}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_lookup_prefers_exact_position() {
        let scenario: Scenario = serde_json::from_value(json!({
            "definitions": {
                "app.py": { "path": "wild.py", "line": 1 },
                "app.py:3:8": { "path": "exact.py", "line": 2 },
            }
        }))
        .unwrap();

        let exact = lookup_definition(&scenario, "file:///w/app.py", 3, 8).unwrap();
        assert_eq!(exact.path, "exact.py");

        let wild = lookup_definition(&scenario, "file:///w/app.py", 9, 9).unwrap();
        assert_eq!(wild.path, "wild.py");

        assert!(lookup_definition(&scenario, "file:///w/other.py", 3, 8).is_none());
    }
}
