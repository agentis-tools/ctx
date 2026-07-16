//! Language Server Protocol extraction backend.
//!
//! Alongside the builtin tree-sitter parsers, any stdio language server can
//! be registered declaratively in `.ctx/config.toml` (see
//! [`config::LspServerConfig`]). Per language the backend is selectable:
//!
//! - `tree-sitter` (default when nothing is configured): builtin grammars.
//! - `lsp`: symbols and call edges come from the language server.
//! - `hybrid` (default for configured blocks): tree-sitter extracts, the
//!   language server resolves cross-file references afterwards (Stage B).
//!
//! The subsystem is runtime-gated: without any `[lsp.*]` config block nothing
//! is spawned and indexing behaves exactly as before. Failures never abort an
//! indexing run — a broken or missing server degrades to tree-sitter (builtin
//! languages) or to file records without symbols (dynamic languages), with a
//! warning on stderr.

pub mod client;
pub mod config;
pub mod extract;
pub mod mock;
pub mod resolve;
pub mod status;
pub mod transport;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::db::{Edge, ParseResult};

pub use config::{LspBackend, LspConfig, LspServerConfig};

use client::LspClient;
use extract::ExtractedSymbol;

/// How a file should be extracted, as decided by extension claims in the
/// `[lsp.*]` config plus the builtin language table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileBackend {
    /// Builtin tree-sitter extraction (also the fallback when no LSP block
    /// claims the file's extension).
    TreeSitter,
    /// Extract with the language server registered for this language.
    Lsp(String),
    /// Extract with tree-sitter, resolve leftover cross-file references with
    /// the language server for this language (Stage B).
    Hybrid(String),
    /// Neither a builtin grammar nor an LSP registration covers this file.
    Unsupported,
}

/// Lifecycle state of one language's server within a run.
enum ClientSlot {
    Ready(Box<LspClient>),
    Failed(String),
}

/// Per-run registry of language-server clients (one per configured language,
/// spawned lazily on first use).
pub struct LspManager {
    root: PathBuf,
    verbose: bool,
    servers: BTreeMap<String, LspServerConfig>,
    /// extension (lowercase, no dot) -> language key; first config block wins.
    extension_claims: BTreeMap<String, String>,
    clients: HashMap<String, ClientSlot>,
    /// Languages already warned about on stderr (warn once per run).
    warned: HashSet<String>,
}

impl LspManager {
    /// Build a manager from the project config. Returns `None` when no valid
    /// `[lsp.*]` block exists — the subsystem then stays completely inert.
    pub fn from_config(root: &Path, lsp_config: &LspConfig, verbose: bool) -> Option<Self> {
        if lsp_config.lsp.is_empty() {
            return None;
        }
        let (servers, extension_claims) = config::validate(&lsp_config.lsp);
        if servers.is_empty() {
            return None;
        }
        Some(Self {
            root: root.to_path_buf(),
            verbose,
            servers,
            extension_claims,
            clients: HashMap::new(),
            warned: HashSet::new(),
        })
    }

    /// Project root the servers run in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Decide the extraction backend for a path.
    pub fn backend_for(&self, path: &Path) -> FileBackend {
        let builtin_supported = crate::parser::CodeParser::is_supported_static(path);

        let claimed = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .and_then(|ext| self.extension_claims.get(&ext));

        let Some(language) = claimed else {
            return if builtin_supported {
                FileBackend::TreeSitter
            } else {
                FileBackend::Unsupported
            };
        };

        match self.servers[language].backend {
            LspBackend::TreeSitter => {
                if builtin_supported {
                    FileBackend::TreeSitter
                } else {
                    FileBackend::Unsupported
                }
            }
            LspBackend::Lsp => FileBackend::Lsp(language.clone()),
            LspBackend::Hybrid => {
                if builtin_supported {
                    FileBackend::Hybrid(language.clone())
                } else {
                    // No builtin grammar to extract with: hybrid degrades to
                    // full LSP extraction for dynamic languages.
                    FileBackend::Lsp(language.clone())
                }
            }
        }
    }

    /// The configured server command for a language (for messages).
    fn command_for(&self, language: &str) -> String {
        self.servers
            .get(language)
            .map(|c| c.command.clone())
            .unwrap_or_default()
    }

    /// Warn once per language that its server is unusable.
    fn warn_fallback(&mut self, language: &str, reason: &str) {
        if self.warned.insert(language.to_string()) {
            eprintln!(
                "Warning: LSP server '{}' for {} {}; falling back to tree-sitter",
                self.command_for(language),
                language,
                reason
            );
        }
    }

    /// Lazily spawn (or reuse) the client for a language. Returns `None` when
    /// the server is (or just became) failed; the warning has been emitted.
    fn client(&mut self, language: &str) -> Option<&mut LspClient> {
        // Existing slot: check for a failure recorded by a previous request.
        if let Some(slot) = self.clients.get(language) {
            match slot {
                ClientSlot::Failed(_) => return None,
                ClientSlot::Ready(client) => {
                    if let Some(reason) = client.failure() {
                        let reason = reason.to_string();
                        self.clients
                            .insert(language.to_string(), ClientSlot::Failed(reason.clone()));
                        self.warn_fallback(language, &reason);
                        return None;
                    }
                }
            }
        } else {
            let cfg = self.servers.get(language).cloned()?;
            match LspClient::spawn(&cfg, &self.root, self.verbose) {
                Ok(client) => {
                    if self.verbose {
                        let (name, version) = client.server_info();
                        eprintln!(
                            "Started LSP server '{}' for {} ({} {})",
                            cfg.command,
                            language,
                            name.unwrap_or("unknown"),
                            version.unwrap_or("")
                        );
                    }
                    self.clients
                        .insert(language.to_string(), ClientSlot::Ready(Box::new(client)));
                }
                Err(e) => {
                    self.clients
                        .insert(language.to_string(), ClientSlot::Failed(e.reason.clone()));
                    self.warn_fallback(language, &e.reason);
                    return None;
                }
            }
        }

        match self.clients.get_mut(language) {
            Some(ClientSlot::Ready(client)) => Some(client.as_mut()),
            _ => None,
        }
    }

    /// Client accessor for the Stage B resolver (crate-internal).
    pub(crate) fn client_for_stage_b(&mut self, language: &str) -> Option<&mut LspClient> {
        self.client(language)
    }

    /// Record a failure detected while a client borrow was held.
    fn note_failure_if_any(&mut self, language: &str) {
        if let Some(ClientSlot::Ready(client)) = self.clients.get(language) {
            if let Some(reason) = client.failure() {
                let reason = reason.to_string();
                self.clients
                    .insert(language.to_string(), ClientSlot::Failed(reason.clone()));
                self.warn_fallback(language, &reason);
            }
        }
    }

    /// Stage A: extract one file with the language's server.
    ///
    /// Returns `None` when the server is unusable (spawn failure, crash,
    /// repeated timeouts) or the extraction request failed — the caller then
    /// falls back to tree-sitter / an empty symbol set. LSP failures never
    /// become indexing errors.
    pub fn extract(&mut self, language: &str, rel_path: &str, text: &str) -> Option<ParseResult> {
        let abs_path = self.root.join(rel_path);
        let uri = path_to_uri(&abs_path);
        let verbose = self.verbose;

        let outcome = {
            let client = self.client(language)?;

            if client.did_open(&uri, language, text).is_err() {
                None
            } else {
                let response = client.document_symbols(&uri);
                match response {
                    Ok(response) => {
                        let symbols: Vec<ExtractedSymbol> = response
                            .map(|r| extract::symbols_from_response(rel_path, text, r))
                            .unwrap_or_default();
                        let edges = collect_call_edges(client, &uri, &symbols, verbose, rel_path);
                        client.did_close(&uri);
                        Some((symbols, edges))
                    }
                    Err(e) => {
                        if verbose {
                            eprintln!("Warning: LSP documentSymbol failed for {rel_path}: {e}");
                        }
                        client.did_close(&uri);
                        None
                    }
                }
            }
        };

        // The borrow on the client is released: sync failure state (warns
        // once per language when the server died mid-run).
        self.note_failure_if_any(language);

        let (symbols, edges) = outcome?;
        Some(ParseResult {
            file_path: rel_path.to_string(),
            language: language.to_string(),
            symbols: symbols.into_iter().map(|e| e.symbol).collect(),
            edges,
            module: None,
        })
    }

    /// Whether Stage B resolution applies to files of this backend.
    pub fn wants_stage_b(&self, backend: &FileBackend) -> bool {
        matches!(backend, FileBackend::Lsp(_) | FileBackend::Hybrid(_))
    }

    /// Write the `.ctx/lsp_status.json` sidecar for this run.
    pub fn write_status(&self) {
        let entries: Vec<status::LspStatusEntry> = self
            .servers
            .iter()
            .map(|(language, cfg)| {
                let (state, reason, server_name, server_version, capabilities) =
                    match self.clients.get(language) {
                        Some(ClientSlot::Ready(client)) => match client.failure() {
                            Some(reason) => (
                                "failed",
                                Some(reason.to_string()),
                                client.server_name.clone(),
                                client.server_version.clone(),
                                client.capability_names(),
                            ),
                            None => (
                                "healthy",
                                None,
                                client.server_name.clone(),
                                client.server_version.clone(),
                                client.capability_names(),
                            ),
                        },
                        Some(ClientSlot::Failed(reason)) => {
                            ("failed", Some(reason.clone()), None, None, Vec::new())
                        }
                        None => ("idle", None, None, None, Vec::new()),
                    };
                status::LspStatusEntry {
                    language: language.clone(),
                    command: cfg.command.clone(),
                    backend: cfg.backend.as_str().to_string(),
                    state: state.to_string(),
                    reason,
                    server_name,
                    server_version,
                    capabilities,
                }
            })
            .collect();

        if let Err(e) = status::write_status_file(&self.root, &entries) {
            if self.verbose {
                eprintln!("Warning: failed to write lsp_status.json: {e}");
            }
        }
    }

    /// Shut down every running server (graceful `shutdown`/`exit`, then kill).
    pub fn shutdown_all(&mut self) {
        for slot in self.clients.values_mut() {
            if let ClientSlot::Ready(client) = slot {
                client.shutdown();
            }
        }
        self.clients.clear();
    }
}

impl Drop for LspManager {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}

/// Collect `Calls` edges via the call-hierarchy requests (best effort; any
/// failure keeps the symbols and simply stops collecting edges).
fn collect_call_edges(
    client: &mut LspClient,
    uri: &str,
    symbols: &[ExtractedSymbol],
    verbose: bool,
    rel_path: &str,
) -> Vec<Edge> {
    let mut edges = Vec::new();
    if !client.supports("callHierarchyProvider") {
        return edges;
    }

    for extracted in symbols {
        if !matches!(
            extracted.symbol.kind,
            crate::db::SymbolKind::Function | crate::db::SymbolKind::Method
        ) {
            continue;
        }

        let items = match client.prepare_call_hierarchy(uri, extracted.sel_line, extracted.sel_col)
        {
            Ok(items) => items,
            Err(e) => {
                if verbose {
                    eprintln!("Warning: prepareCallHierarchy failed for {rel_path}: {e}");
                }
                if client.failure().is_some() {
                    break;
                }
                continue;
            }
        };

        let Some(item) = items.first() else {
            continue;
        };

        match client.outgoing_calls(item) {
            Ok(calls) => {
                edges.extend(extract::edges_from_outgoing_calls(
                    extracted, uri, symbols, &calls,
                ));
            }
            Err(e) => {
                if verbose {
                    eprintln!("Warning: outgoingCalls failed for {rel_path}: {e}");
                }
                if client.failure().is_some() {
                    break;
                }
            }
        }
    }

    edges
}

// ---- file:// URI helpers ----------------------------------------------------

/// Bytes left unencoded in the path portion of a `file://` URI.
fn is_uri_path_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':')
}

/// Strip a Windows verbatim (extended-length) prefix from a path string:
/// `\\?\C:\...` becomes `C:\...` and `\\?\UNC\server\share\...` becomes
/// `\\server\share\...`. Anything else is returned unchanged.
///
/// `Path::canonicalize` on Windows returns verbatim paths; feeding those into
/// a URI verbatim yields `file:////%3F/C:/...`, which real language servers
/// reject. Pure string logic so it is unit-testable on any platform.
fn strip_verbatim(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw.to_string()
    }
}

/// Strip the leading slash of a URI path like `/C:/Users/x` (the decoded path
/// component of `file:///C:/...`) so it becomes a Windows drive path. Pure
/// string logic, applied only on Windows but testable everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
fn strip_uri_drive_slash(text: String) -> String {
    let bytes = text.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        let mut text = text;
        text.remove(0);
        text
    } else {
        text
    }
}

/// Build a `file://` URI for an absolute path.
pub(crate) fn path_to_uri(path: &Path) -> String {
    let raw = strip_verbatim(&path.to_string_lossy()).replace('\\', "/");
    let mut encoded = String::with_capacity(raw.len() + 8);
    encoded.push_str("file://");
    if !raw.starts_with('/') {
        encoded.push('/'); // Windows drive paths: file:///C:/...
    }
    for &b in raw.as_bytes() {
        if is_uri_path_byte(b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    encoded
}

/// Parse a `file://` URI back into a filesystem path.
pub(crate) fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Skip an optional authority (host) component; only empty or localhost
    // authorities are meaningful for local files.
    let path_part = if let Some(stripped) = rest.strip_prefix('/') {
        // Empty authority: rest already was the path (put the slash back).
        let mut s = String::with_capacity(stripped.len() + 1);
        s.push('/');
        s.push_str(stripped);
        s
    } else {
        let (authority, path) = rest.split_once('/')?;
        if !authority.is_empty() && authority != "localhost" {
            return None;
        }
        format!("/{path}")
    };

    // Percent-decode.
    let bytes = path_part.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let text = String::from_utf8(decoded).ok()?;

    // file:///C:/x -> C:/x
    #[cfg(windows)]
    let text = strip_uri_drive_slash(text);

    Some(PathBuf::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_roundtrip_plain() {
        let path = Path::new("/tmp/project/src/main.kt");
        let uri = path_to_uri(path);
        assert_eq!(uri, "file:///tmp/project/src/main.kt");
        assert_eq!(uri_to_path(&uri).unwrap(), path);
    }

    #[test]
    fn uri_roundtrip_with_spaces_and_unicode() {
        let path = Path::new("/tmp/my project/söurce.kt");
        let uri = path_to_uri(path);
        assert!(uri.starts_with("file:///tmp/my%20project/"), "{uri}");
        assert_eq!(uri_to_path(&uri).unwrap(), path);
    }

    #[test]
    fn strip_verbatim_handles_drive_and_unc_prefixes() {
        assert_eq!(strip_verbatim(r"\\?\C:\Users\x"), r"C:\Users\x");
        assert_eq!(strip_verbatim(r"\\?\UNC\srv\share\x"), r"\\srv\share\x");
        // Non-verbatim inputs pass through untouched.
        assert_eq!(strip_verbatim(r"C:\Users\x"), r"C:\Users\x");
        assert_eq!(strip_verbatim("/tmp/project"), "/tmp/project");
    }

    #[test]
    fn uri_from_verbatim_drive_path_has_three_slashes() {
        // Windows canonicalize() yields \\?\C:\...; the URI must not leak
        // the verbatim prefix and must use the file:///C:/... form.
        let uri = path_to_uri(Path::new(r"\\?\C:\Users\x"));
        assert_eq!(uri, "file:///C:/Users/x");
        // Round trip (string logic; the leading-slash strip is applied under
        // cfg(windows) in uri_to_path).
        assert_eq!(
            strip_uri_drive_slash("/C:/Users/x".to_string()),
            "C:/Users/x"
        );
        // Unix absolute paths are never mistaken for drive paths.
        assert_eq!(strip_uri_drive_slash("/tmp/x".to_string()), "/tmp/x");
    }

    #[test]
    fn uri_from_verbatim_unc_path_drops_the_verbatim_prefix() {
        let uri = path_to_uri(Path::new(r"\\?\UNC\srv\share\x"));
        // `\\srv\share\x` -> `//srv/share/x` -> empty-authority file URI.
        assert_eq!(uri, "file:////srv/share/x");
        assert_eq!(uri_to_path(&uri).unwrap(), PathBuf::from("//srv/share/x"));
        assert!(!uri.contains("%3F"), "verbatim `?` must not leak: {uri}");
    }

    #[test]
    fn uri_to_path_rejects_remote_hosts() {
        assert!(uri_to_path("file://example.com/x/y").is_none());
        assert!(uri_to_path("https://example.com/x").is_none());
        assert_eq!(
            uri_to_path("file://localhost/x/y").unwrap(),
            PathBuf::from("/x/y")
        );
    }

    fn manager_with(toml_text: &str) -> LspManager {
        let cfg: LspConfig = toml::from_str(toml_text).unwrap();
        LspManager::from_config(Path::new("/tmp/w"), &cfg, false).unwrap()
    }

    #[test]
    fn from_config_is_none_without_lsp_blocks() {
        let cfg = LspConfig::default();
        assert!(LspManager::from_config(Path::new("/tmp/w"), &cfg, false).is_none());

        // Blocks that all fail validation also yield None.
        let cfg: LspConfig = toml::from_str(
            r#"
[lsp.kotlin]
command = ""
"#,
        )
        .unwrap();
        assert!(LspManager::from_config(Path::new("/tmp/w"), &cfg, false).is_none());
    }

    #[test]
    fn backend_for_honors_claims_and_falls_back_to_builtin() {
        let mgr = manager_with(
            r#"
[lsp.kotlin]
command = "kls"
extensions = ["kt"]
backend = "lsp"

[lsp.python]
command = "pyls"
backend = "hybrid"

[lsp.go]
command = "gopls"
backend = "tree-sitter"

[lsp.scala]
command = "metals"
extensions = ["scala"]
backend = "hybrid"
"#,
        );

        // Dynamic language, explicit lsp backend.
        assert_eq!(
            mgr.backend_for(Path::new("src/App.kt")),
            FileBackend::Lsp("kotlin".to_string())
        );
        // Builtin language, hybrid.
        assert_eq!(
            mgr.backend_for(Path::new("app/main.py")),
            FileBackend::Hybrid("python".to_string())
        );
        // Builtin language forced back to tree-sitter.
        assert_eq!(
            mgr.backend_for(Path::new("pkg/x.go")),
            FileBackend::TreeSitter
        );
        // Dynamic language with hybrid degrades to lsp (no builtin grammar).
        assert_eq!(
            mgr.backend_for(Path::new("core/Main.scala")),
            FileBackend::Lsp("scala".to_string())
        );
        // Unclaimed builtin extension keeps working.
        assert_eq!(
            mgr.backend_for(Path::new("src/lib.rs")),
            FileBackend::TreeSitter
        );
        // Unclaimed unknown extension stays unsupported.
        assert_eq!(
            mgr.backend_for(Path::new("README.adoc")),
            FileBackend::Unsupported
        );
        // Extension matching is case-insensitive.
        assert_eq!(
            mgr.backend_for(Path::new("src/App.KT")),
            FileBackend::Lsp("kotlin".to_string())
        );
    }
}
