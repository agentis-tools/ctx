//! LSP run status sidecar (`.ctx/lsp_status.json`) and health-check probes.
//!
//! The sidecar records, per configured language, which server ran, what it
//! reported during `initialize`, and whether it stayed healthy — without
//! touching the SQLite schema. `.ctx/` is never indexed, so the sidecar never
//! shows up in query results.
//!
//! [`doctor`] powers the `ctx lsp doctor` command.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::index::CTX_DIR;

use super::client::LspClient;
use super::config::{self, LspServerConfig};

/// File name of the sidecar inside `.ctx/`.
pub const STATUS_FILE: &str = "lsp_status.json";

/// One language's entry in the status sidecar.
#[derive(Debug, Clone, Serialize)]
pub struct LspStatusEntry {
    /// Language key from the `[lsp.<language>]` block.
    pub language: String,
    /// Configured server command.
    pub command: String,
    /// Configured backend (`tree-sitter` | `lsp` | `hybrid`).
    pub backend: String,
    /// `healthy`, `failed`, or `idle` (configured but never needed this run).
    pub state: String,
    /// Failure reason when `state == "failed"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Server-reported name from `initialize`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    /// Server-reported version from `initialize`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    /// Negotiated capability names (truthy keys of the server's
    /// `capabilities` object).
    pub capabilities: Vec<String>,
}

#[derive(Serialize)]
struct StatusDocument<'a> {
    generated_at: u64,
    servers: &'a [LspStatusEntry],
}

/// Write the sidecar for this run (best effort; callers treat errors as
/// warnings).
pub(crate) fn write_status_file(root: &Path, entries: &[LspStatusEntry]) -> std::io::Result<()> {
    let ctx_dir = root.join(CTX_DIR);
    std::fs::create_dir_all(&ctx_dir)?;
    let doc = StatusDocument {
        generated_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        servers: entries,
    };
    let text = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
    std::fs::write(ctx_dir.join(STATUS_FILE), text)
}

/// Result of probing one configured server (consumed by `ctx lsp doctor`).
#[derive(Debug, Clone, Serialize)]
pub struct LspHealthReport {
    pub language: String,
    pub command: String,
    pub backend: String,
    /// The command resolves to an executable (PATH lookup or explicit path).
    pub binary_found: bool,
    /// Where the binary was found, when it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    /// Which configured `root_markers` exist under the project root.
    pub root_markers_found: Vec<String>,
    /// Spawn + `initialize` handshake succeeded.
    pub handshake_ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    /// Truthy capability keys negotiated during the probe.
    pub negotiated_capabilities: Vec<String>,
    /// Configured `capabilities` the server did NOT negotiate.
    pub missing_capabilities: Vec<String>,
    /// Last stderr lines captured from the probe process.
    pub stderr: Vec<String>,
    /// Spawn/handshake error, when any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Probe every configured server: PATH check, spawn + handshake, capability
/// diff, recent stderr. Never fatal; one report per valid `[lsp.*]` block.
pub fn doctor(root: &Path, lsp_config: &config::LspConfig) -> Vec<LspHealthReport> {
    let (servers, _) = config::validate(&lsp_config.lsp);
    servers
        .iter()
        .map(|(language, cfg)| probe_server(root, language, cfg))
        .collect()
}

fn probe_server(root: &Path, language: &str, cfg: &LspServerConfig) -> LspHealthReport {
    let binary_path = find_executable(&cfg.command);
    let root_markers_found = cfg
        .root_markers
        .iter()
        .filter(|marker| root.join(marker).exists())
        .cloned()
        .collect();

    let mut report = LspHealthReport {
        language: language.to_string(),
        command: cfg.command.clone(),
        backend: cfg.backend.as_str().to_string(),
        binary_found: binary_path.is_some(),
        binary_path: binary_path.map(|p| p.to_string_lossy().to_string()),
        root_markers_found,
        handshake_ok: false,
        server_name: None,
        server_version: None,
        negotiated_capabilities: Vec::new(),
        missing_capabilities: Vec::new(),
        stderr: Vec::new(),
        error: None,
    };

    if !report.binary_found {
        report.error = Some(format!("`{}` not found on PATH", cfg.command));
        report.missing_capabilities = cfg.capabilities.clone();
        return report;
    }

    match LspClient::spawn(cfg, root, false) {
        Ok(mut client) => {
            report.handshake_ok = true;
            report.server_name = client.server_name.clone();
            report.server_version = client.server_version.clone();
            report.negotiated_capabilities = client.capability_names();
            report.missing_capabilities = cfg
                .capabilities
                .iter()
                .filter(|c| !client.supports(&capability_key(c)))
                .cloned()
                .collect();
            report.stderr = client.recent_stderr();
            client.shutdown();
        }
        Err(e) => {
            report.error = Some(e.reason);
            report.stderr = e.stderr;
            report.missing_capabilities = cfg.capabilities.clone();
        }
    }

    report
}

/// Map a user-facing capability name to the `ServerCapabilities` key
/// (`"documentSymbol"` -> `"documentSymbolProvider"`). Full provider keys are
/// accepted as-is.
fn capability_key(name: &str) -> String {
    if name.ends_with("Provider") || name == "textDocumentSync" {
        name.to_string()
    } else {
        format!("{name}Provider")
    }
}

/// Resolve a command to an executable path (explicit path or PATH search).
/// Also used by `ctx lsp add` to warn when a freshly installed server's
/// binary is not available yet.
pub fn find_executable(command: &str) -> Option<PathBuf> {
    let candidate = Path::new(command);
    if candidate.components().count() > 1 {
        return if candidate.is_file() {
            Some(candidate.to_path_buf())
        } else {
            None
        };
    }

    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let full = dir.join(command);
        if full.is_file() {
            return Some(full);
        }
        #[cfg(windows)]
        {
            for ext in ["exe", "cmd", "bat"] {
                let with_ext = dir.join(format!("{command}.{ext}"));
                if with_ext.is_file() {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_key_mapping() {
        assert_eq!(capability_key("documentSymbol"), "documentSymbolProvider");
        assert_eq!(capability_key("definition"), "definitionProvider");
        assert_eq!(capability_key("callHierarchy"), "callHierarchyProvider");
        assert_eq!(
            capability_key("documentSymbolProvider"),
            "documentSymbolProvider"
        );
        assert_eq!(capability_key("textDocumentSync"), "textDocumentSync");
    }

    #[test]
    fn find_executable_resolves_common_binaries() {
        // `git` is a hard requirement of this repo's test suite already.
        assert!(find_executable("git").is_some());
        assert!(find_executable("definitely-not-on-path-xyz").is_none());
    }

    #[test]
    fn doctor_reports_missing_binary_without_spawning() {
        let cfg: config::LspConfig = toml::from_str(
            r#"
[lsp.kotlin]
command = "definitely-not-on-path-xyz"
extensions = ["kt"]
capabilities = ["documentSymbol", "definition"]
"#,
        )
        .unwrap();
        let reports = doctor(Path::new("/tmp"), &cfg);
        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert!(!report.binary_found);
        assert!(!report.handshake_ok);
        assert_eq!(
            report.missing_capabilities,
            vec!["documentSymbol", "definition"]
        );
        assert!(report.error.as_deref().unwrap().contains("not found"));
    }

    #[test]
    fn status_file_is_written_under_ctx_dir() {
        let temp = tempfile::tempdir().unwrap();
        let entries = vec![LspStatusEntry {
            language: "kotlin".into(),
            command: "kls".into(),
            backend: "lsp".into(),
            state: "healthy".into(),
            reason: None,
            server_name: Some("mock".into()),
            server_version: Some("1.0".into()),
            capabilities: vec!["documentSymbolProvider".into()],
        }];
        write_status_file(temp.path(), &entries).unwrap();

        let text = std::fs::read_to_string(temp.path().join(CTX_DIR).join(STATUS_FILE)).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["servers"][0]["language"], "kotlin");
        assert_eq!(doc["servers"][0]["state"], "healthy");
        assert_eq!(doc["servers"][0]["server_name"], "mock");
        assert!(doc["generated_at"].as_u64().unwrap() > 0);
    }
}
