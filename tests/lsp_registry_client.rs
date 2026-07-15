//! Integration tests for the LSP registry client + config writer pipeline:
//! fetch manifests from a local mock HTTP server, map the chosen server to a
//! config entry, and upsert it into a temp `.ctx/config.toml`.
//!
//! No CLI surface exists yet (it lands in a later PR), so these tests
//! exercise the library functions directly, passing the mock server's URL as
//! the explicit base. The `CTX_LSP_REGISTRY_BASE_URL` env override itself is
//! covered by a unit test on the pure resolution helper in `lsp_registry`.

use std::fs;

use ctx::config_edit::{from_registry, registry_owned_languages, upsert_lsp_entry};
use ctx::lsp_registry::{fetch_index, fetch_language, install_hint_for_current_os};
use ctx::testutil::MockServer;

const INDEX_TOML: &str = r#"
schema_version = 1

[languages.python]
recommended = "pyright"
servers = ["pyright"]
"#;

const PYTHON_TOML: &str = r#"
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
"#;

fn registry_server() -> MockServer {
    let server = MockServer::start();
    server.add_route("/index.toml", "text/plain; charset=utf-8", INDEX_TOML);
    server.add_route(
        "/registry/python.toml",
        "text/plain; charset=utf-8",
        PYTHON_TOML,
    );
    server
}

#[test]
fn fetch_from_registry_and_write_config() {
    let server = registry_server();
    let base = server.url();

    // Index lists python and its recommended server.
    let index = fetch_index(&base).unwrap();
    assert_eq!(index.languages.len(), 1);
    assert_eq!(index.languages["python"].recommended, "pyright");

    // Language entry resolves the recommended server.
    let entry = fetch_language(&base, "python").unwrap();
    let spec = entry.server(None).unwrap();
    assert_eq!(spec.name, "pyright");
    assert!(install_hint_for_current_os(spec).is_some());

    // Map to a config entry and write it into a fresh .ctx/config.toml.
    let dir = tempfile::tempdir().unwrap();
    let config = from_registry(&entry, spec);
    upsert_lsp_entry(dir.path(), &entry.language, &config).unwrap();

    let text = fs::read_to_string(dir.path().join(".ctx/config.toml")).unwrap();
    assert_eq!(
        text,
        r#"[lsp.python]
command = "pyright-langserver"
args = ["--stdio"]
extensions = ["py", "pyi"]
root_markers = ["pyproject.toml", "setup.py", "setup.cfg", ".git"]
capabilities = ["documentSymbol", "references", "callHierarchy"]
backend = "hybrid"
source = "registry"
source_server = "pyright"
"#
    );

    // The written table is recognized as registry-owned.
    assert_eq!(registry_owned_languages(dir.path()).unwrap(), ["python"]);
    assert_eq!(server.hits(), 2);
}

#[test]
fn unknown_language_404_names_the_language() {
    let server = registry_server();
    let err = fetch_language(&server.url(), "cobol")
        .unwrap_err()
        .to_string();
    assert!(err.contains("'cobol'"), "{err}");
    assert!(err.contains("unknown"), "{err}");
}

#[test]
fn missing_index_is_an_error() {
    let server = MockServer::start(); // no routes at all
    let err = fetch_index(&server.url()).unwrap_err().to_string();
    assert!(err.contains("404"), "{err}");
}

#[test]
fn malformed_manifest_is_an_error() {
    let server = MockServer::start();
    server.add_route("/index.toml", "text/plain", "not valid = = toml");
    server.add_route("/registry/python.toml", "text/plain", "also not : toml");

    let err = fetch_index(&server.url()).unwrap_err().to_string();
    assert!(err.contains("malformed"), "{err}");

    let err = fetch_language(&server.url(), "python")
        .unwrap_err()
        .to_string();
    assert!(err.contains("malformed"), "{err}");
}

#[test]
fn wrong_schema_version_says_upgrade() {
    let server = MockServer::start();
    server.add_route(
        "/index.toml",
        "text/plain",
        INDEX_TOML.replace("schema_version = 1", "schema_version = 2"),
    );
    server.add_route(
        "/registry/python.toml",
        "text/plain",
        PYTHON_TOML.replace("schema_version = 1", "schema_version = 2"),
    );

    let err = fetch_index(&server.url()).unwrap_err().to_string();
    assert!(err.contains("upgrade"), "{err}");

    let err = fetch_language(&server.url(), "python")
        .unwrap_err()
        .to_string();
    assert!(err.contains("upgrade"), "{err}");
}
