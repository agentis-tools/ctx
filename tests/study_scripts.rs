//! Tests for the longitudinal-study helper scripts (`scripts/rework-rate.sh`,
//! `scripts/revert-rate.sh`) and the benchmark run-record schema
//! (`docs/benchmark/run-record.schema.json` / `run-record.md`).

#![cfg(unix)]

use std::path::Path;
use std::process::{Command, Output};

use ctx::testutil::GitRepo;

const REWORK_SH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/rework-rate.sh");
const REVERT_SH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/revert-rate.sh");

/// Run a study script with `sh` against `repo_dir`, asserting success.
fn run_script(script: &str, args: &[&str], repo_dir: &Path) -> String {
    let output: Output = Command::new("sh")
        .arg(script)
        .args(args)
        .arg(repo_dir)
        .output()
        .expect("failed to spawn sh");
    assert!(
        output.status.success(),
        "script {script} failed (status {:?}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("script output was not UTF-8")
}

/// Run a raw git command inside the repo (for operations GitRepo doesn't
/// wrap: checkout, merge, revert), with optional fixed commit dates.
fn git(repo: &GitRepo, args: &[&str], date: Option<&str>) {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(&repo.root);
    if let Some(d) = date {
        cmd.env("GIT_AUTHOR_DATE", d).env("GIT_COMMITTER_DATE", d);
    }
    let output = cmd.output().expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Full sha of HEAD.
fn head_sha(repo: &GitRepo) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo.root)
        .output()
        .expect("failed to spawn git");
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn lines_of(f: &[&str]) -> String {
    let mut s = f.join("\n");
    s.push('\n');
    s
}

// ============================================================================
// rework-rate.sh
// ============================================================================

/// Hand-checkable fixture (window = 30 days, all dates 12:00 UTC):
///
/// * day 0  (2024-01-01): A, root commit, adds a 10-line file
///   (root-commit handling: diffed against the git empty tree, added = 10).
/// * day 10 (2024-01-11): B rewrites 4 of A's lines (added = 4).
/// * day 55 (2024-02-25): C rewrites 2 more of A's lines (added = 2).
/// * day 90 (2024-03-31): D, anchor, adds an unrelated file.
///
/// Newest commit date is day 90, so the completeness cutoff is day 60:
/// A, B, C are measured; D is excluded (incomplete window).
///
/// * A: window ends day 30, boundary = B; 4 of A's 10 lines were rewritten
///   by B inside the window (C's rewrite is outside) -> 10 added,
///   6 surviving, rework 0.4000.
/// * B: window ends day 40, boundary = B itself -> 4 added, 4 surviving,
///   rework 0.0000. B's own window is complete (day 10 <= day 60 cutoff),
///   so B must be INCLUDED.
/// * C: window ends day 85, boundary = C itself -> 2 added, 2 surviving,
///   rework 0.0000.
///
/// Aggregate: added 16, surviving 12, fraction 4/16 = 0.2500.
#[test]
fn rework_rate_measures_window_rework_and_handles_root_commit() {
    let temp = tempfile::tempdir().unwrap();
    let repo = GitRepo::init(temp.path());

    repo.write(
        "src.txt",
        &lines_of(&["l1", "l2", "l3", "l4", "l5", "l6", "l7", "l8", "l9", "l10"]),
    );
    repo.commit_all_with_date("A: add ten lines", "2024-01-01T12:00:00 +0000");
    let sha_a = head_sha(&repo);

    repo.write(
        "src.txt",
        &lines_of(&["x1", "x2", "x3", "x4", "l5", "l6", "l7", "l8", "l9", "l10"]),
    );
    repo.commit_all_with_date("B: rewrite four lines", "2024-01-11T12:00:00 +0000");
    let sha_b = head_sha(&repo);

    repo.write(
        "src.txt",
        &lines_of(&["x1", "x2", "x3", "x4", "y5", "y6", "l7", "l8", "l9", "l10"]),
    );
    repo.commit_all_with_date("C: rewrite two more lines", "2024-02-25T12:00:00 +0000");
    let sha_c = head_sha(&repo);

    repo.write("anchor.txt", "anchor\n");
    repo.commit_all_with_date("D: anchor", "2024-03-31T12:00:00 +0000");
    let sha_d = head_sha(&repo);

    let stdout = run_script(REWORK_SH, &["--window-days", "30"], temp.path());
    let lines: Vec<&str> = stdout.lines().collect();

    assert_eq!(lines[0], "commit\tadded\tsurviving\trework_fraction");
    // Rows are emitted newest-first over the qualifying set: C, B, A.
    assert_eq!(lines[1], format!("{sha_c}\t2\t2\t0.0000"));
    assert_eq!(
        lines[2],
        format!("{sha_b}\t4\t4\t0.0000"),
        "B's own window is complete; it must be included"
    );
    assert_eq!(
        lines[3],
        format!("{sha_a}\t10\t6\t0.4000"),
        "root commit A: 10 added via empty-tree base, 4 reworked in window"
    );
    assert_eq!(lines[4], "# aggregate\t16\t12\t0.2500");
    assert_eq!(lines.len(), 5, "unexpected extra rows:\n{stdout}");
    assert!(
        !stdout.contains(&sha_d),
        "D's window is incomplete; it must be excluded"
    );
}

/// Merge commits and formatting-only (whitespace) commits must not appear
/// as output rows.
#[test]
fn rework_rate_excludes_merges_and_whitespace_only_commits() {
    let temp = tempfile::tempdir().unwrap();
    let repo = GitRepo::init(temp.path());

    repo.write("code.txt", "alpha\nbeta\n");
    repo.commit_all_with_date("base", "2024-01-01T12:00:00 +0000");
    let sha_base = head_sha(&repo);

    // Whitespace-only change: indentation only.
    repo.write("code.txt", "  alpha\n  beta\n");
    repo.commit_all_with_date("ws: indent only", "2024-01-02T12:00:00 +0000");
    let sha_ws = head_sha(&repo);

    // Side branch merged back with --no-ff -> a merge commit on main.
    repo.branch("feature");
    repo.write("feature.txt", "feature work\n");
    repo.commit_all_with_date("feature commit", "2024-01-03T12:00:00 +0000");
    let sha_feature = head_sha(&repo);
    git(&repo, &["checkout", "-q", "main"], None);
    repo.write("main.txt", "main work\n");
    repo.commit_all_with_date("main side", "2024-01-04T12:00:00 +0000");
    let sha_main_side = head_sha(&repo);
    git(
        &repo,
        &["merge", "--no-ff", "--no-edit", "-q", "feature"],
        Some("2024-01-05T12:00:00 +0000"),
    );
    let sha_merge = head_sha(&repo);

    // window-days 0: every commit's window is complete, nothing excluded
    // for recency -- only the merge/whitespace rules apply.
    let stdout = run_script(REWORK_SH, &["--window-days", "0"], temp.path());

    assert!(
        stdout.contains(&sha_base),
        "normal commit missing:\n{stdout}"
    );
    assert!(
        stdout.contains(&sha_main_side),
        "normal commit missing:\n{stdout}"
    );
    assert!(
        !stdout.contains(&sha_ws),
        "whitespace-only commit must be skipped:\n{stdout}"
    );
    assert!(
        !stdout.contains(&sha_merge),
        "merge commit must be skipped:\n{stdout}"
    );
    assert!(
        !stdout.contains(&sha_feature),
        "side-branch commit is not on the first-parent chain:\n{stdout}"
    );
}

// ============================================================================
// revert-rate.sh
// ============================================================================

/// 5 first-parent commits: initial, feature, Revert "feature", fix, decoy.
/// -> 1 revert, 1 fix, 20.00 per 100 each.
#[test]
fn revert_rate_counts_reverts_and_fixes() {
    let temp = tempfile::tempdir().unwrap();
    let repo = GitRepo::init(temp.path());

    repo.commit_file("a.txt", "one\n", "initial commit");
    repo.commit_file("b.txt", "two\n", "add feature");
    let feature_sha = head_sha(&repo);
    // git revert produces subject `Revert "add feature"` and a body with
    // "This reverts commit <sha>."
    git(&repo, &["revert", "--no-edit", &feature_sha], None);
    repo.commit_file("c.txt", "three\n", "fix: something");
    // Decoy: contains "fix" but not anchored at the start of the subject.
    repo.commit_file("d.txt", "four\n", "prefix nofix");

    let stdout = run_script(REVERT_SH, &[], temp.path());
    let lines: Vec<&str> = stdout.lines().collect();

    assert_eq!(
        lines[0],
        "total_commits\treverts\tfixes\treverts_per_100\tfixes_per_100"
    );
    assert_eq!(lines[1], "5\t1\t1\t20.00\t20.00");
    assert_eq!(lines.len(), 2, "unexpected extra output:\n{stdout}");
}

// ============================================================================
// run-record schema drift guard
// ============================================================================

const SCHEMA_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/docs/benchmark/run-record.schema.json"
);
const DOC_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/benchmark/run-record.md");

fn required_keys(obj: &serde_json::Value) -> Vec<String> {
    obj["required"]
        .as_array()
        .expect("`required` must be an array")
        .iter()
        .map(|v| {
            v.as_str()
                .expect("required entries are strings")
                .to_string()
        })
        .collect()
}

/// The schema must parse, its `required` list must cover the documented
/// fields, and the example record in run-record.md must contain every
/// required key (top-level and the gate_config / metrics / metrics.score
/// subobjects).
#[test]
fn run_record_schema_and_example_stay_in_sync() {
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(SCHEMA_PATH).expect("read schema"))
            .expect("run-record.schema.json must be valid JSON");

    assert_eq!(
        schema["$id"],
        "https://docs.agentis.tools/schemas/run-record-v1.json"
    );
    assert_eq!(schema["properties"]["schema_version"]["const"], 1);

    let required = required_keys(&schema);
    let documented = [
        "schema_version",
        "run_id",
        "task_id",
        "arm",
        "run_index",
        "started_at",
        "finished_at",
        "model_version",
        "ctx_version",
        "harness_mode",
        "gate_config",
        "transcript_path",
        "exit_status",
        "metrics",
    ];
    for field in documented {
        assert!(
            required.iter().any(|r| r == field),
            "schema `required` is missing documented field `{field}`"
        );
    }
    assert!(
        !required.iter().any(|r| r == "notes"),
        "`notes` is documented as optional and must not be required"
    );

    // Extract the ```json fenced example from run-record.md.
    let doc = std::fs::read_to_string(DOC_PATH).expect("read run-record.md");
    let start = doc
        .find("```json")
        .expect("run-record.md must contain a ```json block")
        + "```json".len();
    let end = doc[start..]
        .find("```")
        .expect("unterminated ```json block")
        + start;
    let example: serde_json::Value =
        serde_json::from_str(&doc[start..end]).expect("example record must be valid JSON");

    for key in &required {
        assert!(
            example.get(key).is_some(),
            "example record is missing required key `{key}`"
        );
    }

    // Nested required keys: gate_config, metrics, metrics.score.
    for (schema_obj, example_obj, name) in [
        (
            &schema["properties"]["gate_config"],
            &example["gate_config"],
            "gate_config",
        ),
        (
            &schema["properties"]["metrics"],
            &example["metrics"],
            "metrics",
        ),
        (
            &schema["properties"]["metrics"]["properties"]["score"],
            &example["metrics"]["score"],
            "metrics.score",
        ),
    ] {
        for key in required_keys(schema_obj) {
            assert!(
                example_obj.get(&key).is_some(),
                "example `{name}` is missing required key `{key}`"
            );
        }
    }

    // The score object documents exactly the seven ctx score metrics.
    let score_required = required_keys(&schema["properties"]["metrics"]["properties"]["score"]);
    let expected_score = [
        "complexity_delta",
        "fan_out_delta",
        "new_duplication",
        "check_violations",
        "symbols_added",
        "symbols_removed",
        "files_changed",
    ];
    assert_eq!(score_required.len(), expected_score.len());
    for key in expected_score {
        assert!(
            score_required.iter().any(|r| r == key),
            "score is missing metric `{key}`"
        );
    }
}
