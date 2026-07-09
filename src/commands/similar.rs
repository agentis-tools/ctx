//! `ctx similar` — find existing functions similar to a description.
//!
//! Thin wrapper over embedding search scoped to callable symbols
//! (functions and methods). Intended agent workflow: before writing a new
//! function, describe its purpose to `ctx similar`; if a strong match with
//! real fan-in exists, extend or reuse it instead of duplicating it.

use std::env;

use ctx::db::{Database, Symbol, SymbolKind};
use ctx::embeddings::{self, Embedding, EmbeddingProvider};
use ctx::error::{CtxError, Result};
use ctx::exit::Outcome;
use ctx::index;
use ctx::json::SymbolRef;
use ctx::utils::{truncate_path, truncate_str};

/// Over-fetch factor: results are requested with `limit * OVERFETCH` and then
/// filtered down to callable kinds, so kind scoping needs no schema changes.
const OVERFETCH: usize = 3;

/// One enriched similarity hit.
struct Hit {
    symbol: Symbol,
    score: f64,
    fan_in: i64,
    brief: String,
}

/// Find functions/methods similar to a natural-language description.
///
/// Exit semantics: `Ok(Outcome::Clean)` on success (even with zero matches);
/// `Err` (exit code 2) when embeddings are missing and `--keyword` was not
/// given, or on any other operational error.
pub fn run_similar(
    query: &str,
    limit: usize,
    keyword: bool,
    use_openai: bool,
    json: bool,
) -> Result<Outcome> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    let (hits, mode) = if keyword {
        (keyword_hits(&db, query, limit)?, "keyword")
    } else {
        ensure_embeddings(&db)?;
        let provider = build_provider(use_openai)?;
        let query_embedding = provider.embed(query)?;
        (semantic_hits(&db, &query_embedding, limit)?, "semantic")
    };

    if json {
        ctx::json::emit("similar", similar_data(query, mode, &hits))?;
        return Ok(Outcome::Clean);
    }

    if hits.is_empty() {
        eprintln!("No similar functions found for '{}'", query);
        return Ok(Outcome::Clean);
    }

    print_hits(query, mode, &hits);
    Ok(Outcome::Clean)
}

/// Fail with an operational error (exit code 2) when no embeddings exist.
fn ensure_embeddings(db: &Database) -> Result<()> {
    if db.count_embeddings()? == 0 {
        return Err(CtxError::Other(
            "No embeddings found. Run 'ctx embed' to generate embeddings, \
             or re-run with --keyword to use FTS-based keyword search \
             (no embeddings required)."
                .to_string(),
        ));
    }
    Ok(())
}

/// Build the embedding provider (local fastembed by default, OpenAI on flag).
fn build_provider(use_openai: bool) -> Result<Box<dyn EmbeddingProvider>> {
    if use_openai {
        let p = embeddings::openai::OpenAIProvider::from_env().map_err(|_| {
            "OPENAI_API_KEY environment variable not set.\n\
             Set it with: export OPENAI_API_KEY=sk-..."
        })?;
        Ok(Box::new(p))
    } else {
        let p = embeddings::local::LocalProvider::new()?;
        Ok(Box::new(p))
    }
}

/// Is this symbol kind in scope for `ctx similar`?
fn is_callable(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Function | SymbolKind::Method)
}

/// Embedding-based hits, scoped to functions/methods.
fn semantic_hits(db: &Database, query_embedding: &Embedding, limit: usize) -> Result<Vec<Hit>> {
    let raw = embeddings::semantic_search(db, query_embedding, limit.saturating_mul(OVERFETCH))?;

    let mut scored: Vec<(Symbol, f64)> = Vec::new();
    for r in raw {
        if scored.len() >= limit {
            break;
        }
        if let Some(symbol) = db.get_symbol(&r.symbol_id)? {
            if is_callable(symbol.kind) {
                scored.push((symbol, f64::from(r.score)));
            }
        }
    }

    enrich(db, scored)
}

/// FTS5/keyword hits (no embeddings or API key needed), scoped to
/// functions/methods. The score is the relevance value returned by
/// `Database::hybrid_search` (see docs/json-output.md).
fn keyword_hits(db: &Database, query: &str, limit: usize) -> Result<Vec<Hit>> {
    let fetch = limit.saturating_mul(OVERFETCH).min(i32::MAX as usize) as i32;
    let raw = db.hybrid_search(query, fetch)?;

    let mut scored: Vec<(Symbol, f64)> = raw
        .into_iter()
        .filter(|(symbol, _, _)| is_callable(symbol.kind))
        .map(|(symbol, score, _match_type)| (symbol, score))
        .collect();
    scored.truncate(limit);

    enrich(db, scored)
}

/// Attach fan-in counts (one batched query) and one-line docs to raw hits.
fn enrich(db: &Database, scored: Vec<(Symbol, f64)>) -> Result<Vec<Hit>> {
    let ids: Vec<String> = scored.iter().map(|(s, _)| s.id.clone()).collect();
    let fan_in = db.fan_in_counts(&ids)?;

    Ok(scored
        .into_iter()
        .map(|(symbol, score)| {
            let fan_in = fan_in.get(&symbol.id).copied().unwrap_or(0);
            let brief = one_line_doc(symbol.brief.as_deref(), symbol.docstring.as_deref());
            Hit {
                symbol,
                score,
                fan_in,
                brief,
            }
        })
        .collect())
}

/// One-line documentation: brief, falling back to the first sentence of the
/// docstring, else empty.
fn one_line_doc(brief: Option<&str>, docstring: Option<&str>) -> String {
    if let Some(b) = brief {
        let b = b.trim();
        if !b.is_empty() {
            return b.to_string();
        }
    }
    if let Some(d) = docstring {
        let d = d.trim();
        if !d.is_empty() {
            return first_sentence(d);
        }
    }
    String::new()
}

/// The first sentence (or first line, whichever ends sooner) of a docstring.
fn first_sentence(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    match line.find(". ") {
        Some(i) => line[..=i].to_string(),
        None => line.to_string(),
    }
}

/// Build the `similar` JSON payload.
fn similar_data(query: &str, mode: &str, hits: &[Hit]) -> serde_json::Value {
    serde_json::json!({
        "query": query,
        "mode": mode,
        "results": hits
            .iter()
            .map(|h| {
                serde_json::json!({
                    "symbol": SymbolRef::from(&h.symbol),
                    "score": h.score,
                    "fan_in": h.fan_in,
                    "brief": h.brief,
                })
            })
            .collect::<Vec<_>>(),
    })
}

/// Print hits as a human-readable table.
fn print_hits(query: &str, mode: &str, hits: &[Hit]) {
    println!(
        "Similar functions for '{}' ({} search, {} results):",
        query,
        mode,
        hits.len()
    );
    println!("{}", "-".repeat(100));
    println!(
        "{:<52} {:<10} {:<7} BRIEF",
        "SYMBOL (FILE:LINE)", "SIMILARITY", "FAN_IN"
    );
    println!("{}", "-".repeat(100));

    for h in hits {
        let location = format!(
            "{} ({}:{})",
            truncate_str(&h.symbol.name, 24),
            truncate_path(&h.symbol.file_path, 20),
            h.symbol.line_start
        );
        println!(
            "{:<52} {:<10.2} {:<7} {}",
            location,
            h.score,
            h.fan_in,
            truncate_str(&h.brief, 40)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx::db::{Edge, EdgeKind, FileRecord, Visibility};

    fn make_symbol(
        name: &str,
        kind: SymbolKind,
        brief: Option<&str>,
        docstring: Option<&str>,
    ) -> Symbol {
        Symbol {
            id: format!("src/lib.rs::{}", name),
            file_path: "src/lib.rs".to_string(),
            name: name.to_string(),
            qualified_name: Some(name.to_string()),
            kind,
            visibility: Visibility::Public,
            signature: Some(format!("fn {}()", name)),
            brief: brief.map(|s| s.to_string()),
            docstring: docstring.map(|s| s.to_string()),
            line_start: 1,
            line_end: 10,
            col_start: 0,
            col_end: 1,
            parent_id: None,
            source: None,
        }
    }

    fn call_edge(source: &str, target: &str) -> Edge {
        Edge {
            source_id: format!("src/lib.rs::{}", source),
            target_id: Some(format!("src/lib.rs::{}", target)),
            target_name: target.to_string(),
            kind: EdgeKind::Calls,
            line: Some(1),
            col: None,
            context: None,
        }
    }

    /// Mixed-kind fixture with hand-inserted embeddings and calls edges.
    ///
    /// Embedding layout (3-dim, small enough to skip the sqlite-vec table and
    /// exercise the cosine-similarity fallback deterministically):
    ///   alpha (function):  [1, 0, 0]   — identical to the test query
    ///   beta (method):     [0.9, 0.1, 0] — close to the query
    ///   shape (struct):    [1, 0, 0]   — perfect score but wrong kind
    ///   delta (function):  [0, 1, 0]   — orthogonal to the query
    fn fixture() -> Database {
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

        db.insert_symbol(&make_symbol(
            "alpha",
            SymbolKind::Function,
            Some("Parse the input"),
            None,
        ))
        .unwrap();
        db.insert_symbol(&make_symbol(
            "beta",
            SymbolKind::Method,
            None,
            Some("Validate a token. Longer detail follows here."),
        ))
        .unwrap();
        db.insert_symbol(&make_symbol("shape", SymbolKind::Struct, None, None))
            .unwrap();
        db.insert_symbol(&make_symbol("delta", SymbolKind::Function, None, None))
            .unwrap();

        db.store_embedding("src/lib.rs::alpha", "test", "default", &[1.0, 0.0, 0.0])
            .unwrap();
        db.store_embedding("src/lib.rs::beta", "test", "default", &[0.9, 0.1, 0.0])
            .unwrap();
        db.store_embedding("src/lib.rs::shape", "test", "default", &[1.0, 0.0, 0.0])
            .unwrap();
        db.store_embedding("src/lib.rs::delta", "test", "default", &[0.0, 1.0, 0.0])
            .unwrap();

        // Fan-in: alpha is called twice, beta once, delta never.
        db.insert_edge(&call_edge("beta", "alpha")).unwrap();
        db.insert_edge(&call_edge("delta", "alpha")).unwrap();
        db.insert_edge(&call_edge("alpha", "beta")).unwrap();

        db
    }

    #[test]
    fn test_semantic_hits_only_functions_and_methods_in_similarity_order() {
        let db = fixture();
        let query = Embedding::new(vec![1.0, 0.0, 0.0]);

        let hits = semantic_hits(&db, &query, 10).unwrap();

        // The struct is excluded even though its vector matches perfectly.
        assert!(hits.iter().all(|h| is_callable(h.symbol.kind)));
        assert!(!hits.iter().any(|h| h.symbol.name == "shape"));

        // Ordering follows cosine similarity: alpha (1.0) > beta > delta (0).
        let names: Vec<&str> = hits.iter().map(|h| h.symbol.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "delta"]);
        assert!(hits[0].score > hits[1].score);
        assert!(hits[1].score > hits[2].score);
        assert!((hits[0].score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_semantic_hits_respects_limit_after_kind_filter() {
        let db = fixture();
        let query = Embedding::new(vec![1.0, 0.0, 0.0]);

        let hits = semantic_hits(&db, &query, 2).unwrap();
        let names: Vec<&str> = hits.iter().map(|h| h.symbol.name.as_str()).collect();
        // The struct occupying an over-fetched slot must not push out beta.
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn test_no_embeddings_is_an_operational_error_mentioning_ctx_embed() {
        let db = Database::open_in_memory().unwrap();

        let err = ensure_embeddings(&db).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ctx embed"), "message was: {}", msg);
        assert!(msg.contains("--keyword"), "message was: {}", msg);

        // Any Err from a command maps to exit code 2 in main() (operational
        // error), unlike Ok(Outcome::Clean)/Ok(Outcome::Findings) which map
        // to 0/1. Returning Err here is what makes `ctx similar` exit 2.
        let result: Result<Outcome> = Err(err);
        assert!(result.is_err());
    }

    #[test]
    fn test_keyword_hits_work_with_zero_embeddings() {
        let db = fixture();
        // Remove all embeddings; the keyword path must still work.
        let deleted = db.delete_embeddings("test", None).unwrap();
        assert!(deleted > 0);
        assert_eq!(db.count_embeddings().unwrap(), 0);

        let hits = keyword_hits(&db, "alpha", 5).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].symbol.name, "alpha");
        assert!(hits.iter().all(|h| is_callable(h.symbol.kind)));
    }

    #[test]
    fn test_keyword_hits_exclude_non_callable_kinds() {
        let db = fixture();
        // "shape" is a struct; an exact-name keyword query must not return it.
        let hits = keyword_hits(&db, "shape", 5).unwrap();
        assert!(!hits.iter().any(|h| h.symbol.name == "shape"));
    }

    #[test]
    fn test_fan_in_matches_inserted_calls_edges() {
        let db = fixture();
        let query = Embedding::new(vec![1.0, 0.0, 0.0]);

        let hits = semantic_hits(&db, &query, 10).unwrap();
        let get = |name: &str| hits.iter().find(|h| h.symbol.name == name).unwrap();

        assert_eq!(get("alpha").fan_in, 2);
        assert_eq!(get("beta").fan_in, 1);
        assert_eq!(get("delta").fan_in, 0);
    }

    #[test]
    fn test_one_line_doc_prefers_brief() {
        assert_eq!(
            one_line_doc(Some("Brief line"), Some("Docstring. More.")),
            "Brief line"
        );
    }

    #[test]
    fn test_one_line_doc_falls_back_to_first_sentence_of_docstring() {
        assert_eq!(
            one_line_doc(None, Some("Validates a token. Extra detail here.")),
            "Validates a token."
        );
        // Multi-line docstring: only the first line is considered.
        assert_eq!(
            one_line_doc(None, Some("First line without period\nsecond line")),
            "First line without period"
        );
        // Blank brief falls through to the docstring.
        assert_eq!(one_line_doc(Some("  "), Some("Doc.")), "Doc.");
    }

    #[test]
    fn test_one_line_doc_empty_when_neither_present() {
        assert_eq!(one_line_doc(None, None), "");
        assert_eq!(one_line_doc(Some(""), Some("   ")), "");
    }

    #[test]
    fn test_similar_data_payload_shape() {
        let db = fixture();
        let query = Embedding::new(vec![1.0, 0.0, 0.0]);
        let hits = semantic_hits(&db, &query, 10).unwrap();

        let data = similar_data("parse input", "semantic", &hits);
        assert_eq!(data["query"], "parse input");
        assert_eq!(data["mode"], "semantic");

        let first = &data["results"][0];
        assert_eq!(first["symbol"]["name"], "alpha");
        assert_eq!(first["symbol"]["kind"], "function");
        assert_eq!(first["symbol"]["file"], "src/lib.rs");
        assert!(first["score"].is_number());
        assert_eq!(first["fan_in"], 2);
        assert_eq!(first["brief"], "Parse the input");
    }

    #[test]
    fn test_similar_data_empty_results_still_has_results_array() {
        let data = similar_data("nothing", "keyword", &[]);
        assert_eq!(data["results"], serde_json::json!([]));
        assert_eq!(data["mode"], "keyword");
    }
}
