//! Declarative LSP server registration (`[lsp.<language>]` in `.ctx/config.toml`).
//!
//! Users register any stdio language server per language:
//!
//! ```toml
//! [lsp.kotlin]
//! command = "kotlin-language-server"
//! extensions = ["kt", "kts"]
//! backend = "lsp"            # tree-sitter | lsp | hybrid (default hybrid)
//!
//! [lsp.python]
//! command = "pyright-langserver"
//! args = ["--stdio"]
//! # extensions default to the builtin set for builtin language names
//! ```
//!
//! Configuration is never fatal: invalid blocks are skipped with a warning so
//! `ctx index` keeps working with the remaining (or builtin) backends.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

/// The `[lsp.*]` sections of `.ctx/config.toml`, loaded independently of
/// [`crate::config::CtxConfig`] so the optional LSP subsystem never changes
/// that struct's public shape. Unknown keys are ignored so older binaries
/// tolerate newer config files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LspConfig {
    /// Language servers keyed by language name (`[lsp.kotlin]`,
    /// `[lsp.python]`, ...). Empty map when no LSP is configured — the LSP
    /// subsystem then never spawns anything.
    pub lsp: BTreeMap<String, LspServerConfig>,
}

impl LspConfig {
    /// Load `<root>/.ctx/config.toml`. A missing file yields defaults; a
    /// malformed file yields defaults with a warning (never fatal — config
    /// is optional, matching [`crate::config::CtxConfig::load`]).
    pub fn load(root: &Path) -> Self {
        Self::load_file(
            &root
                .join(crate::index::CTX_DIR)
                .join(crate::config::CONFIG_FILE),
        )
    }

    /// Load from an explicit path (used by tests and [`LspConfig::load`]).
    pub fn load_file(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(_) => return Self::default(), // absent/unreadable → defaults
        };
        match toml::from_str(&text) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Warning: ignoring malformed {} ({e})", path.display());
                Self::default()
            }
        }
    }
}

/// Per-language extraction backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LspBackend {
    /// Extract with tree-sitter only (LSP block is effectively disabled for
    /// extraction; useful to keep a registration around without using it).
    TreeSitter,
    /// Extract symbols and call edges with the language server.
    Lsp,
    /// Extract with tree-sitter, then use the language server to resolve
    /// cross-file references tree-sitter left unresolved.
    #[default]
    Hybrid,
}

impl LspBackend {
    /// Stable string form (matches the kebab-case config spelling).
    pub fn as_str(&self) -> &'static str {
        match self {
            LspBackend::TreeSitter => "tree-sitter",
            LspBackend::Lsp => "lsp",
            LspBackend::Hybrid => "hybrid",
        }
    }
}

/// One `[lsp.<language>]` block. All fields are optional except `command`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LspServerConfig {
    /// Executable to spawn (resolved via `PATH` unless absolute).
    pub command: String,
    /// Arguments passed to the server (e.g. `["--stdio"]`).
    pub args: Vec<String>,
    /// File extensions (without the dot) this server claims. Defaults to the
    /// builtin extension set when the block key is a builtin language name.
    pub extensions: Vec<String>,
    /// Marker files/dirs identifying a workspace root (informational; used by
    /// health checks).
    pub root_markers: Vec<String>,
    /// Capabilities the user expects the server to provide (e.g.
    /// `["documentSymbol", "definition"]`); compared against the negotiated
    /// server capabilities by the doctor probe.
    pub capabilities: Vec<String>,
    /// Extraction backend for files claimed by this block.
    pub backend: LspBackend,
    /// Passed through verbatim as LSP `initializationOptions`.
    pub initialization_options: Option<serde_json::Value>,
    /// Extra environment variables for the server process.
    pub env: BTreeMap<String, String>,
    /// Per-request timeout in milliseconds (default 10 000).
    pub timeout_ms: Option<u64>,
    /// Provenance metadata written by tooling (`ctx lsp add`); accepted and
    /// ignored so configs from newer binaries keep loading.
    pub source: Option<String>,
    /// Provenance metadata written by tooling; accepted and ignored.
    pub source_server: Option<String>,
}

/// Builtin extension sets, keyed by the builtin language name used as an
/// `[lsp.<name>]` table key (mirrors [`crate::parser::Language::from_path`]).
pub(crate) fn builtin_extensions(language: &str) -> Option<&'static [&'static str]> {
    match language {
        "rust" => Some(&["rs"]),
        "typescript" => Some(&["ts"]),
        "tsx" => Some(&["tsx"]),
        "javascript" => Some(&["js", "mjs", "cjs"]),
        "jsx" => Some(&["jsx"]),
        "python" => Some(&["py", "pyi"]),
        "go" => Some(&["go"]),
        "solidity" => Some(&["sol"]),
        "yaml" => Some(&["yaml", "yml"]),
        _ => None,
    }
}

/// Validate and normalize the raw `[lsp.*]` blocks from `.ctx/config.toml`.
///
/// Never fatal: invalid blocks are dropped with an `eprintln!` warning.
/// Returns the surviving blocks plus the extension → language claim map
/// (first block wins on duplicate extension claims, in table-key order).
pub(crate) fn validate(
    lsp: &BTreeMap<String, LspServerConfig>,
) -> (BTreeMap<String, LspServerConfig>, BTreeMap<String, String>) {
    let mut servers: BTreeMap<String, LspServerConfig> = BTreeMap::new();
    let mut extension_claims: BTreeMap<String, String> = BTreeMap::new();

    for (language, raw) in lsp {
        let mut cfg = raw.clone();

        if cfg.command.trim().is_empty() {
            eprintln!("Warning: ignoring [lsp.{language}] in .ctx/config.toml: `command` is empty");
            continue;
        }

        // Validation floor: a zero timeout would make every request fail
        // instantly; treat it as "not set" (the built-in default applies).
        if cfg.timeout_ms == Some(0) {
            eprintln!(
                "Warning: [lsp.{language}] timeout_ms = 0 is invalid; using the default timeout"
            );
            cfg.timeout_ms = None;
        }

        // Normalize extensions: lowercase, no leading dot, no empties.
        cfg.extensions = cfg
            .extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();

        if cfg.extensions.is_empty() {
            match builtin_extensions(language) {
                Some(defaults) => {
                    cfg.extensions = defaults.iter().map(|e| e.to_string()).collect();
                }
                None => {
                    eprintln!(
                        "Warning: ignoring [lsp.{language}] in .ctx/config.toml: \
                         `extensions` is required for non-builtin languages"
                    );
                    continue;
                }
            }
        }

        for ext in &cfg.extensions {
            match extension_claims.get(ext) {
                Some(owner) => {
                    eprintln!(
                        "Warning: [lsp.{language}] also claims extension `.{ext}`, \
                         already claimed by [lsp.{owner}]; first claim wins"
                    );
                }
                None => {
                    extension_claims.insert(ext.clone(), language.clone());
                }
            }
        }

        servers.insert(language.clone(), cfg);
    }

    (servers, extension_claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_text: &str) -> BTreeMap<String, LspServerConfig> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Wrapper {
            lsp: BTreeMap<String, LspServerConfig>,
        }
        toml::from_str::<Wrapper>(toml_text)
            .expect("valid toml")
            .lsp
    }

    #[test]
    fn defaults_and_backend_enum() {
        let lsp = parse(
            r#"
[lsp.kotlin]
command = "kotlin-language-server"
extensions = ["kt", "kts"]
backend = "lsp"

[lsp.python]
command = "pyright-langserver"
args = ["--stdio"]

[lsp.rust]
command = "rust-analyzer"
backend = "tree-sitter"
timeout_ms = 5000
"#,
        );
        assert_eq!(lsp["kotlin"].backend, LspBackend::Lsp);
        assert_eq!(
            lsp["python"].backend,
            LspBackend::Hybrid,
            "default is hybrid"
        );
        assert_eq!(lsp["rust"].backend, LspBackend::TreeSitter);
        assert_eq!(lsp["python"].args, vec!["--stdio"]);
        assert_eq!(lsp["rust"].timeout_ms, Some(5000));
        assert!(lsp["kotlin"].initialization_options.is_none());
        assert!(lsp["kotlin"].env.is_empty());

        let (servers, claims) = validate(&lsp);
        assert_eq!(servers.len(), 3);
        // Builtin names get builtin extension defaults.
        assert_eq!(servers["python"].extensions, vec!["py", "pyi"]);
        assert_eq!(servers["rust"].extensions, vec!["rs"]);
        assert_eq!(claims["kt"], "kotlin");
        assert_eq!(claims["py"], "python");
    }

    #[test]
    fn tolerates_provenance_keys_and_unknown_keys() {
        let lsp = parse(
            r#"
[lsp.kotlin]
command = "kls"
extensions = ["kt"]
source = "ctx lsp add"
source_server = "kotlin-language-server@1.3"
some_future_key = true
"#,
        );
        assert_eq!(lsp["kotlin"].command, "kls");
        assert_eq!(lsp["kotlin"].source.as_deref(), Some("ctx lsp add"));
        assert_eq!(
            lsp["kotlin"].source_server.as_deref(),
            Some("kotlin-language-server@1.3")
        );
    }

    #[test]
    fn empty_command_is_dropped_not_fatal() {
        let lsp = parse(
            r#"
[lsp.kotlin]
command = ""
extensions = ["kt"]
"#,
        );
        let (servers, claims) = validate(&lsp);
        assert!(servers.is_empty());
        assert!(claims.is_empty());
    }

    #[test]
    fn dynamic_language_without_extensions_is_dropped() {
        let lsp = parse(
            r#"
[lsp.kotlin]
command = "kls"
"#,
        );
        let (servers, _) = validate(&lsp);
        assert!(servers.is_empty());
    }

    #[test]
    fn zero_timeout_is_floored_to_default() {
        let lsp = parse(
            r#"
[lsp.python]
command = "pyls"
timeout_ms = 0
"#,
        );
        let (servers, _) = validate(&lsp);
        assert_eq!(
            servers["python"].timeout_ms, None,
            "0 falls back to default"
        );

        // Non-zero values pass through.
        let lsp = parse(
            r#"
[lsp.python]
command = "pyls"
timeout_ms = 200
"#,
        );
        let (servers, _) = validate(&lsp);
        assert_eq!(servers["python"].timeout_ms, Some(200));
    }

    #[test]
    fn duplicate_extension_claims_first_wins() {
        let lsp = parse(
            r#"
[lsp.alpha]
command = "a-ls"
extensions = ["zz"]

[lsp.beta]
command = "b-ls"
extensions = ["zz", "yy"]
"#,
        );
        let (servers, claims) = validate(&lsp);
        assert_eq!(servers.len(), 2);
        // BTreeMap order: alpha validates first and claims `.zz`.
        assert_eq!(claims["zz"], "alpha");
        assert_eq!(claims["yy"], "beta");
    }

    #[test]
    fn extensions_are_normalized() {
        let lsp = parse(
            r#"
[lsp.kotlin]
command = "kls"
extensions = [".KT", " kts "]
"#,
        );
        let (servers, claims) = validate(&lsp);
        assert_eq!(servers["kotlin"].extensions, vec!["kt", "kts"]);
        assert_eq!(claims["kt"], "kotlin");
        assert_eq!(claims["kts"], "kotlin");
    }

    fn write_temp(contents: &str) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        f
    }

    #[test]
    fn load_parses_lsp_sections_and_ignores_other_tables() {
        let f = write_temp(
            r#"
[embedding]
provider = "local"

[lsp.kotlin]
command = "kotlin-language-server"
extensions = ["kt", "kts"]
backend = "lsp"
timeout_ms = 15000
env = { JAVA_HOME = "/opt/java" }

[lsp.python]
command = "pyright-langserver"
args = ["--stdio"]
initialization_options = { python = { venvPath = ".venv" } }
"#,
        );
        let cfg = LspConfig::load_file(f.path());
        assert_eq!(cfg.lsp.len(), 2);

        let kotlin = &cfg.lsp["kotlin"];
        assert_eq!(kotlin.command, "kotlin-language-server");
        assert_eq!(kotlin.extensions, vec!["kt", "kts"]);
        assert_eq!(kotlin.backend, LspBackend::Lsp);
        assert_eq!(kotlin.timeout_ms, Some(15000));
        assert_eq!(kotlin.env["JAVA_HOME"], "/opt/java");

        let python = &cfg.lsp["python"];
        assert_eq!(python.args, vec!["--stdio"]);
        assert_eq!(python.backend, LspBackend::Hybrid, "default");
        assert!(python.initialization_options.is_some());
    }

    #[test]
    fn load_malformed_file_falls_back_to_defaults() {
        // A type error anywhere in the file keeps load fault-tolerant.
        let f = write_temp(
            r#"
[lsp.kotlin]
command = 42
"#,
        );
        assert!(LspConfig::load_file(f.path()).lsp.is_empty());

        let f = write_temp("this is not valid toml : : :");
        assert!(LspConfig::load_file(f.path()).lsp.is_empty());
    }

    #[test]
    fn load_missing_file_is_default() {
        assert!(
            LspConfig::load_file(std::path::Path::new("/nonexistent/config.toml"))
                .lsp
                .is_empty()
        );
    }
}
