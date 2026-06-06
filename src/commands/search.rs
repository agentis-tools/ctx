//! Search command implementation.
//!
//! Handles semantic/text search for symbols in the codebase.

use std::env;

use ctx::db;
use ctx::error::Result;
use ctx::index;
use ctx::utils::{truncate_path, truncate_str};

/// Run semantic/text search.
pub fn run_search(query: &str, limit: i32, output: &str) -> Result<()> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Use hybrid search combining exact matches with FTS5 semantic search
    let results = db.hybrid_search(query, limit)?;

    if results.is_empty() {
        // Fallback to simple name search
        let symbols = db.find_symbols(query, limit)?;
        if symbols.is_empty() {
            eprintln!("No results found for '{}'", query);
            return Ok(());
        }

        // Convert to format with scores
        let results: Vec<_> = symbols.iter().map(|s| (s, 0.5, "name")).collect();

        print_search_results(&results, query, output)?;
        return Ok(());
    }

    // Convert references for printing
    let results_ref: Vec<_> = results
        .iter()
        .map(|(s, score, match_type)| (s, *score, match_type.as_str()))
        .collect();

    print_search_results(&results_ref, query, output)?;

    Ok(())
}

/// Print search results in the specified format.
fn print_search_results(
    results: &[(&db::Symbol, f64, &str)],
    query: &str,
    output: &str,
) -> Result<()> {
    if output == "json" {
        let json_results: Vec<_> = results
            .iter()
            .map(|(s, score, match_type)| {
                serde_json::json!({
                    "name": s.name,
                    "kind": s.kind.as_str(),
                    "file": s.file_path,
                    "line": s.line_start,
                    "signature": s.signature,
                    "brief": s.brief,
                    "relevance": format!("{:.2}", score),
                    "match_type": match_type,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_results)?);
    } else {
        println!(
            "Search results for '{}' ({} matches):",
            query,
            results.len()
        );
        println!("{}", "-".repeat(75));
        println!("{:<40} {:<8} {:<6} FILE", "SYMBOL", "KIND", "SCORE");
        println!("{}", "-".repeat(75));

        for (symbol, score, match_type) in results {
            let name = truncate_str(&symbol.name, 38);
            let file = truncate_path(&symbol.file_path, 25);

            let score_display = format!("{:.0}%", score * 100.0);
            let kind_display = symbol.kind.as_str().to_string();

            println!(
                "{:<40} {:<8} {:<6} {}:{}",
                name, kind_display, score_display, file, symbol.line_start
            );

            // Show match type indicator
            let indicator = match *match_type {
                "exact" => "[exact]",
                "semantic" => "[semantic]",
                _ => "[name]",
            };

            if let Some(sig) = &symbol.signature {
                let sig_short = truncate_str(sig, 70);
                println!("  {} {}", indicator, sig_short);
            }

            if let Some(brief) = &symbol.brief {
                let brief_short = truncate_str(brief, 70);
                println!("  # {}", brief_short);
            }
            println!();
        }
    }

    Ok(())
}
