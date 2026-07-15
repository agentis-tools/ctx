//! End-to-end coverage for source locations in `ctx query impact --json`.

#![cfg(feature = "duckdb")]

use assert_cmd::Command;
use serde_json::Value;

#[test]
fn impact_json_reports_indexed_source_locations() {
    let temp = tempfile::tempdir().expect("create fixture");
    let src = temp.path().join("src");
    std::fs::create_dir_all(&src).expect("create source directory");
    std::fs::write(
        src.join("main.rs"),
        r#"fn helper() {}

fn caller() {
    helper();
}

fn main() {
    caller();
}
"#,
    )
    .expect("write fixture");

    Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .arg("index")
        .assert()
        .success();

    let query = |depth: &str| {
        let output = Command::cargo_bin("ctx")
            .unwrap()
            .current_dir(temp.path())
            .args(["--json", "query", "impact", "helper", "--depth", depth])
            .output()
            .expect("query impact JSON");
        assert!(
            output.status.success(),
            "query failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).expect("valid JSON envelope")
    };

    let depth_one = query("1");
    let impacted = depth_one["data"]["impacted"]
        .as_array()
        .expect("impacted array");
    assert_eq!(impacted.len(), 1);
    let symbol = &impacted[0]["symbol"];
    assert_eq!(symbol["name"], "caller");
    assert_eq!(symbol["file"], "src/main.rs");
    let line_start = symbol["line_start"].as_i64().expect("numeric start line");
    let line_end = symbol["line_end"].as_i64().expect("numeric end line");
    assert_eq!((line_start, line_end), (3, 5));
    assert_eq!(impacted[0]["distance"], 1);

    let depth_two = query("2");
    let impacted = depth_two["data"]["impacted"]
        .as_array()
        .expect("impacted array");
    assert_eq!(impacted.len(), 2);
    assert_eq!(impacted[0]["symbol"]["name"], "caller");
    assert_eq!(impacted[0]["symbol"]["line_start"], 3);
    assert_eq!(impacted[0]["symbol"]["line_end"], 5);
    assert_eq!(impacted[0]["distance"], 1);
    assert_eq!(impacted[1]["symbol"]["name"], "main");
    assert_eq!(impacted[1]["symbol"]["line_start"], 7);
    assert_eq!(impacted[1]["symbol"]["line_end"], 9);
    assert_eq!(impacted[1]["distance"], 2);
}
