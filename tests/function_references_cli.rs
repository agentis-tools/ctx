//! End-to-end coverage for Rust function-item `uses` relationships.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn index_fixture(files: &[(&str, &str)]) -> TempDir {
    let temp = TempDir::new().expect("create fixture");
    for (path, source) in files {
        let path = temp.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).expect("create fixture directory");
        std::fs::write(path, source).expect("write fixture source");
    }

    Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .arg("index")
        .assert()
        .success();
    temp
}

fn json_command(temp: &TempDir, args: &[&str]) -> Value {
    let output = Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .args(args)
        .output()
        .expect("run ctx command");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid JSON envelope")
}

#[test]
fn references_surface_in_deps_and_explain_but_not_callers() {
    let temp = index_fixture(&[
        (
            "src/main.rs",
            r#"
mod callbacks;

fn spawn<T>(_callback: T) {}
fn transform(value: i32) -> i32 { value + 1 }
fn direct() {}

fn main() {
    spawn(callbacks::run_main);
    [1, 2].into_iter().map(transform);
    direct();
}
"#,
        ),
        ("src/callbacks.rs", "pub fn run_main() {}\n"),
    ]);

    let deps = json_command(
        &temp,
        &["--json", "query", "deps", "main", "--file", "src/main.rs"],
    );
    let dependencies = deps["data"]["dependencies"]
        .as_array()
        .expect("dependencies array");
    for target in ["run_main", "transform"] {
        let reference = dependencies
            .iter()
            .find(|dependency| dependency["kind"] == "uses" && dependency["target_name"] == target)
            .unwrap_or_else(|| panic!("missing uses edge to {target}"));
        assert_eq!(reference["distance"], 1);
        assert_eq!(reference["resolved"]["kind"], "function");
    }
    assert!(!dependencies
        .iter()
        .any(|dependency| dependency["kind"] == "calls"
            && matches!(
                dependency["target_name"].as_str(),
                Some("run_main" | "transform")
            )));

    Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .args(["explain", "main", "--file", "src/main.rs"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Relationships"))
        .stdout(predicates::str::contains("run_main [uses]"))
        .stdout(predicates::str::contains("transform [uses]"));

    let callers = json_command(
        &temp,
        &[
            "--json",
            "query",
            "callers",
            "run_main",
            "--file",
            "src/callbacks.rs",
        ],
    );
    assert!(callers["data"]["callers"].as_array().unwrap().is_empty());
    assert!(callers["data"]["unresolved_callers"]
        .as_array()
        .unwrap()
        .is_empty());

    let explained = json_command(
        &temp,
        &[
            "--json",
            "explain",
            "run_main",
            "--file",
            "src/callbacks.rs",
        ],
    );
    assert_eq!(explained["data"]["callers_count"], 0);
}

#[test]
fn ambiguous_rust_function_items_remain_unresolved() {
    let temp = index_fixture(&[
        (
            "src/main.rs",
            r#"
fn consume<T>(_callback: T) {}
mod first;
fn main() {
    consume(transform);
    consume(first::transform);
}
"#,
        ),
        (
            "src/first.rs",
            "fn transform(value: i32) -> i32 { value }\n",
        ),
        (
            "src/second.rs",
            "fn transform(value: i32) -> i32 { value + 1 }\n",
        ),
    ]);

    let deps = json_command(
        &temp,
        &["--json", "query", "deps", "main", "--file", "src/main.rs"],
    );
    let references: Vec<_> = deps["data"]["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|dependency| dependency["kind"] == "uses")
        .collect();
    assert_eq!(references.len(), 2);
    assert!(references.iter().any(|reference| {
        reference["target_name"] == "transform" && reference["resolved"].is_null()
    }));
    assert!(references.iter().any(|reference| {
        reference["resolved"]["file"] == "src/first.rs"
            && reference["resolved"]["name"] == "transform"
    }));
}

#[cfg(feature = "duckdb")]
#[test]
fn references_are_queryable_but_do_not_change_call_metrics_or_call_graph() {
    let temp = index_fixture(&[(
        "src/main.rs",
        r#"
fn spawn<T>(_callback: T) {}
fn run_main() {}
fn main() { spawn(run_main); }
"#,
    )]);

    let sql = json_command(
        &temp,
        &[
            "sql",
            "--json",
            "SELECT e.kind, s.fan_out FROM v1.edges e JOIN v1.symbols s ON s.id = e.source_id WHERE e.source_name = 'main' ORDER BY e.kind",
        ],
    );
    let rows = sql["data"]["rows"].as_array().expect("SQL rows");
    assert!(rows.iter().any(|row| row[0] == "uses"));
    assert!(rows.iter().all(|row| row[1] == 1));

    let graph = json_command(&temp, &["--json", "query", "graph", "main", "--depth", "2"]);
    let serialized = serde_json::to_string(&graph["data"]).unwrap();
    assert!(serialized.contains("spawn"));
    assert!(!serialized.contains("run_main"));

    let impact = json_command(
        &temp,
        &["--json", "query", "impact", "run_main", "--depth", "2"],
    );
    assert_eq!(impact["data"]["total"], 0);
}

/// Nothing in the syntax separates a function value from a constant in argument
/// position, so the capture takes both. An identifier that names no function in
/// the index is not a function value, and must not survive as an unresolved
/// dependency leaf -- unlike an ambiguous name, which is genuine evidence.
#[test]
fn non_function_arguments_do_not_become_reference_edges() {
    let temp = index_fixture(&[(
        "src/main.rs",
        r#"
const MAX_RETRIES: i32 = 3;
static GREETING: &str = "hi";

fn retry(_limit: i32) {}
fn announce(_text: &str) {}
fn consume<T>(_callback: T) {}
fn worker() {}

fn main() {
    retry(MAX_RETRIES);
    announce(GREETING);
    consume(worker);
}
"#,
    )]);

    let deps = json_command(
        &temp,
        &["--json", "query", "deps", "main", "--file", "src/main.rs"],
    );
    let references: Vec<_> = deps["data"]["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|dependency| dependency["kind"] == "uses")
        .collect();

    let names: Vec<_> = references
        .iter()
        .map(|reference| reference["target_name"].as_str().unwrap_or_default())
        .collect();
    assert!(
        !names.contains(&"MAX_RETRIES"),
        "a constant must not become a function reference: {names:?}"
    );
    assert!(
        !names.contains(&"GREETING"),
        "a static must not become a function reference: {names:?}"
    );
    assert!(
        names.contains(&"worker"),
        "a real function value must still be recorded: {names:?}"
    );
    assert_eq!(references.len(), 1, "unexpected references: {names:?}");
}
