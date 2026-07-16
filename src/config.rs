//! Project configuration loaded from `.ctx/config.toml`.
//!
//! An optional, committed TOML file that sets per-project defaults so teams
//! don't have to pass the same flags/env vars on every invocation. Currently it
//! configures the embedding backend; more sections can be added over time.
//! (The `[lsp.*]` sections of the same file are loaded separately by
//! [`crate::lsp::LspConfig`].)
//!
//! ```toml
//! [embedding]
//! provider = "ollama"            # local | openai | ollama
//! model = "qwen3-embedding:8b"   # provider-specific (Ollama/OpenAI model)
//! # host = "http://localhost:11434"  # Ollama only
//! ```
//!
//! Precedence for the resolved settings is always **CLI flag > environment
//! variable > this file > built-in default**, so the config never overrides an
//! explicit request.

use std::path::Path;

use serde::Deserialize;

use crate::embeddings::Provider;
use crate::index::CTX_DIR;

/// Config file name inside `.ctx/`.
pub const CONFIG_FILE: &str = "config.toml";

/// Top-level `.ctx/config.toml` contents. Unknown keys are ignored so older
/// binaries tolerate newer config files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CtxConfig {
    /// Embedding backend defaults.
    pub embedding: EmbeddingConfig,
}

/// `[embedding]` section: default provider and provider-specific settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    /// Default provider when `--provider`/`--openai` are not given.
    pub provider: Option<Provider>,
    /// Model name (Ollama/OpenAI). For Ollama this overrides the built-in
    /// default but is itself overridden by `OLLAMA_EMBED_MODEL`.
    pub model: Option<String>,
    /// Ollama host URL; overridden by `OLLAMA_HOST`.
    pub host: Option<String>,
}

impl CtxConfig {
    /// Load `<root>/.ctx/config.toml`. A missing file yields defaults; a malformed
    /// file yields defaults with a warning (never fatal — config is optional).
    pub fn load(root: &Path) -> Self {
        Self::load_file(&root.join(CTX_DIR).join(CONFIG_FILE))
    }

    /// Load from an explicit path (used by tests and [`CtxConfig::load`]).
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(contents: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        f
    }

    #[test]
    fn missing_file_is_default() {
        let cfg = CtxConfig::load_file(Path::new("/nonexistent/.ctx/config.toml"));
        assert!(cfg.embedding.provider.is_none());
        assert!(cfg.embedding.model.is_none());
    }

    #[test]
    fn parses_embedding_section() {
        let f = write_temp(
            r#"
[embedding]
provider = "ollama"
model = "qwen3-embedding:8b"
"#,
        );
        let cfg = CtxConfig::load_file(f.path());
        assert_eq!(cfg.embedding.provider, Some(Provider::Ollama));
        assert_eq!(cfg.embedding.model.as_deref(), Some("qwen3-embedding:8b"));
        assert!(cfg.embedding.host.is_none());
    }

    #[test]
    fn unknown_keys_ignored() {
        let f = write_temp(
            r#"
[embedding]
provider = "openai"

[future_section]
whatever = true
"#,
        );
        let cfg = CtxConfig::load_file(f.path());
        assert_eq!(cfg.embedding.provider, Some(Provider::Openai));
    }

    #[test]
    fn malformed_file_is_default() {
        let f = write_temp("this is not valid toml : : :");
        let cfg = CtxConfig::load_file(f.path());
        assert!(cfg.embedding.provider.is_none());
    }

    #[test]
    fn config_provides_resolution_default() {
        // Flag wins over config; config wins over built-in default.
        assert_eq!(
            Provider::resolve(Some(Provider::Local), false, Some(Provider::Ollama)),
            Provider::Local
        );
        assert_eq!(
            Provider::resolve(None, false, Some(Provider::Ollama)),
            Provider::Ollama
        );
        assert_eq!(Provider::resolve(None, false, None), Provider::Local);
        // Deprecated --openai still beats config.
        assert_eq!(
            Provider::resolve(None, true, Some(Provider::Ollama)),
            Provider::Openai
        );
    }
}
