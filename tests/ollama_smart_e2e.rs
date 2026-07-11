//! Real end-to-end `ctx smart` relevance test driven by a live Ollama server.
//!
//! This is the "last mile" that the deterministic seeded-vector guard
//! (`tests/smart_relevance.rs`) can't cover: it indexes and embeds a real
//! fixture with a real embedding model, then runs the real `ctx smart` CLI and
//! checks it selects the on-topic file.
//!
//! It is **gated** and never runs in normal CI: it is a no-op unless
//! `CTX_TEST_OLLAMA=1` is set AND an Ollama daemon is reachable. Locally:
//!
//! ```sh
//! ollama pull qwen3-embedding:8b            # or set CTX_TEST_OLLAMA_MODEL
//! CTX_TEST_OLLAMA=1 cargo test --test ollama_smart_e2e -- --nocapture
//! ```
//!
//! Assertions are on relative file *ranking*, not on exact vectors/scores, so
//! they survive model/version drift.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use assert_cmd::Command;
use tempfile::TempDir;

/// Default model — overridable with `CTX_TEST_OLLAMA_MODEL`.
const DEFAULT_MODEL: &str = "qwen3-embedding:8b";

/// Return the model to use, or `None` if the test should be skipped (gate unset
/// or the Ollama daemon is unreachable).
fn skip_or_model() -> Option<String> {
    if std::env::var("CTX_TEST_OLLAMA").ok().as_deref() != Some("1") {
        eprintln!("skipping ollama e2e: set CTX_TEST_OLLAMA=1 to run");
        return None;
    }
    // Probe reachability so we skip (not fail) when nothing is listening.
    let host = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "localhost:11434".to_string());
    let hostport = host
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_string();
    let reachable = hostport
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .map(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(750)).is_ok())
        .unwrap_or(false);
    if !reachable {
        eprintln!("skipping ollama e2e: no Ollama daemon reachable at {host}");
        return None;
    }
    Some(std::env::var("CTX_TEST_OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string()))
}

/// Write a small fixture repo with three topically-distinct Rust files so the
/// embedding model has real signal to separate them.
fn write_fixture(root: &Path) {
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();

    std::fs::write(
        src.join("parser.rs"),
        r#"
/// Parse Solidity contract source code into an abstract syntax tree.
///
/// Walks the token stream and builds AST nodes for contracts, functions,
/// and state variables so downstream passes can analyze the syntax tree.
pub fn parse_contract_source(source: &str) -> Vec<AstNode> {
    let mut nodes = Vec::new();
    for line in source.lines() {
        nodes.push(AstNode::from_line(line));
    }
    nodes
}

pub struct AstNode {
    pub kind: String,
}

impl AstNode {
    pub fn from_line(line: &str) -> Self {
        AstNode { kind: line.trim().to_string() }
    }
}
"#,
    )
    .unwrap();

    std::fs::write(
        src.join("http_client.rs"),
        r#"
/// Send an authenticated HTTP request to a remote server and return the body.
pub fn send_request(url: &str, token: &str) -> String {
    format!("GET {url} with bearer {token}")
}
"#,
    )
    .unwrap();

    std::fs::write(
        src.join("math_utils.rs"),
        r#"
/// Compute the greatest common divisor of two integers.
pub fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}
"#,
    )
    .unwrap();
}

fn ctx(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("ctx").unwrap();
    c.current_dir(dir);
    c
}

#[test]
fn smart_selects_on_topic_file_with_ollama() {
    let Some(model) = skip_or_model() else {
        return;
    };

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_fixture(root);

    // 1. Index the fixture.
    ctx(root).arg("index").assert().success();

    // 2. Embed it with the real Ollama model.
    ctx(root)
        .args(["embed", "--provider", "ollama"])
        .env("OLLAMA_EMBED_MODEL", &model)
        .assert()
        .success();

    // 3. Ask `ctx smart` for a clearly parser-related task and inspect the
    //    ranked candidate list (dry-run shows all candidates, budget aside).
    let output = ctx(root)
        .args([
            "smart",
            "--provider",
            "ollama",
            "--dry-run",
            "parse solidity contract source into a syntax tree",
        ])
        .env("OLLAMA_EMBED_MODEL", &model)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    eprintln!("--- ctx smart --dry-run output ---\n{stdout}");

    // Collect selected paths in ranked order from the "  <path> (N tokens) - ..." lines.
    let ranked: Vec<String> = stdout
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|tok| tok.ends_with(".rs"))
        .map(|s| s.to_string())
        .collect();

    let rank = |needle: &str| ranked.iter().position(|p| p.contains(needle));

    assert!(
        rank("parser.rs").is_some(),
        "parser.rs must be selected for a parsing task; got {ranked:?}"
    );
    // The on-topic parser file must rank above the clearly off-topic files.
    if let (Some(p), Some(h)) = (rank("parser.rs"), rank("http_client.rs")) {
        assert!(
            p < h,
            "parser.rs must outrank http_client.rs for a parsing task; got {ranked:?}"
        );
    }
    if let (Some(p), Some(m)) = (rank("parser.rs"), rank("math_utils.rs")) {
        assert!(
            p < m,
            "parser.rs must outrank math_utils.rs for a parsing task; got {ranked:?}"
        );
    }
}
