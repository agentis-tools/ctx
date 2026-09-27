//! `ctx judge`: model-assisted judgments over the index (see `ctx::judge`).

use std::path::Path;

use ctx::error::Result;
use ctx::exit::Outcome;
use ctx::judge::{judge_edges, JudgeOptions, JudgeReport};

use crate::cli::JudgeCommand;

pub fn run_judge(cmd: JudgeCommand, json: bool) -> Result<Outcome> {
    let root = std::env::current_dir()?;
    match cmd {
        JudgeCommand::Edges {
            offline,
            dry_run,
            limit,
            min_confidence,
            concurrency,
        } => {
            let judge_cfg = ctx::judge::JudgeConfig::load(&root);
            let opts = JudgeOptions {
                offline,
                dry_run,
                limit,
                min_confidence: min_confidence.unwrap_or(judge_cfg.min_confidence),
                concurrency,
                model: judge_cfg.model.clone(),
                verbose: !json,
                ..JudgeOptions::default()
            };
            let report = run_edges(&root, &opts)?;
            if json {
                ctx::json::emit(
                    "judge.edges",
                    serde_json::json!({"dry_run": dry_run, "report": report}),
                )?;
            } else {
                print_report(&report, dry_run);
            }
            Ok(Outcome::Clean)
        }
    }
}

pub fn run_edges(root: &Path, opts: &JudgeOptions) -> Result<JudgeReport> {
    let db = ctx::index::open_database(root)?;
    judge_edges(root, &db, opts)
}

pub fn print_report(r: &JudgeReport, dry_run: bool) {
    let verb = if dry_run { "would change" } else { "changed" };
    eprintln!(
        "Judged {} call edges ({} cached, {} asked, {} failed, {} not asked) in {:.1}s",
        r.considered, r.cached, r.asked, r.failed, r.skipped_uncached, r.seconds
    );
    eprintln!(
        "  {verb}: {} newly bound, {} re-bound, {} unbound as external; {} unchanged, {} below confidence",
        r.bound, r.rebound, r.unbound_external, r.unchanged, r.low_confidence
    );
}
