//! Regression fixtures for identity-safe `ctx query callers` output.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn indexed_mixed_language_fixture() -> TempDir {
    let temp = TempDir::new().expect("create fixture");
    let src = temp.path().join("src");
    let scripts = temp.path().join("scripts");
    std::fs::create_dir_all(&src).expect("create Rust fixture directory");
    std::fs::create_dir_all(&scripts).expect("create Python fixture directory");

    std::fs::write(
        src.join("main.rs"),
        r#"
fn run() {}

fn run_main() {
    run();
}

fn main() {
    run_main();
}
"#,
    )
    .expect("write Rust target fixture");
    std::fs::write(
        src.join("same_name.rs"),
        r#"
fn run() {}

fn calls_other_run() {
    run();
}
"#,
    )
    .expect("write same-name Rust fixture");
    std::fs::write(
        src.join("unresolved.rs"),
        r#"
fn rust_probe() {
    run();
}
"#,
    )
    .expect("write unresolved Rust fixture");
    std::fs::write(
        scripts.join("runner.py"),
        r#"
import subprocess

def run():
    return None

def python_probe():
    run()
    subprocess.run(["true"])
"#,
    )
    .expect("write Python fixture");

    Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .arg("index")
        .assert()
        .success();

    temp
}

#[test]
fn callers_are_resolved_to_the_selected_id_and_unresolved_evidence_is_separate() {
    let temp = indexed_mixed_language_fixture();
    let output = Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .args(["--json", "query", "callers", "run", "--file", "src/main.rs"])
        .output()
        .expect("query callers JSON");
    assert!(
        output.status.success(),
        "query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let envelope: Value = serde_json::from_slice(&output.stdout).expect("valid JSON envelope");
    let data = &envelope["data"];
    assert_eq!(data["target"]["file"], "src/main.rs");

    let callers = data["callers"].as_array().expect("callers array");
    assert_eq!(callers.len(), 2);
    assert_eq!(callers[0]["symbol"]["name"], "run_main");
    assert_eq!(callers[0]["symbol"]["file"], "src/main.rs");
    assert_eq!(callers[0]["distance"], 1);
    assert_eq!(callers[1]["symbol"]["name"], "main");
    assert_eq!(callers[1]["distance"], 2);

    let unresolved = data["unresolved_callers"]
        .as_array()
        .expect("unresolved callers array");
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0]["symbol"]["name"], "rust_probe");
    assert_eq!(unresolved[0]["symbol"]["file"], "src/unresolved.rs");
    assert_eq!(unresolved[0]["distance"], 1);

    let serialized = serde_json::to_string(data).unwrap();
    assert!(!serialized.contains("python_probe"));
    assert!(!serialized.contains("calls_other_run"));
    assert!(!serialized.contains("subprocess.run"));
}

#[test]
fn callers_and_deps_honor_requested_depth() {
    let temp = indexed_mixed_language_fixture();

    let query = |args: &[&str]| {
        let output = Command::cargo_bin("ctx")
            .unwrap()
            .current_dir(temp.path())
            .args(args)
            .output()
            .expect("query JSON");
        assert!(
            output.status.success(),
            "query failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let envelope: Value = serde_json::from_slice(&output.stdout).expect("valid JSON envelope");
        envelope["data"].clone()
    };

    let callers_one = query(&[
        "--json",
        "query",
        "callers",
        "run",
        "--file",
        "src/main.rs",
        "--depth",
        "1",
    ]);
    assert_eq!(callers_one["callers"].as_array().unwrap().len(), 1);
    assert_eq!(callers_one["callers"][0]["distance"], 1);

    let callers_three = query(&[
        "--json",
        "query",
        "callers",
        "run",
        "--file",
        "src/main.rs",
        "--depth",
        "3",
    ]);
    assert_eq!(callers_three["callers"].as_array().unwrap().len(), 2);
    assert_eq!(callers_three["callers"][1]["distance"], 2);

    let deps_one = query(&[
        "--json",
        "query",
        "deps",
        "main",
        "--file",
        "src/main.rs",
        "--depth",
        "1",
    ]);
    assert_eq!(deps_one["dependencies"].as_array().unwrap().len(), 1);
    assert_eq!(deps_one["dependencies"][0]["target_name"], "run_main");
    assert_eq!(deps_one["dependencies"][0]["distance"], 1);

    let deps_three = query(&[
        "--json",
        "query",
        "deps",
        "main",
        "--file",
        "src/main.rs",
        "--depth",
        "3",
    ]);
    assert_eq!(deps_three["dependencies"].as_array().unwrap().len(), 2);
    assert_eq!(deps_three["dependencies"][1]["target_name"], "run");
    assert_eq!(deps_three["dependencies"][1]["distance"], 2);

    Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "query",
            "callers",
            "run",
            "--file",
            "src/main.rs",
            "--depth",
            "3",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("Distance 1:"))
        .stdout(predicates::str::contains("Distance 2:"));
}
