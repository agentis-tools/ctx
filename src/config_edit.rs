//! Format-preserving writes to `.ctx/config.toml`.
//!
//! [`crate::config`] *reads* the project config with plain serde; this module
//! *writes* it using `toml_edit` so comments, formatting, and unknown keys the
//! current binary does not understand all survive round-trips.
//!
//! The only writer today manages `[lsp.<lang>]` tables installed from the
//! community LSP registry ([`crate::lsp_registry`]). Registry-owned tables are
//! marked with `source = "registry"` / `source_server = "<name>"` provenance
//! keys so a future `ctx lsp update` can tell curated entries apart from
//! hand-written ones.
//!
//! This module is internal groundwork for the future `ctx lsp` commands; it
//! has no CLI surface yet.

use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{value, Array, DocumentMut, Item, Table};

use crate::config::CONFIG_FILE;
use crate::error::{CtxError, Result};
use crate::index::CTX_DIR;
use crate::lsp_registry::{LanguageEntry, ServerSpec};

/// Provenance value marking a `[lsp.<lang>]` table as registry-managed.
pub const SOURCE_REGISTRY: &str = "registry";

/// One `[lsp.<lang>]` table as written to `.ctx/config.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspConfigEntry {
    /// Language-server executable.
    pub command: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// File extensions (without dots) the server applies to.
    pub extensions: Vec<String>,
    /// Project-root marker files.
    pub root_markers: Vec<String>,
    /// LSP capabilities ctx relies on.
    pub capabilities: Vec<String>,
    /// Intelligence backend; defaults to `"hybrid"` (tree-sitter + LSP).
    pub backend: String,
    /// Provenance: `"registry"` for registry-installed entries.
    pub source: String,
    /// Provenance: registry server name the entry was installed from.
    pub source_server: String,
}

impl Default for LspConfigEntry {
    fn default() -> Self {
        LspConfigEntry {
            command: String::new(),
            args: Vec::new(),
            extensions: Vec::new(),
            root_markers: Vec::new(),
            capabilities: Vec::new(),
            backend: "hybrid".to_string(),
            source: String::new(),
            source_server: String::new(),
        }
    }
}

/// Map a registry language entry + chosen server to a config entry
/// (backend `"hybrid"`, provenance `source = "registry"`).
pub fn from_registry(entry: &LanguageEntry, server: &ServerSpec) -> LspConfigEntry {
    LspConfigEntry {
        command: server.command.clone(),
        args: server.args.clone(),
        extensions: entry.extensions.clone(),
        root_markers: entry.root_markers.clone(),
        capabilities: server.capabilities.clone(),
        backend: "hybrid".to_string(),
        source: SOURCE_REGISTRY.to_string(),
        source_server: server.name.clone(),
    }
}

/// Insert or replace `[lsp.<lang>]` in `<root>/.ctx/config.toml`.
///
/// Creates `.ctx/` and the config file when absent. Everything outside the
/// touched table — comments, formatting, unknown keys, other `[lsp.*]`
/// tables — is preserved byte-for-byte. The file is written atomically
/// (temp file in `.ctx/` + rename).
pub fn upsert_lsp_entry(root: &Path, lang: &str, entry: &LspConfigEntry) -> Result<()> {
    let path = config_path(root);
    fs::create_dir_all(path.parent().expect("config path has a parent"))?;
    let mut doc = load_document(&path)?;

    let lsp = lsp_table_mut(&mut doc, &path)?;
    lsp.insert(lang, Item::Table(entry_table(entry)));

    write_atomic(&path, doc.to_string().as_bytes())
}

/// Remove `[lsp.<lang>]` from `<root>/.ctx/config.toml` if present.
/// Returns `true` when an entry was removed. A missing file is a no-op.
pub fn remove_lsp_entry(root: &Path, lang: &str) -> Result<bool> {
    let path = config_path(root);
    if !path.exists() {
        return Ok(false);
    }
    let mut doc = load_document(&path)?;
    let removed = match doc.get_mut("lsp").and_then(Item::as_table_mut) {
        Some(lsp) => lsp.remove(lang).is_some(),
        None => false,
    };
    if removed {
        write_atomic(&path, doc.to_string().as_bytes())?;
    }
    Ok(removed)
}

/// Languages under `[lsp]` whose tables are registry-managed
/// (`source = "registry"`), in file order. Missing file yields an empty list.
pub fn registry_owned_languages(root: &Path) -> Result<Vec<String>> {
    let path = config_path(root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let doc = load_document(&path)?;
    let mut owned = Vec::new();
    if let Some(lsp) = doc.get("lsp").and_then(Item::as_table) {
        for (lang, item) in lsp.iter() {
            let source = item
                .as_table_like()
                .and_then(|t| t.get("source"))
                .and_then(Item::as_str);
            if source == Some(SOURCE_REGISTRY) {
                owned.push(lang.to_string());
            }
        }
    }
    Ok(owned)
}

fn config_path(root: &Path) -> PathBuf {
    root.join(CTX_DIR).join(CONFIG_FILE)
}

fn load_document(path: &Path) -> Result<DocumentMut> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    text.parse::<DocumentMut>().map_err(|e| {
        CtxError::Other(format!(
            "cannot edit {}: file is not valid TOML ({e})",
            path.display()
        ))
    })
}

/// Get (or create) the `[lsp]` table, kept implicit so only `[lsp.<lang>]`
/// headers appear in the file.
fn lsp_table_mut<'a>(doc: &'a mut DocumentMut, path: &Path) -> Result<&'a mut Table> {
    let item = doc.entry("lsp").or_insert(Item::Table(Table::new()));
    let table = item.as_table_mut().ok_or_else(|| {
        CtxError::Other(format!(
            "cannot edit {}: existing 'lsp' key is not a table",
            path.display()
        ))
    })?;
    table.set_implicit(true);
    Ok(table)
}

/// Build the `[lsp.<lang>]` table with the canonical key order.
fn entry_table(entry: &LspConfigEntry) -> Table {
    let mut table = Table::new();
    table["command"] = value(&entry.command);
    table["args"] = string_array(&entry.args);
    table["extensions"] = string_array(&entry.extensions);
    table["root_markers"] = string_array(&entry.root_markers);
    table["capabilities"] = string_array(&entry.capabilities);
    table["backend"] = value(&entry.backend);
    table["source"] = value(&entry.source);
    table["source_server"] = value(&entry.source_server);
    table
}

fn string_array(items: &[String]) -> Item {
    let mut array = Array::new();
    for item in items {
        array.push(item.as_str());
    }
    value(array)
}

/// Atomically replace `path` with `data`: write to a temp file in the same
/// directory, then rename over the target (pattern shared with the
/// self-update binary swap).
fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| CtxError::Other("config path has no parent directory".to_string()))?;
    let staged = dir.join(format!(".config-toml-staged-{}", std::process::id()));
    if let Err(e) = fs::write(&staged, data) {
        let _ = fs::remove_file(&staged);
        return Err(e.into());
    }
    // On Windows, rename cannot replace an existing file.
    if cfg!(windows) && path.exists() {
        let _ = fs::remove_file(path);
    }
    if let Err(e) = fs::rename(&staged, path) {
        let _ = fs::remove_file(&staged);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry() -> LspConfigEntry {
        LspConfigEntry {
            command: "pyright-langserver".to_string(),
            args: vec!["--stdio".to_string()],
            extensions: vec!["py".to_string(), "pyi".to_string()],
            root_markers: vec!["pyproject.toml".to_string(), ".git".to_string()],
            capabilities: vec!["documentSymbol".to_string(), "references".to_string()],
            source_server: "pyright".to_string(),
            source: SOURCE_REGISTRY.to_string(),
            ..LspConfigEntry::default()
        }
    }

    const PYTHON_TABLE: &str = r#"[lsp.python]
command = "pyright-langserver"
args = ["--stdio"]
extensions = ["py", "pyi"]
root_markers = ["pyproject.toml", ".git"]
capabilities = ["documentSymbol", "references"]
backend = "hybrid"
source = "registry"
source_server = "pyright"
"#;

    #[test]
    fn creates_config_from_scratch() {
        let dir = tempfile::tempdir().unwrap();
        upsert_lsp_entry(dir.path(), "python", &sample_entry()).unwrap();
        let text = fs::read_to_string(config_path(dir.path())).unwrap();
        assert_eq!(text, PYTHON_TABLE);
    }

    #[test]
    fn preserves_comments_and_unrelated_tables() {
        let existing = r#"# Team defaults — do not remove.
[embedding]
provider = "ollama"   # local | openai | ollama
model = "qwen3-embedding:8b"

[future_section]
whatever = true
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, existing).unwrap();

        upsert_lsp_entry(dir.path(), "python", &sample_entry()).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        // Everything outside the touched table is preserved byte-for-byte.
        assert_eq!(text, format!("{existing}\n{PYTHON_TABLE}"));
    }

    #[test]
    fn overwrites_existing_entry_with_canonical_key_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "[lsp.python]\nsource = \"registry\"\ncommand = \"old-server\"\nstale_key = 1\n",
        )
        .unwrap();

        upsert_lsp_entry(dir.path(), "python", &sample_entry()).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text, PYTHON_TABLE);
        assert!(!text.contains("old-server"));
        assert!(!text.contains("stale_key"));
    }

    #[test]
    fn upsert_keeps_other_lsp_tables() {
        let dir = tempfile::tempdir().unwrap();
        let mut go = sample_entry();
        go.command = "gopls".to_string();
        go.args = vec![];
        go.source_server = "gopls".to_string();
        upsert_lsp_entry(dir.path(), "go", &go).unwrap();
        upsert_lsp_entry(dir.path(), "python", &sample_entry()).unwrap();

        let text = fs::read_to_string(config_path(dir.path())).unwrap();
        assert!(text.contains("[lsp.go]"));
        assert!(text.contains("[lsp.python]"));
        // No explicit bare [lsp] header is emitted.
        assert!(!text.contains("[lsp]\n"));
    }

    #[test]
    fn registry_owned_languages_filters_on_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"[lsp.python]
command = "pyright-langserver"
source = "registry"

[lsp.go]
command = "gopls"

[lsp.rust]
command = "rust-analyzer"
source = "manual"
"#,
        )
        .unwrap();

        assert_eq!(registry_owned_languages(dir.path()).unwrap(), ["python"]);
    }

    #[test]
    fn registry_owned_languages_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(registry_owned_languages(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn remove_lsp_entry_removes_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        upsert_lsp_entry(dir.path(), "python", &sample_entry()).unwrap();
        assert!(remove_lsp_entry(dir.path(), "python").unwrap());
        assert!(!remove_lsp_entry(dir.path(), "python").unwrap());
        let text = fs::read_to_string(config_path(dir.path())).unwrap();
        assert!(!text.contains("[lsp.python]"));
        // Missing file is a no-op.
        let empty = tempfile::tempdir().unwrap();
        assert!(!remove_lsp_entry(empty.path(), "python").unwrap());
    }

    #[test]
    fn write_is_atomic_no_temp_file_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        upsert_lsp_entry(dir.path(), "python", &sample_entry()).unwrap();
        let ctx_dir = dir.path().join(CTX_DIR);
        let leftovers: Vec<_> = fs::read_dir(&ctx_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".config-toml-staged-"))
            .collect();
        assert!(leftovers.is_empty(), "staged files left: {leftovers:?}");
        assert!(ctx_dir.join(CONFIG_FILE).exists());
    }

    #[test]
    fn malformed_existing_config_is_an_error_not_a_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "this is not : valid toml : :").unwrap();

        let err = upsert_lsp_entry(dir.path(), "python", &sample_entry()).unwrap_err();
        assert!(err.to_string().contains("not valid TOML"), "{err}");
        // Original contents untouched.
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "this is not : valid toml : :"
        );
    }

    #[test]
    fn non_table_lsp_key_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "lsp = 3\n").unwrap();

        let err = upsert_lsp_entry(dir.path(), "python", &sample_entry()).unwrap_err();
        assert!(err.to_string().contains("not a table"), "{err}");
    }
}
