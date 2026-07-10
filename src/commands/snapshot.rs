//! `ctx snapshot` -- per-commit Parquet metric snapshot CLI.
//!
//! Thin wrapper around the [`ctx::snapshot`] engine: bare `ctx snapshot`
//! captures HEAD into `.ctx/snapshots/sha=<sha>/`; `ctx snapshot backfill`
//! captures historical commits via temporary git worktrees.
//!
//! Exit codes: 0 = success (including "partition already exists"),
//! 2 = operational error (not a git repo, stub build, IO failure).

use ctx::error::Result;
use ctx::exit::Outcome;
#[cfg(feature = "duckdb")]
use ctx::snapshot::{BackfillOptions, CaptureMode, CaptureOptions, SnapshotReport, SNAPSHOTS_DIR};

use crate::cli::SnapshotCommand;

/// Run `ctx snapshot [backfill]` in the current directory.
// The flag arguments are only read on the `duckdb` execution path.
#[cfg_attr(not(feature = "duckdb"), allow(unused_variables))]
pub fn run_snapshot(
    cmd: Option<SnapshotCommand>,
    force: bool,
    churn_window: &str,
    json: bool,
) -> Result<Outcome> {
    #[cfg(not(feature = "duckdb"))]
    {
        Err(ctx::error::CtxError::Other(
            "ctx snapshot requires the duckdb feature; reinstall with default features".into(),
        ))
    }

    #[cfg(feature = "duckdb")]
    {
        let root = std::env::current_dir()?;
        let out_dir = root.join(SNAPSHOTS_DIR);
        match cmd {
            None => {
                // A live snapshot is labeled with HEAD's sha but reflects the
                // working tree; warn when the two can disagree.
                if ctx::gitutil::is_dirty_in(&root).unwrap_or(false) {
                    eprintln!(
                        "warning: working tree is dirty; the snapshot is labeled with \
                         HEAD's sha but reflects the working tree"
                    );
                }
                let report = ctx::snapshot::capture(
                    &root,
                    &CaptureOptions {
                        out_dir,
                        churn_window: churn_window.to_string(),
                        capture_mode: CaptureMode::Live,
                        force,
                    },
                )?;
                if json {
                    ctx::json::emit("snapshot.capture", serde_json::to_value(&report)?)?;
                } else {
                    print_report(&report);
                }
                Ok(Outcome::Clean)
            }
            Some(SnapshotCommand::Backfill {
                since,
                every,
                churn_window,
            }) => {
                let reports = ctx::snapshot::backfill(
                    &root,
                    &BackfillOptions {
                        since: since.clone(),
                        every,
                        churn_window,
                        out_dir,
                    },
                )?;
                if json {
                    let captured = reports.iter().filter(|r| !r.skipped_existing).count();
                    let skipped = reports.len() - captured;
                    ctx::json::emit(
                        "snapshot.backfill",
                        serde_json::json!({
                            "since": since,
                            "captured": captured,
                            "skipped_existing": skipped,
                            "snapshots": serde_json::to_value(&reports)?,
                        }),
                    )?;
                } else {
                    for report in &reports {
                        print_report(report);
                    }
                    println!(
                        "backfilled {} snapshot{}",
                        reports.len(),
                        if reports.len() == 1 { "" } else { "s" }
                    );
                }
                Ok(Outcome::Clean)
            }
        }
    }
}

/// One concise human line per partition.
#[cfg(feature = "duckdb")]
fn print_report(report: &SnapshotReport) {
    let short = &report.commit_sha[..12.min(report.commit_sha.len())];
    if report.skipped_existing {
        println!(
            "snapshot {} -> {} (exists, skipped; use --force to rewrite)",
            short, report.partition_dir
        );
    } else {
        println!(
            "snapshot {} -> {} (files {}, symbols {}, dup_pairs {}, violations {})",
            short,
            report.partition_dir,
            report.files,
            report.symbols,
            report.dup_pairs,
            report.violations
        );
    }
}
