//! Client for the community LSP server registry (`agentis-tools/ctx-lsp-registry`).
//!
//! The registry is a separate repository hosting curated language-server
//! entries as TOML. ctx fetches raw files over HTTPS from a pinned `v1`
//! branch:
//!
//! - `{base}/index.toml` — the language index ([`RegistryIndex`])
//! - `{base}/registry/{lang}.toml` — one entry per language ([`LanguageEntry`])
//!
//! The base URL can be overridden with the `CTX_LSP_REGISTRY_BASE_URL`
//! environment variable (a test hook, also usable for mirrors). It is
//! deliberately *not* a config key: the registry location is a distribution
//! concern, not a per-project setting.
//!
//! The `ctx lsp` commands (`add`, `list --available`, `update`) are the CLI
//! surface over this client.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{CtxError, Result};

/// Raw-file base URL of the pinned `v1` branch of the registry repository.
pub const DEFAULT_REGISTRY_BASE_URL: &str =
    "https://raw.githubusercontent.com/agentis-tools/ctx-lsp-registry/v1";

/// The registry manifest schema version this ctx build understands.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Network timeout for registry fetches (small TOML files).
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Registry base URL, overridable via `CTX_LSP_REGISTRY_BASE_URL` (test
/// hook / mirrors; deliberately not a config key).
pub fn registry_base_url() -> String {
    resolve_base_url(std::env::var("CTX_LSP_REGISTRY_BASE_URL").ok())
}

/// Pure resolution helper behind [`registry_base_url`], unit-testable without
/// mutating process environment.
fn resolve_base_url(env_override: Option<String>) -> String {
    match env_override {
        Some(url) if !url.trim().is_empty() => url,
        _ => DEFAULT_REGISTRY_BASE_URL.to_string(),
    }
}

fn http_client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("ctx/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(CtxError::Network)
}

// ============================================================================
// Manifest model
// ============================================================================

/// `index.toml`: the set of languages the registry covers.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistryIndex {
    /// Manifest schema version (must equal [`SUPPORTED_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Language name -> summary. `BTreeMap` keeps listings deterministic.
    #[serde(default)]
    pub languages: BTreeMap<String, IndexLanguage>,
}

/// One `[languages.<name>]` row in `index.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct IndexLanguage {
    /// Name of the recommended server for this language.
    pub recommended: String,
    /// All server names available in the language entry.
    #[serde(default)]
    pub servers: Vec<String>,
}

/// `registry/<lang>.toml`: full entry for one language.
#[derive(Debug, Clone, Deserialize)]
pub struct LanguageEntry {
    /// Manifest schema version (must equal [`SUPPORTED_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Language name (matches the file name and the index key).
    pub language: String,
    /// File extensions (without dots) the language servers apply to.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Project-root marker files used to locate the workspace root.
    #[serde(default)]
    pub root_markers: Vec<String>,
    /// Available servers; exactly one must be `recommended = true`.
    #[serde(default)]
    pub servers: Vec<ServerSpec>,
}

/// One `[[servers]]` block in a language entry.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerSpec {
    /// Unique server name within the language entry (e.g. `pyright`).
    pub name: String,
    /// Whether this is the curated default for the language.
    #[serde(default)]
    pub recommended: bool,
    /// Executable to spawn (must be non-empty).
    pub command: String,
    /// Arguments passed to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// LSP capabilities ctx relies on (e.g. `documentSymbol`, `references`).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Project homepage, if any.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Free-form curator notes.
    #[serde(default)]
    pub notes: Option<String>,
    /// Install hints keyed by OS, with a required default.
    #[serde(default)]
    pub install: Option<InstallHints>,
}

/// `[servers.install]`: per-OS install commands falling back to `default`.
#[derive(Debug, Clone, Deserialize)]
pub struct InstallHints {
    /// Fallback install command for any OS without a specific key.
    pub default: String,
    /// macOS-specific install command.
    #[serde(default)]
    pub macos: Option<String>,
    /// Linux-specific install command.
    #[serde(default)]
    pub linux: Option<String>,
    /// Windows-specific install command.
    #[serde(default)]
    pub windows: Option<String>,
}

impl LanguageEntry {
    /// Select a server by name, or the recommended one when `name` is `None`.
    pub fn server(&self, name: Option<&str>) -> Result<&ServerSpec> {
        match name {
            Some(name) => self.servers.iter().find(|s| s.name == name).ok_or_else(|| {
                CtxError::Other(format!(
                    "language '{}' has no server named '{name}' in the LSP registry \
                     (available: {})",
                    self.language,
                    self.server_names().join(", ")
                ))
            }),
            None => self.servers.iter().find(|s| s.recommended).ok_or_else(|| {
                CtxError::Other(format!(
                    "registry entry for '{}' has no recommended server",
                    self.language
                ))
            }),
        }
    }

    fn server_names(&self) -> Vec<&str> {
        self.servers.iter().map(|s| s.name.as_str()).collect()
    }

    /// Validate invariants beyond what serde enforces.
    fn validate(&self, what: &str) -> Result<()> {
        validate_schema_version(self.schema_version, what)?;
        let recommended = self.servers.iter().filter(|s| s.recommended).count();
        if recommended != 1 {
            return Err(CtxError::Other(format!(
                "{what} must mark exactly one server as recommended, found {recommended}"
            )));
        }
        for server in &self.servers {
            if server.name.trim().is_empty() {
                return Err(CtxError::Other(format!(
                    "{what} contains a server with an empty name"
                )));
            }
            if server.command.trim().is_empty() {
                return Err(CtxError::Other(format!(
                    "{what}: server '{}' has an empty command",
                    server.name
                )));
            }
        }
        Ok(())
    }
}

/// The install hint matching the current OS, falling back to `default`.
/// `None` when the server ships no install section at all.
pub fn install_hint_for_current_os(server: &ServerSpec) -> Option<&str> {
    install_hint_for_os(server, std::env::consts::OS)
}

fn install_hint_for_os<'a>(server: &'a ServerSpec, os: &str) -> Option<&'a str> {
    let install = server.install.as_ref()?;
    let os_specific = match os {
        "macos" => install.macos.as_deref(),
        "linux" => install.linux.as_deref(),
        "windows" => install.windows.as_deref(),
        _ => None,
    };
    Some(os_specific.unwrap_or(&install.default))
}

fn validate_schema_version(found: u32, what: &str) -> Result<()> {
    if found != SUPPORTED_SCHEMA_VERSION {
        return Err(CtxError::Other(format!(
            "{what} uses registry schema_version {found}, but this ctx build supports \
             version {SUPPORTED_SCHEMA_VERSION}; the registry entry requires a newer ctx — \
             upgrade ctx or pin the registry to a compatible branch"
        )));
    }
    Ok(())
}

// ============================================================================
// Parsing (pure; also used by the fetch functions below)
// ============================================================================

/// Parse and validate `index.toml` contents.
pub fn parse_index(text: &str) -> Result<RegistryIndex> {
    let index: RegistryIndex = toml::from_str(text)
        .map_err(|e| CtxError::Other(format!("malformed registry index.toml: {e}")))?;
    validate_schema_version(index.schema_version, "registry index.toml")?;
    Ok(index)
}

/// Parse and validate a `registry/<lang>.toml` entry.
pub fn parse_language(text: &str, lang: &str) -> Result<LanguageEntry> {
    let what = format!("registry entry for '{lang}'");
    let entry: LanguageEntry =
        toml::from_str(text).map_err(|e| CtxError::Other(format!("malformed {what}: {e}")))?;
    entry.validate(&what)?;
    Ok(entry)
}

// ============================================================================
// Fetching
// ============================================================================

/// Fetch and validate `{base}/index.toml`.
pub fn fetch_index(base: &str) -> Result<RegistryIndex> {
    let url = format!("{}/index.toml", base.trim_end_matches('/'));
    parse_index(&fetch_text(&url, "registry index")?)
}

/// Fetch and validate `{base}/registry/{lang}.toml`.
///
/// A 404 means the language is not in the registry; the error says so
/// explicitly (callers can print available languages from [`fetch_index`]).
pub fn fetch_language(base: &str, lang: &str) -> Result<LanguageEntry> {
    let url = format!("{}/registry/{lang}.toml", base.trim_end_matches('/'));
    let client = http_client(FETCH_TIMEOUT)?;
    let response = client.get(&url).send()?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(CtxError::Other(format!(
            "language '{lang}' is unknown to the LSP registry ({url} returned HTTP 404); \
             run against the registry index to list available languages"
        )));
    }
    if !status.is_success() {
        return Err(CtxError::Other(format!(
            "LSP registry fetch failed: {url} returned HTTP {status}"
        )));
    }
    parse_language(&response.text()?, lang)
}

fn fetch_text(url: &str, what: &str) -> Result<String> {
    let client = http_client(FETCH_TIMEOUT)?;
    let response = client.get(url).send()?;
    let status = response.status();
    if !status.is_success() {
        return Err(CtxError::Other(format!(
            "{what} fetch failed: {url} returned HTTP {status}"
        )));
    }
    Ok(response.text()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_INDEX: &str = r#"
schema_version = 1

[languages.python]
recommended = "pyright"
servers = ["pyright"]

[languages.go]
recommended = "gopls"
servers = ["gopls"]
"#;

    const GOOD_PYTHON: &str = r#"
schema_version = 1
language = "python"
extensions = ["py", "pyi"]
root_markers = ["pyproject.toml", "setup.py", "setup.cfg", ".git"]

[[servers]]
name = "pyright"
recommended = true
command = "pyright-langserver"
args = ["--stdio"]
capabilities = ["documentSymbol", "references", "callHierarchy"]
homepage = "https://github.com/microsoft/pyright"
notes = "Fast, actively maintained."

[servers.install]
default = "npm install -g pyright"
macos = "brew install pyright"

[[servers]]
name = "pylsp"
recommended = false
command = "pylsp"

[servers.install]
default = "pip install python-lsp-server"
"#;

    #[test]
    fn parses_good_index() {
        let index = parse_index(GOOD_INDEX).unwrap();
        assert_eq!(index.schema_version, 1);
        assert_eq!(index.languages.len(), 2);
        let python = &index.languages["python"];
        assert_eq!(python.recommended, "pyright");
        assert_eq!(python.servers, vec!["pyright"]);
    }

    #[test]
    fn parses_good_language_entry() {
        let entry = parse_language(GOOD_PYTHON, "python").unwrap();
        assert_eq!(entry.language, "python");
        assert_eq!(entry.extensions, vec!["py", "pyi"]);
        assert_eq!(entry.root_markers.len(), 4);
        assert_eq!(entry.servers.len(), 2);

        let recommended = entry.server(None).unwrap();
        assert_eq!(recommended.name, "pyright");
        assert_eq!(recommended.command, "pyright-langserver");
        assert_eq!(recommended.args, vec!["--stdio"]);
        assert_eq!(
            recommended.capabilities,
            vec!["documentSymbol", "references", "callHierarchy"]
        );

        let named = entry.server(Some("pylsp")).unwrap();
        assert_eq!(named.command, "pylsp");

        let err = entry.server(Some("nope")).unwrap_err().to_string();
        assert!(err.contains("no server named 'nope'"), "{err}");
        assert!(err.contains("pyright, pylsp"), "{err}");
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let text = GOOD_PYTHON.replace("schema_version = 1", "schema_version = 2");
        let err = parse_language(&text, "python").unwrap_err().to_string();
        assert!(err.contains("upgrade ctx"), "{err}");

        let text = GOOD_INDEX.replace("schema_version = 1", "schema_version = 99");
        let err = parse_index(&text).unwrap_err().to_string();
        assert!(err.contains("upgrade ctx"), "{err}");
    }

    #[test]
    fn rejects_zero_recommended_servers() {
        let text = GOOD_PYTHON.replace("recommended = true", "recommended = false");
        let err = parse_language(&text, "python").unwrap_err().to_string();
        assert!(err.contains("exactly one server"), "{err}");
        assert!(err.contains("found 0"), "{err}");
    }

    #[test]
    fn rejects_two_recommended_servers() {
        let text = GOOD_PYTHON.replace("recommended = false", "recommended = true");
        let err = parse_language(&text, "python").unwrap_err().to_string();
        assert!(err.contains("exactly one server"), "{err}");
        assert!(err.contains("found 2"), "{err}");
    }

    #[test]
    fn rejects_empty_command() {
        let text = GOOD_PYTHON.replace("command = \"pylsp\"", "command = \"\"");
        let err = parse_language(&text, "python").unwrap_err().to_string();
        assert!(err.contains("empty command"), "{err}");
    }

    #[test]
    fn rejects_missing_command() {
        let text = GOOD_PYTHON.replace("command = \"pylsp\"\n", "");
        let err = parse_language(&text, "python").unwrap_err().to_string();
        assert!(err.contains("malformed"), "{err}");
        assert!(err.contains("command"), "{err}");
    }

    #[test]
    fn install_hint_prefers_current_os_and_falls_back_to_default() {
        let entry = parse_language(GOOD_PYTHON, "python").unwrap();
        let pyright = entry.server(Some("pyright")).unwrap();
        assert_eq!(
            install_hint_for_os(pyright, "macos"),
            Some("brew install pyright")
        );
        assert_eq!(
            install_hint_for_os(pyright, "linux"),
            Some("npm install -g pyright")
        );
        assert_eq!(
            install_hint_for_os(pyright, "windows"),
            Some("npm install -g pyright")
        );
        // The current-OS wrapper resolves to one of the above.
        assert!(install_hint_for_current_os(pyright).is_some());

        let no_install = ServerSpec {
            name: "x".into(),
            recommended: true,
            command: "x".into(),
            args: vec![],
            capabilities: vec![],
            homepage: None,
            notes: None,
            install: None,
        };
        assert_eq!(install_hint_for_current_os(&no_install), None);
    }

    #[test]
    fn base_url_resolution() {
        assert_eq!(resolve_base_url(None), DEFAULT_REGISTRY_BASE_URL);
        assert_eq!(
            resolve_base_url(Some(String::new())),
            DEFAULT_REGISTRY_BASE_URL
        );
        assert_eq!(
            resolve_base_url(Some("http://127.0.0.1:9/mirror".to_string())),
            "http://127.0.0.1:9/mirror"
        );
    }
}
