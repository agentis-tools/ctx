//! Format-preserving writes to `.ctx/config.toml`.
//!
//! [`crate::config`] *reads* the project config with plain serde; this module
//! *writes* it using `toml_edit` so comments, formatting, and unknown keys the
//! current binary does not understand all survive round-trips.
//!
//! The only writer today manages `[lsp.<lang>]` tables installed from the
//! community LSP registry ([`crate::lsp_registry`]). Registry-owned tables are
//! marked with `source = "registry"` / `source_server = "<name>"` provenance
//! keys so `ctx lsp update` can tell curated entries apart from hand-written
//! ones. `ctx lsp add` and `ctx lsp update` are the CLI surface over this
//! module.

use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{value, Array, DocumentMut, Item, Table, TableLike};

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

/// Keys `ctx lsp add`/`ctx lsp update` manage in a `[lsp.<lang>]` table.
/// Anything else on the table is a user customization (`timeout_ms`, `env`,
/// `initialization_options`, ...) that tooling must never remove.
const CANONICAL_KEYS: [&str; 8] = [
    "command",
    "args",
    "extensions",
    "root_markers",
    "capabilities",
    "backend",
    "source",
    "source_server",
];

/// Insert or replace `[lsp.<lang>]` in `<root>/.ctx/config.toml`.
///
/// Creates `.ctx/` and the config file when absent. Everything outside the
/// touched table — comments, formatting, unknown keys, other `[lsp.*]`
/// tables — is preserved byte-for-byte. The file is written atomically
/// (temp file in `.ctx/` + rename). Used by the `ctx lsp add` path (fresh
/// entries); `ctx lsp update` uses [`refresh_lsp_entry`] instead so user
/// customizations on the table survive.
pub fn upsert_lsp_entry(root: &Path, lang: &str, entry: &LspConfigEntry) -> Result<()> {
    let path = config_path(root);
    fs::create_dir_all(path.parent().expect("config path has a parent"))?;
    let mut doc = load_document(&path)?;

    let lsp = lsp_table_mut(&mut doc, &path)?;
    lsp.insert(lang, Item::Table(entry_table(entry)));

    write_atomic(&path, doc.to_string().as_bytes())
}

/// Like [`upsert_lsp_entry`], but when `[lsp.<lang>]` already exists only
/// the canonical keys are (re)written: extra keys the user added to the
/// table (`timeout_ms`, `env`, `initialization_options`, ...) are left
/// untouched, as are their positions. Missing canonical keys are appended.
pub fn refresh_lsp_entry(root: &Path, lang: &str, entry: &LspConfigEntry) -> Result<()> {
    let path = config_path(root);
    fs::create_dir_all(path.parent().expect("config path has a parent"))?;
    let mut doc = load_document(&path)?;

    let lsp = lsp_table_mut(&mut doc, &path)?;
    match lsp.get_mut(lang).and_then(Item::as_table_like_mut) {
        Some(existing) => {
            let canonical = entry_table(entry);
            for (key, item) in canonical.iter() {
                existing.insert(key, item.clone());
            }
        }
        None => {
            lsp.insert(lang, Item::Table(entry_table(entry)));
        }
    }

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

/// Raw (`toml_edit`-level) ownership check for `[lsp.<lang>]`, so entries
/// serde cannot type (e.g. `command = 3`) are still visible to `ctx lsp add`
/// and are never silently replaced.
///
/// - `Ok(None)`: no config file, or no `[lsp.<lang>]` entry.
/// - `Ok(Some(true))`: the entry carries `source = "registry"` provenance.
/// - `Ok(Some(false))`: hand-written — no registry provenance, including
///   non-table values and type-broken tables.
pub fn lsp_entry_registry_owned(root: &Path, lang: &str) -> Result<Option<bool>> {
    let path = config_path(root);
    if !path.exists() {
        return Ok(None);
    }
    let doc = load_document(&path)?;
    let Some(item) = raw_lsp_item(&doc, lang) else {
        return Ok(None);
    };
    let source = item
        .as_table_like()
        .and_then(|t| t.get("source"))
        .and_then(Item::as_str);
    Ok(Some(source == Some(SOURCE_REGISTRY)))
}

/// Whether the raw `[lsp.<lang>]` table matches `entry` on the canonical
/// keys. Missing keys compare against defaults (empty string/array, backend
/// `"hybrid"`); a key of the wrong type never matches; extra user keys are
/// ignored. `Ok(false)` when the file or entry is absent.
pub fn lsp_entry_matches(root: &Path, lang: &str, entry: &LspConfigEntry) -> Result<bool> {
    let path = config_path(root);
    if !path.exists() {
        return Ok(false);
    }
    let doc = load_document(&path)?;
    let Some(table) = raw_lsp_item(&doc, lang).and_then(Item::as_table_like) else {
        return Ok(false);
    };
    Ok(
        raw_str(table, "command", "") == Some(entry.command.as_str())
            && raw_str(table, "backend", "hybrid") == Some(entry.backend.as_str())
            && raw_str(table, "source", "") == Some(entry.source.as_str())
            && raw_str(table, "source_server", "") == Some(entry.source_server.as_str())
            && raw_str_array(table, "args").as_deref() == Some(entry.args.as_slice())
            && raw_str_array(table, "extensions").as_deref() == Some(entry.extensions.as_slice())
            && raw_str_array(table, "root_markers").as_deref()
                == Some(entry.root_markers.as_slice())
            && raw_str_array(table, "capabilities").as_deref()
                == Some(entry.capabilities.as_slice()),
    )
}

/// Non-canonical keys present on the raw `[lsp.<lang>]` table — user
/// customizations [`refresh_lsp_entry`] preserves. Empty when the file or
/// entry is absent.
pub fn lsp_entry_extra_keys(root: &Path, lang: &str) -> Result<Vec<String>> {
    let path = config_path(root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let doc = load_document(&path)?;
    let Some(table) = raw_lsp_item(&doc, lang).and_then(Item::as_table_like) else {
        return Ok(Vec::new());
    };
    Ok(table
        .iter()
        .map(|(key, _)| key)
        .filter(|key| !CANONICAL_KEYS.contains(key))
        .map(str::to_string)
        .collect())
}

/// The raw `[lsp.<lang>]` item from the parsed document, if any.
fn raw_lsp_item<'a>(doc: &'a DocumentMut, lang: &str) -> Option<&'a Item> {
    doc.get("lsp")
        .and_then(Item::as_table_like)
        .and_then(|t| t.get(lang))
}

/// String value of `key`, or `default` when the key is absent.
/// `None` when the key exists with a non-string value.
fn raw_str<'a>(table: &'a dyn TableLike, key: &str, default: &'a str) -> Option<&'a str> {
    match table.get(key) {
        None => Some(default),
        Some(item) => item.as_str(),
    }
}

/// String-array value of `key`, or empty when the key is absent.
/// `None` when the key exists but is not an array of strings.
fn raw_str_array(table: &dyn TableLike, key: &str) -> Option<Vec<String>> {
    match table.get(key) {
        None => Some(Vec::new()),
        Some(item) => {
            let array = item.as_array()?;
            let mut out = Vec::with_capacity(array.len());
            for value in array {
                out.push(value.as_str()?.to_string());
            }
            Some(out)
        }
    }
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
    fn refresh_preserves_user_keys_and_updates_canonical() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"[lsp.python]
command = "pyright-langserver"
args = ["--stdio"]
source = "registry"
source_server = "pyright"
# tuned for the monorepo
timeout_ms = 15000
env = { JAVA_HOME = "/opt/java" }
"#,
        )
        .unwrap();

        let mut entry = sample_entry();
        entry.args = vec!["--stdio".to_string(), "--verbose".to_string()];
        refresh_lsp_entry(dir.path(), "python", &entry).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        // Canonical drift is applied...
        assert!(
            text.contains(r#"args = ["--stdio", "--verbose"]"#),
            "{text}"
        );
        // ...missing canonical keys are added...
        assert!(text.contains(r#"backend = "hybrid""#), "{text}");
        // ...and user customizations survive byte-for-byte.
        assert!(text.contains("timeout_ms = 15000\n"), "{text}");
        assert!(
            text.contains("env = { JAVA_HOME = \"/opt/java\" }\n"),
            "{text}"
        );
        assert!(text.contains("# tuned for the monorepo\n"), "{text}");
    }

    #[test]
    fn refresh_creates_entry_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        refresh_lsp_entry(dir.path(), "python", &sample_entry()).unwrap();
        let text = fs::read_to_string(config_path(dir.path())).unwrap();
        assert_eq!(text, PYTHON_TABLE);
    }

    #[test]
    fn raw_registry_ownership_sees_type_broken_entries() {
        let dir = tempfile::tempdir().unwrap();
        // No file at all.
        assert_eq!(
            lsp_entry_registry_owned(dir.path(), "python").unwrap(),
            None
        );

        let path = config_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        // Hand-written, serde-typable.
        fs::write(&path, "[lsp.python]\ncommand = \"my-pylsp\"\n").unwrap();
        assert_eq!(
            lsp_entry_registry_owned(dir.path(), "python").unwrap(),
            Some(false)
        );
        assert_eq!(lsp_entry_registry_owned(dir.path(), "go").unwrap(), None);

        // Hand-written, type-broken (valid TOML, invalid type): still seen.
        fs::write(&path, "[lsp.python]\ncommand = 3\n").unwrap();
        assert_eq!(
            lsp_entry_registry_owned(dir.path(), "python").unwrap(),
            Some(false)
        );

        // Registry-owned.
        upsert_lsp_entry(dir.path(), "python", &sample_entry()).unwrap();
        assert_eq!(
            lsp_entry_registry_owned(dir.path(), "python").unwrap(),
            Some(true)
        );
    }

    #[test]
    fn raw_entry_matching_ignores_extra_keys_and_rejects_drift() {
        let dir = tempfile::tempdir().unwrap();
        let entry = sample_entry();
        assert!(!lsp_entry_matches(dir.path(), "python", &entry).unwrap());

        upsert_lsp_entry(dir.path(), "python", &entry).unwrap();
        assert!(lsp_entry_matches(dir.path(), "python", &entry).unwrap());

        // Extra user keys do not break "identical".
        let path = config_path(dir.path());
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("timeout_ms = 15000\n");
        fs::write(&path, &text).unwrap();
        assert!(lsp_entry_matches(dir.path(), "python", &entry).unwrap());

        // Canonical drift is a mismatch.
        let mut drifted = entry.clone();
        drifted.args.push("--verbose".to_string());
        assert!(!lsp_entry_matches(dir.path(), "python", &drifted).unwrap());

        // A type-broken canonical key never matches.
        fs::write(&path, "[lsp.python]\ncommand = 3\nsource = \"registry\"\n").unwrap();
        assert!(!lsp_entry_matches(dir.path(), "python", &entry).unwrap());
    }

    #[test]
    fn extra_keys_lists_only_non_canonical_keys() {
        let dir = tempfile::tempdir().unwrap();
        assert!(lsp_entry_extra_keys(dir.path(), "python")
            .unwrap()
            .is_empty());

        upsert_lsp_entry(dir.path(), "python", &sample_entry()).unwrap();
        assert!(lsp_entry_extra_keys(dir.path(), "python")
            .unwrap()
            .is_empty());

        let path = config_path(dir.path());
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("timeout_ms = 15000\nenv = { JAVA_HOME = \"/opt/java\" }\n");
        fs::write(&path, &text).unwrap();
        assert_eq!(
            lsp_entry_extra_keys(dir.path(), "python").unwrap(),
            ["timeout_ms", "env"]
        );
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
