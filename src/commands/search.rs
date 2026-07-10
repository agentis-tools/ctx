//! Search command implementation.
//!
//! Handles semantic/text search for symbols in the codebase.

use std::env;

use ctx::db;
use ctx::error::Result;
use ctx::index;
use ctx::json::SymbolRef;
use ctx::utils::{truncate_path, truncate_str};

/// Run semantic/text search.
pub fn run_search(query: &str, limit: i32, output: &str) -> Result<()> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Use hybrid search combining exact matches with FTS5 semantic search
    let mut results = db.hybrid_search(query, limit)?;

    if results.is_empty() {
        // Fallback to simple name search
        results = db
            .find_symbols(query, limit)?
            .into_iter()
            .map(|s| (s, 0.5, "name".to_string()))
            .collect();
    }

    if output == "json" {
        return ctx::json::emit("search", search_data(query, limit, &results));
    }

    if results.is_empty() {
        eprintln!("No results found for '{}'", query);
        return Ok(());
    }

    print_search_results(&results, query);
    Ok(())
}

/// Build the `search` JSON payload.
fn search_data(
    query: &str,
    limit: i32,
    results: &[(db::Symbol, f64, String)],
) -> serde_json::Value {
    serde_json::json!({
        "query": query,
        "limit": limit,
        "results": results
            .iter()
            .map(|(symbol, score, match_type)| {
                serde_json::json!({
                    "symbol": SymbolRef::from(symbol),
                    "score": score,
                    "match_type": match_type,
                    "signature": symbol.signature,
                    "brief": symbol.brief,
                })
            })
            .collect::<Vec<_>>(),
    })
}

/// Print search results as a human-readable table.
fn print_search_results(results: &[(db::Symbol, f64, String)], query: &str) {
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
        let indicator = match match_type.as_str() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use ctx::db::{Database, FileRecord, Symbol, SymbolKind, Visibility};

    fn seeded_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        db.upsert_file(
            &FileRecord {
                path: "src/lib.rs".to_string(),
                content_hash: "h".to_string(),
                size_bytes: 1,
                language: Some("rust".to_string()),
                last_indexed: 0,
            },
            None,
        )
        .unwrap();
        db.insert_symbol(&Symbol {
            id: "src/lib.rs::parseinput".to_string(),
            file_path: "src/lib.rs".to_string(),
            name: "parseinput".to_string(),
            qualified_name: Some("parseinput".to_string()),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: Some("fn parseinput(s: &str) -> Ast".to_string()),
            brief: Some("Parse raw input".to_string()),
            docstring: None,
            line_start: 4,
            line_end: 20,
            col_start: 0,
            col_end: 1,
            parent_id: None,
            source: None,
        })
        .unwrap();
        db
    }

    #[test]
    fn test_search_data_payload() {
        let db = seeded_db();
        let results = db.hybrid_search("parseinput", 10).unwrap();
        assert!(!results.is_empty());

        let data = search_data("parseinput", 10, &results);
        assert_eq!(data["query"], "parseinput");
        assert_eq!(data["limit"], 10);

        let first = &data["results"][0];
        assert_eq!(first["symbol"]["name"], "parseinput");
        assert_eq!(first["symbol"]["kind"], "function");
        assert_eq!(first["symbol"]["file"], "src/lib.rs");
        assert_eq!(first["symbol"]["line_start"], 4);
        assert_eq!(first["symbol"]["line_end"], 20);
        assert!(first["score"].is_number());
        assert_eq!(first["match_type"], "exact");
        assert_eq!(first["signature"], "fn parseinput(s: &str) -> Ast");
    }

    #[test]
    fn test_search_data_empty_results() {
        let data = search_data("nothing", 5, &[]);
        assert_eq!(data["results"], serde_json::json!([]));
        assert_eq!(data["query"], "nothing");
    }
}
