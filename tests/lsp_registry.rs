//! End-to-end CLI tests for `ctx lsp add|list|update|doctor`.
//!
//! A local [`MockServer`] plays the community LSP registry (`/index.toml`,
//! `/registry/python.toml`) and `CTX_LSP_REGISTRY_BASE_URL` points the child
//! ctx process at it. The doctor tests reuse the scripted mock language
//! server built into the ctx binary (`CTX_INTERNAL_MOCK_LSP`, see
//! `src/lsp/mock.rs` and `tests/lsp_cli.rs`).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use ctx::testutil::MockServer;

const INDEX_TOML: &str = r#"
schema_version = 1

[languages.python]
recommended = "pyright"
servers = ["pyright"]

[languages.go]
recommended = "gopls"
servers = ["gopls"]
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

/// Drifted variant of `PYTHON_TOML`: the curated args gained `--verbose`.
const PYTHON_TOML_DRIFTED: &str = r#"
schema_version = 1
language = "python"
extensions = ["py", "pyi"]
root_markers = ["pyproject.toml", "setup.py", "setup.cfg", ".git"]

[[servers]]
name = "pyright"
recommended = true
command = "pyright-langserver"
args = ["--stdio", "--verbose"]
capabilities = ["documentSymbol", "references", "callHierarchy"]
homepage = "https://github.com/microsoft/pyright"

[servers.install]
default = "npm install -g pyright"
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

/// Run the ctx binary in `dir` against the mock registry at `base`.
fn ctx(dir: &Path, base: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args(args)
        .current_dir(dir)
        .env("CTX_LSP_REGISTRY_BASE_URL", base)
        .env("CTX_NO_UPDATE_CHECK", "1")
        .output()
        .expect("failed to run ctx binary")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn config_text(root: &Path) -> String {
    fs::read_to_string(root.join(".ctx/config.toml")).expect("config file exists")
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Parse the single JSON envelope a `--json` invocation must print.
fn parse_envelope(out: &Output, command: &str) -> serde_json::Value {
    let text = stdout(out);
    let value: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}): {text}"));
    assert_eq!(value["command"], command, "envelope: {value}");
    value
}

// ============================================================================
// add
// ============================================================================

#[test]
fn add_with_yes_writes_registry_entry_and_is_idempotent() {
    let server = registry_server();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let out = ctx(root, &server.url(), &["lsp", "add", "python", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let text = config_text(root);
    assert!(text.contains("[lsp.python]"), "config: {text}");
    assert!(text.contains("command = \"pyright-langserver\""), "{text}");
    assert!(text.contains("backend = \"hybrid\""), "{text}");
    assert!(text.contains("source = \"registry\""), "{text}");
    assert!(text.contains("source_server = \"pyright\""), "{text}");
    let output = stdout(&out);
    assert!(output.contains("wrote [lsp.python]"), "stdout: {output}");
    assert!(output.contains("ctx index"), "stdout: {output}");
    // pyright-langserver is not installed in CI: the install hint is
    // surfaced as a warning instead of blocking the write.
    if !stderr(&out).is_empty() {
        assert!(
            stderr(&out).contains("pyright-langserver"),
            "{}",
            stderr(&out)
        );
    }

    // Second identical add: exit 0, nothing rewritten.
    let before = config_text(root);
    let out = ctx(root, &server.url(), &["lsp", "add", "python", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("already configured"),
        "stdout: {}",
        stdout(&out)
    );
    assert_eq!(config_text(root), before);
}

#[test]
fn add_refuses_manually_configured_language() {
    let server = registry_server();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let manual = "[lsp.python]\ncommand = \"my-pylsp\"\n";
    write(root, ".ctx/config.toml", manual);

    let out = ctx(root, &server.url(), &["lsp", "add", "python", "--yes"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("not registry-managed"), "stderr: {err}");
    // The hand-written entry is untouched.
    assert_eq!(config_text(root), manual);
    // Refused before any network call.
    assert_eq!(server.hits(), 0);
}

#[test]
fn add_refuses_type_broken_hand_written_entry() {
    let server = registry_server();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // Valid TOML, invalid type: the serde loader falls back to empty
    // defaults, but the raw-document ownership check must still see the
    // hand-written entry and refuse to touch it.
    let broken = "[lsp.python]\ncommand = 3\n";
    write(root, ".ctx/config.toml", broken);

    let out = ctx(root, &server.url(), &["lsp", "add", "python", "--yes"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("not registry-managed"), "stderr: {err}");
    assert!(err.contains("fix or remove"), "stderr: {err}");
    // The type-broken entry is byte-identical.
    assert_eq!(config_text(root), broken);
    // Refused before any network call.
    assert_eq!(server.hits(), 0);
}

#[test]
fn add_over_drifted_registry_entry_suggests_removal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let original = registry_server();
    let out = ctx(root, &original.url(), &["lsp", "add", "python", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let before = config_text(root);

    // Same server, drifted registry content: re-running add must not offer
    // the dead-end 'ctx lsp update' hint (update keys off source_server),
    // but point at removing the entry and re-adding with --server.
    let drifted = MockServer::start();
    drifted.add_route("/index.toml", "text/plain; charset=utf-8", INDEX_TOML);
    drifted.add_route(
        "/registry/python.toml",
        "text/plain; charset=utf-8",
        PYTHON_TOML_DRIFTED,
    );
    let out = ctx(root, &drifted.url(), &["lsp", "add", "python", "--yes"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("remove the entry"), "stderr: {err}");
    assert!(err.contains("--server pyright"), "stderr: {err}");
    assert!(!err.contains("ctx lsp update"), "stderr: {err}");
    assert_eq!(config_text(root), before);
}

#[test]
fn add_unknown_language_with_empty_registry_prints_none() {
    let server = MockServer::start();
    server.add_route(
        "/index.toml",
        "text/plain; charset=utf-8",
        "schema_version = 1\n",
    );
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let out = ctx(root, &server.url(), &["lsp", "add", "cobol", "--yes"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("available languages: (none)"), "stderr: {err}");
}

#[test]
fn add_without_yes_and_no_tty_exits_2() {
    let server = registry_server();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // Child stdin is closed (not a TTY): the prompt must refuse, not hang.
    let out = ctx(root, &server.url(), &["lsp", "add", "python"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("--yes"), "stderr: {}", stderr(&out));
    assert!(!root.join(".ctx/config.toml").exists());

    // JSON mode never prompts either.
    let out = ctx(root, &server.url(), &["--json", "lsp", "add", "python"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("--yes"), "stderr: {}", stderr(&out));
    assert!(!root.join(".ctx/config.toml").exists());
}

#[test]
fn add_unknown_language_lists_available_languages() {
    let server = registry_server();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let out = ctx(root, &server.url(), &["lsp", "add", "cobol", "--yes"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("'cobol'"), "stderr: {err}");
    assert!(err.contains("python"), "stderr: {err}");
    assert!(err.contains("go"), "stderr: {err}");
}

#[test]
fn add_json_reports_what_was_written() {
    let server = registry_server();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let out = ctx(
        root,
        &server.url(),
        &["--json", "lsp", "add", "python", "--yes"],
    );
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let value = parse_envelope(&out, "lsp.add");
    assert_eq!(value["data"]["status"], "added");
    assert_eq!(value["data"]["language"], "python");
    assert_eq!(value["data"]["server"], "pyright");
    assert_eq!(value["data"]["command"], "pyright-langserver");
    assert_eq!(value["data"]["backend"], "hybrid");
    assert!(config_text(root).contains("[lsp.python]"));
}

// ============================================================================
// list
// ============================================================================

#[test]
fn list_reports_empty_and_configured_states() {
    let server = registry_server();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // Empty state is friendly.
    let out = ctx(root, &server.url(), &["lsp", "list"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout(&out).contains("no LSP servers configured"),
        "stdout: {}",
        stdout(&out)
    );

    // Configured state shows language, command, backend, and provenance.
    let out = ctx(root, &server.url(), &["lsp", "add", "python", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let out = ctx(root, &server.url(), &["lsp", "list"]);
    assert_eq!(out.status.code(), Some(0));
    let output = stdout(&out);
    assert!(output.contains("python"), "stdout: {output}");
    assert!(output.contains("pyright-langserver --stdio"), "{output}");
    assert!(output.contains("hybrid"), "{output}");
    assert!(output.contains("registry (pyright)"), "{output}");

    // JSON variant parses and carries the same data.
    let out = ctx(root, &server.url(), &["--json", "lsp", "list"]);
    assert_eq!(out.status.code(), Some(0));
    let value = parse_envelope(&out, "lsp.list");
    let servers = value["data"]["servers"].as_array().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["language"], "python");
    assert_eq!(servers[0]["command"], "pyright-langserver");
    assert_eq!(servers[0]["backend"], "hybrid");
    assert_eq!(servers[0]["source"], "registry");
}

#[test]
fn list_available_marks_configured_languages() {
    let server = registry_server();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let out = ctx(root, &server.url(), &["lsp", "add", "python", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));

    let out = ctx(root, &server.url(), &["lsp", "list", "--available"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let output = stdout(&out);
    assert!(output.contains("pyright"), "stdout: {output}");
    assert!(output.contains("gopls"), "stdout: {output}");
    // python is configured, go is not.
    for line in output.lines() {
        if line.contains("python") {
            assert!(line.contains("[configured]"), "line: {line}");
        }
        if line.contains("  go ") || line.trim_start().starts_with("go ") {
            assert!(!line.contains("[configured]"), "line: {line}");
        }
    }

    // JSON variant carries the configured flag.
    let out = ctx(
        root,
        &server.url(),
        &["--json", "lsp", "list", "--available"],
    );
    assert_eq!(out.status.code(), Some(0));
    let value = parse_envelope(&out, "lsp.list");
    let languages = value["data"]["languages"].as_array().unwrap();
    let python = languages
        .iter()
        .find(|l| l["language"] == "python")
        .unwrap();
    assert_eq!(python["configured"], true);
    assert_eq!(python["recommended"], "pyright");
    let go = languages.iter().find(|l| l["language"] == "go").unwrap();
    assert_eq!(go["configured"], false);
}

// ============================================================================
// update
// ============================================================================

#[test]
fn update_applies_registry_drift_then_reports_up_to_date() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // Install from the original registry.
    let original = registry_server();
    let out = ctx(root, &original.url(), &["lsp", "add", "python", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(!config_text(root).contains("--verbose"));

    // The registry entry drifts (args gained --verbose).
    let drifted = MockServer::start();
    drifted.add_route(
        "/registry/python.toml",
        "text/plain; charset=utf-8",
        PYTHON_TOML_DRIFTED,
    );

    let out = ctx(root, &drifted.url(), &["lsp", "update", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let output = stdout(&out);
    assert!(output.contains("updated [lsp.python]"), "stdout: {output}");
    assert!(output.contains("args"), "stdout: {output}");
    assert!(config_text(root).contains("--verbose"));

    // Re-running against the same registry is a no-op.
    let out = ctx(root, &drifted.url(), &["lsp", "update", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("up to date"),
        "stdout: {}",
        stdout(&out)
    );

    // JSON variant reports the same status.
    let out = ctx(root, &drifted.url(), &["--json", "lsp", "update", "--yes"]);
    assert_eq!(out.status.code(), Some(0));
    let value = parse_envelope(&out, "lsp.update");
    assert_eq!(value["data"]["languages"][0]["language"], "python");
    assert_eq!(value["data"]["languages"][0]["status"], "up_to_date");
}

#[test]
fn update_preserves_user_keys_on_registry_entry() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let original = registry_server();
    let out = ctx(root, &original.url(), &["lsp", "add", "python", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));

    // The user tunes the registry-managed entry with legal config keys.
    let mut text = config_text(root);
    text.push_str("timeout_ms = 15000\nenv = { JAVA_HOME = \"/opt/java\" }\n");
    fs::write(root.join(".ctx/config.toml"), &text).unwrap();

    let drifted = MockServer::start();
    drifted.add_route(
        "/registry/python.toml",
        "text/plain; charset=utf-8",
        PYTHON_TOML_DRIFTED,
    );
    let out = ctx(root, &drifted.url(), &["lsp", "update", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let output = stdout(&out);
    assert!(output.contains("updated [lsp.python]"), "stdout: {output}");
    assert!(
        output.contains("preserving user keys: timeout_ms, env"),
        "stdout: {output}"
    );
    assert!(output.contains("updated 1, skipped 0"), "stdout: {output}");

    let text = config_text(root);
    // Canonical drift applied...
    assert!(text.contains("--verbose"), "config: {text}");
    // ...user customizations preserved byte-for-byte.
    assert!(text.contains("timeout_ms = 15000\n"), "config: {text}");
    assert!(
        text.contains("env = { JAVA_HOME = \"/opt/java\" }\n"),
        "config: {text}"
    );
    assert!(text.contains("source = \"registry\""), "config: {text}");
}

#[test]
fn update_json_without_yes_with_pending_changes_exits_2_with_empty_stdout() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let original = registry_server();
    let out = ctx(root, &original.url(), &["lsp", "add", "python", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));

    let drifted = MockServer::start();
    drifted.add_route(
        "/registry/python.toml",
        "text/plain; charset=utf-8",
        PYTHON_TOML_DRIFTED,
    );
    // --json never prompts: pending changes without --yes refuse with exit 2
    // and print nothing to stdout (no partial JSON envelope).
    let out = ctx(root, &drifted.url(), &["--json", "lsp", "update"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", stdout(&out));
    assert!(stdout(&out).is_empty(), "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("--yes"), "stderr: {}", stderr(&out));
    // Nothing was written.
    assert!(!config_text(root).contains("--verbose"));
}

#[test]
fn update_user_owned_language_exits_2() {
    let server = registry_server();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, ".ctx/config.toml", "[lsp.go]\ncommand = \"gopls\"\n");

    let out = ctx(root, &server.url(), &["lsp", "update", "go", "--yes"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", stdout(&out));
    assert!(
        stderr(&out).contains("user-owned"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn update_with_nothing_registry_managed_is_a_clean_noop() {
    let server = registry_server();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let out = ctx(root, &server.url(), &["lsp", "update", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("nothing to update"),
        "stdout: {}",
        stdout(&out)
    );
    // No registry traffic without registry-managed entries.
    assert_eq!(server.hits(), 0);
}

// ============================================================================
// doctor
// ============================================================================

#[test]
fn doctor_with_no_config_is_clean() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let out = ctx(root, "http://127.0.0.1:9", &["lsp", "doctor"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("no LSP servers configured"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn doctor_missing_binary_fails_with_exit_1() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        ".ctx/config.toml",
        r#"
[lsp.python]
command = "definitely-not-on-path-xyz"
capabilities = ["documentSymbol"]
"#,
    );

    let out = ctx(root, "http://127.0.0.1:9", &["lsp", "doctor"]);
    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    let output = stdout(&out);
    assert!(output.contains("FAIL"), "stdout: {output}");
    assert!(output.contains("not found on PATH"), "stdout: {output}");

    // JSON variant reports the failure.
    let out = ctx(root, "http://127.0.0.1:9", &["--json", "lsp", "doctor"]);
    assert_eq!(out.status.code(), Some(1));
    let value = parse_envelope(&out, "lsp.doctor");
    assert_eq!(value["data"]["healthy"], false);
    assert_eq!(value["data"]["servers"][0]["status"], "fail");
    assert_eq!(value["data"]["servers"][0]["binary_found"], false);
}

#[test]
fn doctor_malformed_config_fails_with_exit_1() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // Valid TOML, invalid type: the fault-tolerant loader would silently
    // fall back to "nothing configured"; doctor must fail instead.
    write(root, ".ctx/config.toml", "[lsp.python]\ncommand = 3\n");

    let out = ctx(root, "http://127.0.0.1:9", &["lsp", "doctor"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    let output = stdout(&out);
    assert!(output.contains("FAIL"), "stdout: {output}");
    assert!(output.contains("cannot be loaded"), "stdout: {output}");

    let out = ctx(root, "http://127.0.0.1:9", &["--json", "lsp", "doctor"]);
    assert_eq!(out.status.code(), Some(1));
    let value = parse_envelope(&out, "lsp.doctor");
    assert_eq!(value["data"]["healthy"], false);
    assert_eq!(value["data"]["summary"]["fail"], 1);
    assert_eq!(value["data"]["servers"][0]["status"], "fail");
    assert!(
        value["data"]["servers"][0]["error"]
            .as_str()
            .unwrap()
            .contains("cannot be loaded"),
        "envelope: {value}"
    );

    // Whole-file TOML syntax errors fail the same way.
    write(root, ".ctx/config.toml", "this is not : valid toml : :");
    let out = ctx(root, "http://127.0.0.1:9", &["lsp", "doctor"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("FAIL"), "stdout: {}", stdout(&out));
}

#[test]
fn doctor_invalid_blocks_fail_with_exit_1() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // Both blocks parse but are dropped by validation: empty command, and a
    // non-builtin language without extensions.
    write(
        root,
        ".ctx/config.toml",
        r#"
[lsp.python]
command = ""

[lsp.mystery]
command = "mystery-ls"
"#,
    );

    let out = ctx(root, "http://127.0.0.1:9", &["lsp", "doctor"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    let output = stdout(&out);
    assert!(
        output.contains("FAIL python: invalid [lsp.python] block"),
        "stdout: {output}"
    );
    assert!(output.contains("`command` is empty"), "stdout: {output}");
    assert!(
        output.contains("FAIL mystery: invalid [lsp.mystery] block"),
        "stdout: {output}"
    );
    assert!(output.contains("`extensions`"), "stdout: {output}");
    assert!(
        output.contains("2 servers: 0 pass, 0 warn, 2 fail"),
        "stdout: {output}"
    );

    let out = ctx(root, "http://127.0.0.1:9", &["--json", "lsp", "doctor"]);
    assert_eq!(out.status.code(), Some(1));
    let value = parse_envelope(&out, "lsp.doctor");
    assert_eq!(value["data"]["healthy"], false);
    assert_eq!(value["data"]["summary"]["fail"], 2);
    let servers = value["data"]["servers"].as_array().unwrap();
    assert_eq!(servers.len(), 2);
    for server in servers {
        assert_eq!(server["status"], "fail", "envelope: {value}");
        assert!(
            server["error"].as_str().unwrap().contains("invalid [lsp."),
            "envelope: {value}"
        );
    }
}

#[test]
fn doctor_warns_when_capability_missing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let scratch = tempfile::tempdir().unwrap();
    let scenario_path = scratch.path().join("scenario.json");
    // The mock server advertises documentSymbol only; the config also
    // requests definition -> WARN bucket, exit 0.
    fs::write(
        &scenario_path,
        serde_json::json!({
            "server_name": "ctx-mock-warn",
            "capabilities": { "documentSymbolProvider": true },
        })
        .to_string(),
    )
    .unwrap();

    let config = format!(
        r#"
[lsp.kotlin]
command = '{command}'
extensions = ["kt"]
backend = "lsp"
capabilities = ["documentSymbol", "definition"]
env = {{ CTX_INTERNAL_MOCK_LSP = '{scenario}' }}
"#,
        command = env!("CARGO_BIN_EXE_ctx"),
        scenario = scenario_path.display(),
    );
    write(root, ".ctx/config.toml", &config);

    let out = ctx(root, "http://127.0.0.1:9", &["lsp", "doctor"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    let output = stdout(&out);
    assert!(output.contains("WARN"), "stdout: {output}");
    assert!(output.contains("definition"), "stdout: {output}");
    assert!(
        output.contains("1 server: 0 pass, 1 warn, 0 fail"),
        "stdout: {output}"
    );

    let out = ctx(root, "http://127.0.0.1:9", &["--json", "lsp", "doctor"]);
    assert_eq!(out.status.code(), Some(0));
    let value = parse_envelope(&out, "lsp.doctor");
    assert_eq!(value["data"]["healthy"], true);
    assert_eq!(value["data"]["servers"][0]["status"], "warn");
    assert_eq!(
        value["data"]["servers"][0]["missing_capabilities"][0],
        "definition"
    );
}

#[test]
fn doctor_with_mock_server_passes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let scratch = tempfile::tempdir().unwrap();
    let scenario_path = scratch.path().join("scenario.json");
    fs::write(
        &scenario_path,
        serde_json::json!({ "server_name": "ctx-mock-doctor" }).to_string(),
    )
    .unwrap();

    // The ctx binary doubles as a scripted language server when
    // CTX_INTERNAL_MOCK_LSP is set (same trick as tests/lsp_cli.rs). TOML
    // literal strings keep Windows backslashes intact.
    let config = format!(
        r#"
[lsp.kotlin]
command = '{command}'
extensions = ["kt"]
backend = "lsp"
capabilities = ["documentSymbol", "definition"]
env = {{ CTX_INTERNAL_MOCK_LSP = '{scenario}' }}
"#,
        command = env!("CARGO_BIN_EXE_ctx"),
        scenario = scenario_path.display(),
    );
    write(root, ".ctx/config.toml", &config);

    let out = ctx(root, "http://127.0.0.1:9", &["lsp", "doctor"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    let output = stdout(&out);
    assert!(output.contains("PASS"), "stdout: {output}");
    assert!(output.contains("ctx-mock-doctor"), "stdout: {output}");

    let out = ctx(root, "http://127.0.0.1:9", &["--json", "lsp", "doctor"]);
    assert_eq!(out.status.code(), Some(0));
    let value = parse_envelope(&out, "lsp.doctor");
    assert_eq!(value["data"]["healthy"], true);
    assert_eq!(value["data"]["servers"][0]["status"], "pass");
    assert_eq!(value["data"]["servers"][0]["handshake_ok"], true);
}
