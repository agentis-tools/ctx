//! End-to-end CLI tests for `ctx harness` (init / compat / doctor).

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use ctx::testutil::GitRepo;

/// A small Rust source file so `ctx index` has something to chew on.
const SOURCE: &str = r#"
pub fn compute_total(items: &[i64]) -> i64 {
    let mut total = 0;
    for item in items {
        total += *item;
    }
    total
}
"#;

fn ctx(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run ctx binary")
}

/// Run a generated hook script with `sh`, empty stdin, and the ctx binary's
/// directory prefixed to PATH (so the script's bare `ctx` resolves to the
/// freshly built binary).
#[cfg(unix)]
fn run_hook(dir: &Path, script: &Path) -> Output {
    let bin_dir = Path::new(env!("CARGO_BIN_EXE_ctx")).parent().unwrap();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new("sh")
        .arg(script)
        .current_dir(dir)
        .env("PATH", path)
        .stdin(Stdio::null())
        .output()
        .expect("failed to run hook script")
}

// ============================================================================
// (1) local init end-to-end: generated hook runs and emits check JSON
// ============================================================================

#[cfg(unix)]
#[test]
fn test_local_init_post_tool_use_hook_runs_end_to_end() {
    let temp = tempfile::tempdir().unwrap();
    let repo = GitRepo::init(temp.path());
    let root = &repo.root;
    repo.commit_file("src/lib.rs", SOURCE, "initial");

    assert!(ctx(root, &["index"]).status.success());
    let out = ctx(root, &["harness", "init", "--mode", "local"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Scaffold exists.
    for name in ["session-start.sh", "post-tool-use.sh", "stop.sh"] {
        assert!(root.join(".claude/hooks/ctx").join(name).exists(), "{name}");
    }
    assert!(root.join(".ctx/rules.toml").exists());
    assert!(root.join(".ctx/harness.lock").exists());

    // init now wires .claude/settings.json itself (no manual paste).
    let settings_path = root.join(".claude/settings.json");
    assert!(settings_path.exists(), "settings.json should be written");
    let settings = fs::read_to_string(&settings_path).unwrap();
    serde_json::from_str::<serde_json::Value>(&settings).expect("settings.json is valid JSON");
    assert!(
        settings.contains(".claude/hooks/ctx/"),
        "settings: {settings}"
    );

    // The CLAUDE.md guidance still goes to stdout.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ctx map --budget 2000"), "stdout: {stdout}");

    // Run the PostToolUse hook like Claude Code would.
    let hook = root.join(".claude/hooks/ctx/post-tool-use.sh");
    let out = run_hook(root, &hook);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"command\": \"check\""),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ============================================================================
// (1b) settings.json auto-wiring: merge, idempotency, invalid-JSON fallback
// ============================================================================

#[test]
fn test_init_merges_into_existing_settings_additively() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".claude")).unwrap();
    fs::write(
        root.join(".claude/settings.json"),
        r#"{"permissions":{"allow":["Bash(git *)"]},"foo":"bar"}"#,
    )
    .unwrap();

    let out = ctx(root, &["harness", "init", "--mode", "local"]);
    assert_eq!(out.status.code(), Some(0));

    let content = fs::read_to_string(root.join(".claude/settings.json")).unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&content).expect("settings.json stays valid JSON");

    // Unrelated user settings preserved.
    assert_eq!(value["foo"], "bar");
    let allow = value["permissions"]["allow"].as_array().unwrap();
    assert!(allow.contains(&serde_json::json!("Bash(git *)")));
    // ctx entries added.
    assert!(allow.contains(&serde_json::json!("Bash(ctx *)")));
    assert!(content.contains(".claude/hooks/ctx/"), "content: {content}");

    // No duplicate hook groups: each event has exactly one ctx group.
    for event in ["SessionStart", "PostToolUse", "Stop"] {
        let groups = value["hooks"][event].as_array().unwrap();
        let ctx_groups = groups
            .iter()
            .filter(|g| g.to_string().contains(".claude/hooks/ctx/"))
            .count();
        assert_eq!(ctx_groups, 1, "{event}: {groups:?}");
    }
}

#[test]
fn test_init_is_idempotent_on_settings() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let out = ctx(root, &["harness", "init", "--mode", "local"]);
    assert_eq!(out.status.code(), Some(0));
    let first = fs::read(root.join(".claude/settings.json")).unwrap();

    let out = ctx(root, &["harness", "init", "--mode", "local"]);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("already wired"), "stderr: {stderr}");

    let second = fs::read(root.join(".claude/settings.json")).unwrap();
    assert_eq!(first, second, "settings bytes must be identical on re-init");
}

#[test]
fn test_init_leaves_invalid_settings_untouched_and_prints_snippet() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".claude")).unwrap();
    fs::write(root.join(".claude/settings.json"), "{not json").unwrap();

    let out = ctx(root, &["harness", "init", "--mode", "local"]);
    assert_eq!(out.status.code(), Some(0));

    // File left byte-for-byte unchanged.
    assert_eq!(
        fs::read_to_string(root.join(".claude/settings.json")).unwrap(),
        "{not json"
    );
    // Fallback: the snippet is printed to stdout for manual merge.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"Bash(ctx *)\""), "stdout: {stdout}");
    assert!(stdout.contains("$CLAUDE_PROJECT_DIR"), "stdout: {stdout}");
}

// ============================================================================
// (2) checksum-guarded regeneration
// ============================================================================

#[test]
fn test_reinit_skips_modified_files_and_force_regenerates() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    assert!(ctx(root, &["harness", "init"]).status.success());

    // Tamper with one hook.
    let stop = root.join(".claude/hooks/ctx/stop.sh");
    let tampered = fs::read_to_string(&stop).unwrap() + "echo tampered\n";
    fs::write(&stop, &tampered).unwrap();

    // Re-init: warns on stderr, leaves the file untouched, regenerates the
    // unmodified ones silently (no warning for them).
    let out = ctx(root, &["harness", "init"]);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("skipped .claude/hooks/ctx/stop.sh"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("skipped .claude/hooks/ctx/session-start.sh"));
    assert!(stderr.contains("regenerated  .claude/hooks/ctx/session-start.sh"));
    assert_eq!(fs::read_to_string(&stop).unwrap(), tampered);

    // Customize rules.toml; --force regenerates the hook but not the rules.
    fs::write(root.join(".ctx/rules.toml"), "version = 1\n# my policy\n").unwrap();
    let out = ctx(root, &["harness", "init", "--force"]);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("overwrote    .claude/hooks/ctx/stop.sh"),
        "stderr: {stderr}"
    );
    assert!(!fs::read_to_string(&stop).unwrap().contains("tampered"));
    assert_eq!(
        fs::read_to_string(root.join(".ctx/rules.toml")).unwrap(),
        "version = 1\n# my policy\n"
    );
    assert!(stderr.contains("never overwritten"), "stderr: {stderr}");
}

// ============================================================================
// (3) compat exit codes
// ============================================================================

#[test]
fn test_compat_exit_codes_and_output_discipline() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // Unsatisfied requirement: exit 3, exactly one stderr line, no stdout.
    let out = ctx(root, &["harness", "compat", "--require", "999.0"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty(), "stdout: {:?}", out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stderr.trim_end().lines().count(), 1, "stderr: {stderr}");
    assert!(stderr.contains("999.0"), "stderr: {stderr}");

    // Satisfied requirement: exit 0, fully silent.
    let out = ctx(root, &["harness", "compat", "--require", "0.1"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());

    // Garbage requirement: operational error (2), never 3.
    let out = ctx(root, &["harness", "compat", "--require", "garbage"]);
    assert_eq!(out.status.code(), Some(2));
}

// ============================================================================
// (4) fail-open hook with a too-new baked version
// ============================================================================

#[cfg(unix)]
#[test]
fn test_hook_fails_open_when_binary_is_older_than_templates() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let content =
        ctx::harness::templates::render_hook_with_version("post-tool-use", "999.0.0", "main")
            .unwrap();
    let script = root.join("post-tool-use.sh");
    fs::write(&script, content).unwrap();

    let out = run_hook(root, &script);
    assert_eq!(out.status.code(), Some(0), "fail open: exit 0");
    assert!(
        out.stdout.is_empty(),
        "no update notices on stdout: {:?}",
        out.stdout
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("999.0.0"), "stderr: {stderr}");
    assert!(
        stderr.contains("skipping post-tool-use action"),
        "stderr: {stderr}"
    );
}

// ============================================================================
// (5) doctor --json reports independent findings
// ============================================================================

#[test]
fn test_doctor_json_reports_missing_index_and_invalid_rules() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ctx")).unwrap();
    fs::write(root.join(".ctx/rules.toml"), "[layers\nbroken = [").unwrap();

    let out = ctx(root, &["harness", "doctor", "--json"]);
    assert_eq!(out.status.code(), Some(1), "problems -> exit 1");

    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is a single JSON envelope");
    assert_eq!(doc["command"], "harness.doctor");
    assert_eq!(doc["ctx_version"], env!("CARGO_PKG_VERSION"));
    let data = &doc["data"];
    assert_eq!(data["healthy"], false);
    assert_eq!(data["binary_version"], env!("CARGO_PKG_VERSION"));
    assert!(data["mcp_compiled"].is_boolean());

    let codes: Vec<&str> = data["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"index_missing"), "codes: {codes:?}");
    assert!(codes.contains(&"rules_invalid"), "codes: {codes:?}");
    assert!(
        data["summary"]["errors"].as_u64().unwrap() >= 1
            && data["summary"]["warnings"].as_u64().unwrap() >= 1,
        "summary: {}",
        data["summary"]
    );
}

// ============================================================================
// (6) unknown target
// ============================================================================

#[test]
fn test_unknown_target_is_usage_error_listing_supported_targets() {
    let temp = tempfile::tempdir().unwrap();
    let out = ctx(temp.path(), &["harness", "init", "--target", "cursor"]);
    assert_eq!(out.status.code(), Some(2), "clap usage errors exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("claude"), "stderr: {stderr}");
    assert!(stderr.contains("cursor"), "stderr: {stderr}");
}

// ============================================================================
// (7) plugin scaffold
// ============================================================================

#[test]
fn test_plugin_scaffold_shape() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let out = ctx(root, &["harness", "init", "--mode", "plugin"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Every generated .json parses.
    let mut json_files = vec![
        ".claude-plugin/plugin.json",
        ".claude-plugin/marketplace.json",
        "hooks/hooks.json",
        "settings.json",
    ];
    if root.join(".mcp.json").exists() {
        json_files.push(".mcp.json");
    }
    for rel in &json_files {
        let content = fs::read_to_string(root.join(rel)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("{rel} is not valid JSON: {e}"));
        if *rel == ".claude-plugin/plugin.json" {
            assert_eq!(value["name"], "ctx");
            assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        }
    }

    // hooks.json references the plugin root variable.
    let hooks = fs::read_to_string(root.join("hooks/hooks.json")).unwrap();
    assert!(hooks.contains("${CLAUDE_PLUGIN_ROOT}"), "hooks: {hooks}");

    // Skill has name + description frontmatter.
    let skill = fs::read_to_string(root.join("skills/ctx/SKILL.md")).unwrap();
    assert!(skill.starts_with("---\n"), "skill: {skill}");
    let frontmatter = skill.split("---").nth(1).unwrap();
    assert!(
        frontmatter.contains("name: ctx"),
        "frontmatter: {frontmatter}"
    );
    assert!(
        frontmatter.contains("description:"),
        "frontmatter: {frontmatter}"
    );

    // README documents the manual install walkthrough.
    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    assert!(readme.contains("/plugin marketplace add ./"));
    assert!(readme.contains("/plugin install ctx@ctx-local"));

    // Hook scripts are executable (unix).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["session-start.sh", "post-tool-use.sh", "stop.sh"] {
            let mode = fs::metadata(root.join("hooks").join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "{name} mode: {mode:o}");
        }
    }
}

// ============================================================================
// help/docs sanity
// ============================================================================

#[test]
fn test_harness_help_documents_exit_code_3() {
    let temp = tempfile::tempdir().unwrap();
    let out = ctx(temp.path(), &["harness", "--help"]);
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("init"), "help: {help}");
    assert!(help.contains("compat"), "help: {help}");
    assert!(help.contains("doctor"), "help: {help}");
    assert!(help.contains('3'), "help: {help}");
    assert!(help.contains("version requirement not met"), "help: {help}");
}
