//! `ctx grep`: ripgrep recall, index-derived tags.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn write(temp: &TempDir, path: &str, content: &str) {
    let path = temp.path().join(path);
    std::fs::create_dir_all(path.parent().unwrap()).expect("create fixture dir");
    std::fs::write(path, content).expect("write fixture");
}

fn fixture(indexed: bool) -> TempDir {
    let temp = TempDir::new().expect("create fixture");
    write(
        &temp,
        "src/conn.rs",
        r#"pub trait Closeable {
    fn close(&self);
}

pub struct Conn;

impl Closeable for Conn {
    fn close(&self) {}
}

fn release_helper() {}

pub fn shutdown(conn: &Conn) {
    // close the connection before exit
    release_helper();
    conn.close();
}
"#,
    );
    write(
        &temp,
        "src/file.rs",
        r#"pub struct File;

impl File {
    pub fn close(&self) {}
}
"#,
    );
    write(&temp, "NOTES.md", "Remember to close everything.\n");

    if indexed {
        Command::cargo_bin("ctx")
            .unwrap()
            .current_dir(temp.path())
            .arg("index")
            .assert()
            .success();
    }
    temp
}

fn grep_json(temp: &TempDir, args: &[&str]) -> Value {
    let output = Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .arg("--json")
        .arg("grep")
        .args(args)
        .output()
        .expect("run ctx grep");
    assert!(
        output.status.success(),
        "ctx grep failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).expect("one JSON envelope");
    assert_eq!(envelope["command"], "grep");
    envelope["data"].clone()
}

fn line_tags(data: &Value, file: &str, line: u64) -> Vec<Value> {
    data["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["file"] == file && m["line"] == line)
        .unwrap_or_else(|| panic!("no match at {file}:{line}"))["tags"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn annotation_never_drops_a_match() {
    let temp = fixture(true);
    let annotated = grep_json(&temp, &["close"]);
    let plain = grep_json(&temp, &["--no-annotate", "close"]);

    let sites = |data: &Value| -> Vec<(String, u64)> {
        data["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m["file"].as_str().unwrap().to_string(),
                    m["line"].as_u64().unwrap(),
                )
            })
            .collect()
    };
    assert_eq!(sites(&annotated), sites(&plain));
    assert_eq!(sites(&annotated).len(), 6);
    assert_eq!(plain["index"], "disabled");
    assert!(plain["matches"][0]["tags"].as_array().unwrap().is_empty());
}

#[test]
fn matches_are_tagged_from_the_index() {
    let temp = fixture(true);
    let data = grep_json(&temp, &["-w", "close|release_helper"]);
    assert_eq!(data["index"], "present");
    assert_eq!(data["tag_vocabulary_version"], 1);

    // Definition inside `impl Closeable for Conn`: a member of the family.
    let def = line_tags(&data, "src/conn.rs", 8);
    assert_eq!(def[0]["tag"], "def");
    assert_eq!(def[0]["target"]["name"], "close");
    assert_eq!(def[0]["family"], "Closeable");

    // A unique-name call resolves to its definition.
    let call = line_tags(&data, "src/conn.rs", 15);
    assert_eq!(call[0]["tag"], "ref");
    assert_eq!(call[0]["edge_kind"], "calls");
    assert_eq!(call[0]["target"]["name"], "release_helper");

    // A comment mentions the name; the index has nothing to say about it.
    let comment = line_tags(&data, "src/conn.rs", 14);
    assert_eq!(comment[0]["tag"], "unlinked");
    assert_eq!(comment[0]["identifier"], "close");

    // Markdown is outside the index but still searched.
    let notes = line_tags(&data, "NOTES.md", 1);
    assert_eq!(notes[0]["tag"], "unindexed");

    assert!(data["by_tag"]["def"].as_u64().unwrap() >= 2);
}

#[test]
fn edited_files_are_stale_not_misannotated() {
    let temp = fixture(true);
    write(
        &temp,
        "src/file.rs",
        "// moved\npub struct File;\n\nimpl File {\n    pub fn close(&self) {}\n}\n",
    );
    let data = grep_json(&temp, &["-w", "close", "src/file.rs"]);
    let tags = line_tags(&data, "src/file.rs", 5);
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0]["tag"], "stale");
}

#[test]
fn works_without_an_index() {
    let temp = fixture(false);
    let data = grep_json(&temp, &["close"]);
    assert_eq!(data["index"], "missing");
    assert_eq!(data["matches"].as_array().unwrap().len(), 6);
    assert_eq!(data["by_tag"]["unindexed"], 6);
}

#[test]
fn no_matches_is_success_and_bad_patterns_are_operational_errors() {
    let temp = fixture(true);
    let data = grep_json(&temp, &["no_such_identifier_anywhere"]);
    assert!(data["matches"].as_array().unwrap().is_empty());

    Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .args(["grep", "unclosed("])
        .assert()
        .code(2);
}

#[test]
fn text_output_has_a_summary_and_plain_mode_matches_rg() {
    let temp = fixture(true);
    let annotated = Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .args(["grep", "-w", "release_helper"])
        .output()
        .unwrap();
    let text = String::from_utf8(annotated.stdout).unwrap();
    let mut lines = text.lines();
    assert!(lines
        .next()
        .unwrap()
        .starts_with("ctx grep: 2 matches in 1 files"));
    assert!(lines
        .next()
        .unwrap()
        .starts_with("src/conn.rs:11:fn release_helper() {}  [def "));

    let plain = Command::cargo_bin("ctx")
        .unwrap()
        .current_dir(temp.path())
        .args(["grep", "--no-annotate", "-w", "release_helper"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(plain.stdout).unwrap(),
        "src/conn.rs:11:fn release_helper() {}\nsrc/conn.rs:15:    release_helper();\n"
    );
}

#[test]
fn globs_and_types_scope_the_search() {
    let temp = fixture(true);
    let only_md = grep_json(&temp, &["-g", "*.md", "close"]);
    assert_eq!(only_md["matches"].as_array().unwrap().len(), 1);

    let only_rust = grep_json(&temp, &["-t", "rust", "close"]);
    assert_eq!(only_rust["matches"].as_array().unwrap().len(), 5);

    // The global `-i/--ignore` keeps its meaning: it excludes files.
    let ignored = grep_json(&temp, &["-i", "*.md", "close"]);
    assert_eq!(ignored["matches"].as_array().unwrap().len(), 5);

    let max = grep_json(&temp, &["--max-results", "2", "close"]);
    assert_eq!(max["matches"].as_array().unwrap().len(), 2);
    assert_eq!(max["truncated"], true);
}
