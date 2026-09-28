//! `ctx scip`: precise call edges from a SCIP index (see `ctx::scip`).

use ctx::error::Result;
use ctx::exit::Outcome;
use ctx::scip::{import, ScipOptions, ScipReport};

use crate::cli::ScipCommand;

pub fn run_scip(cmd: ScipCommand, json: bool) -> Result<Outcome> {
    let root = std::env::current_dir()?;
    let db = ctx::index::open_database(&root)?;
    match cmd {
        ScipCommand::Import { path, dry_run } => {
            let report = import(&db, &path, &ScipOptions { dry_run })?;
            if json {
                ctx::json::emit(
                    "scip.import",
                    serde_json::json!({"dry_run": dry_run, "report": report}),
                )?;
            } else {
                print_report(&report, dry_run);
            }
        }
        ScipCommand::Status => {
            let counts = db.call_edge_provenance_counts()?;
            if json {
                let map: serde_json::Map<String, serde_json::Value> = counts
                    .iter()
                    .map(|(k, v)| (k.clone(), (*v).into()))
                    .collect();
                ctx::json::emit("scip.status", serde_json::json!({"call_edges": map}))?;
            } else {
                let total: i64 = counts.iter().map(|(_, n)| n).sum();
                println!("Call edges by resolver ({total}):");
                for (source, n) in &counts {
                    let pct = if total > 0 {
                        100.0 * *n as f64 / total as f64
                    } else {
                        0.0
                    };
                    println!("  {source:5} {n:7}  ({pct:.1}%)");
                }
            }
        }
    }
    Ok(Outcome::Clean)
}

pub fn print_report(r: &ScipReport, dry_run: bool) {
    let verb = if dry_run { "would change" } else { "changed" };
    eprintln!(
        "SCIP answered {} of {} call edges ({} documents) in {:.1}s",
        r.answered, r.call_edges, r.documents, r.seconds
    );
    eprintln!(
        "  {verb}: {} newly bound, {} re-bound, {} unbound as external; {} confirmed",
        r.bound, r.rebound, r.unbound_external, r.unchanged
    );
    eprintln!(
        "  not answered: {} in files the SCIP index lacks, {} without a reference at the call site, {} defined outside a known function",
        r.file_not_indexed, r.no_reference, r.unmapped_definition
    );
    if r.shadowed_package_calls > 0 {
        eprintln!(
            "  warning: {} calls resolve to {}, another copy of this project's own package; \
             the indexer probably saw an installed copy instead of the repository \
             (Python: `pip install -e .` in the indexer's environment). Those calls were unbound as external.",
            r.shadowed_package_calls,
            r.shadowed_package.as_deref().unwrap_or("?")
        );
    }
    if r.unknown_documents > 0 {
        eprintln!(
            "  note: {} SCIP documents are not in the ctx index (different root, or excluded files)",
            r.unknown_documents
        );
    }
}
