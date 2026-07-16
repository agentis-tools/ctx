//! Deterministic, offline regression guard for `ctx smart` file-selection ranking.
//!
//! `ctx smart` normally embeds the task with a downloaded ONNX model (fastembed),
//! which is neither offline nor byte-stable across architectures — so we cannot
//! drive the real CLI in CI. Instead these tests hand-build a tiny on-disk index
//! (files + symbols + edges) and **seed exact embedding vectors** via the library
//! API, then call `ctx::smart::smart_context_with_embedding` directly with a
//! crafted query vector. This exercises the full seed → call-graph expand →
//! lexical → rank → budget pipeline deterministically, with no model download.
//!
//! The scenarios reproduce the two ranking behaviours that motivated the lexical
//! path boost (PR #25):
//!   1. a task token in a file's PATH promotes the on-topic file over a
//!      higher-semantic-scored file whose only "match" is a symbol NAME
//!      (the `ctx` test-helper regression that was originally caught by hand);
//!   2. a graph-only candidate whose path matches the task is surfaced to the top
//!      instead of being outranked by the semantic matches and dropped.
//!
//! Requires the `duckdb` feature: call-graph expansion goes through the real
//! DuckDB-backed `Analytics`. In `--no-default-features` builds the analytics
//! stub returns no edges, so this file compiles to nothing there.
#![cfg(feature = "duckdb")]

use std::path::Path;

use ctx::analytics::Analytics;
use ctx::db::{Database, Edge, EdgeKind, FileRecord, Symbol, SymbolKind, Visibility};
use ctx::embeddings::Embedding;
use ctx::smart::{
    smart_context_with_embedding, smart_context_with_embedding_filtered, SmartConfig,
};
use ctx::walker::FilePatternFilter;
use tempfile::TempDir;

/// One symbol to seed: its file path, symbol name, optional embedding vector
/// (None = present in the index but not a semantic hit), and optional call edge
/// to another symbol id (source of a `Calls` edge, for call-graph expansion).
struct Seed {
    path: &'static str,
    name: &'static str,
    embedding: Option<[f32; 4]>,
    calls: Option<String>, // target symbol id of a `Calls` edge from this symbol
}

fn file_record(path: &str) -> FileRecord {
    FileRecord {
        path: path.to_string(),
        content_hash: "hash".to_string(),
        size_bytes: 0,
        language: Some("rust".to_string()),
        last_indexed: 0,
    }
}

fn symbol(path: &str, name: &str) -> Symbol {
    Symbol {
        id: Symbol::make_id(path, name, None),
        file_path: path.to_string(),
        name: name.to_string(),
        qualified_name: None,
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        signature: None,
        brief: None,
        docstring: None,
        line_start: 1,
        line_end: 2,
        col_start: 0,
        col_end: 0,
        parent_id: None,
        source: None,
    }
}

/// Build a temp `.ctx/codebase.sqlite` from the seeds and return the temp dir
/// (the index root) so the caller can open a fresh reader + Analytics on it.
fn build_fixture(seeds: &[Seed]) -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join(".ctx")).unwrap();
    let db_path = root.join(".ctx").join("codebase.sqlite");

    // Scope the writer so it is dropped (WAL checkpointed) before Analytics
    // attaches the file READ_ONLY.
    {
        let db = Database::open(&db_path).unwrap();

        // De-duplicate file rows (several symbols may share a file).
        let mut seen_files = std::collections::BTreeSet::new();
        for s in seeds {
            if seen_files.insert(s.path) {
                db.upsert_file(&file_record(s.path), None).unwrap();
            }
        }
        for s in seeds {
            db.insert_symbol(&symbol(s.path, s.name)).unwrap();
        }
        for s in seeds {
            if let Some(target_id) = &s.calls {
                db.insert_edge(&Edge {
                    source_id: Symbol::make_id(s.path, s.name, None),
                    target_id: Some(target_id.clone()),
                    target_name: "callee".to_string(),
                    kind: EdgeKind::Calls,
                    line: Some(1),
                    col: None,
                    context: None,
                })
                .unwrap();
            }
        }
        for s in seeds {
            if let Some(vec) = s.embedding {
                db.store_embedding(
                    &Symbol::make_id(s.path, s.name, None),
                    "test",
                    "default",
                    &vec,
                )
                .unwrap();
            }
        }
    }

    temp
}

/// Ranked paths that `ctx smart` selects for `task` against the fixture at `root`,
/// using `query` as the (seeded) task embedding.
fn ranked_paths(root: &Path, task: &str, query: [f32; 4]) -> Vec<String> {
    let db = Database::open(&root.join(".ctx").join("codebase.sqlite")).unwrap();
    let analytics = Analytics::open(root).unwrap();
    let embedding = Embedding::new(query.to_vec());
    let result =
        smart_context_with_embedding(&db, &analytics, task, &embedding, SmartConfig::default())
            .unwrap();
    result.selected_files.into_iter().map(|f| f.path).collect()
}

fn rank_of(paths: &[String], path: &str) -> Option<usize> {
    paths.iter().position(|p| p == path)
}

/// Regression guard: a task token in a file's PATH must promote the on-topic
/// file over a file that scores higher semantically and merely contains a symbol
/// *named* like a task token. This is the `ctx`-helper case: `tests/*_cli.rs`
/// define a helper `ctx`, and "ctx" is in the task — but matching symbol NAMES
/// (rather than paths) let those test files displace `commands/sql.rs`.
#[test]
fn path_match_beats_symbol_name_noise() {
    // Fixture paths use a `fixture/` prefix so they do NOT exist on disk relative
    // to the test's CWD — token counts are a deterministic 0 and the budget never
    // interferes with the ordering under test.
    let seeds = [
        // Highest semantic score, symbol literally named "ctx", path has no task token.
        Seed {
            path: "fixture/tests/foo_cli.rs",
            name: "ctx",
            embedding: Some([1.0, 0.0, 0.0, 0.0]),
            calls: None,
        },
        // Lower semantic score, but its PATH contains the task token "sql".
        Seed {
            path: "fixture/commands/sql.rs",
            name: "run_sql",
            embedding: Some([0.8, 0.6, 0.0, 0.0]),
            calls: None,
        },
        // Off-topic filler.
        Seed {
            path: "fixture/other.rs",
            name: "helper",
            embedding: Some([0.6, 0.8, 0.0, 0.0]),
            calls: None,
        },
    ];
    let temp = build_fixture(&seeds);

    // Query closest to the "ctx" symbol: semantic order is foo_cli > sql > other.
    let paths = ranked_paths(
        temp.path(),
        "add output format to ctx sql",
        [1.0, 0.0, 0.0, 0.0],
    );

    let sql = rank_of(&paths, "fixture/commands/sql.rs");
    let noise = rank_of(&paths, "fixture/tests/foo_cli.rs");
    assert!(sql.is_some(), "on-topic sql.rs must be selected: {paths:?}");
    assert!(
        noise.is_some(),
        "foo_cli.rs should still be selected: {paths:?}"
    );
    assert!(
        sql < noise,
        "path match (commands/sql.rs) must outrank the higher-semantic symbol-name \
         noise (tests/foo_cli.rs); got {paths:?}. If this fails, lexical scoring is \
         matching symbol names instead of the path."
    );
    assert_eq!(
        paths.first().map(String::as_str),
        Some("fixture/commands/sql.rs"),
        "the path-matched file should rank first: {paths:?}"
    );
}

/// A candidate reachable only through call-graph expansion (never a direct
/// semantic hit) whose PATH matches the task must be promoted to the top, rather
/// than sitting below every semantic match. Guards the lexical tier-promotion
/// that surfaces `embeddings/openai.rs` for "…openai".
#[test]
fn graph_only_path_match_is_promoted() {
    let openai_id = Symbol::make_id("fixture/embeddings/openai.rs", "call_api", None);
    let seeds = [
        // Semantic hit that CALLS the openai symbol; its path matches "embeddings".
        Seed {
            path: "fixture/embeddings/mod.rs",
            name: "embed_all",
            embedding: Some([1.0, 0.0, 0.0, 0.0]),
            calls: Some(openai_id.clone()),
        },
        // Graph-only: no embedding, so it is never a semantic hit — it enters only
        // via the Calls edge above. Its path contains "embeddings" AND "openai".
        Seed {
            path: "fixture/embeddings/openai.rs",
            name: "call_api",
            embedding: None,
            calls: None,
        },
        // Another semantic hit with no task-token path overlap.
        Seed {
            path: "fixture/other.rs",
            name: "helper",
            embedding: Some([0.7, 0.714, 0.0, 0.0]),
            calls: None,
        },
    ];
    let temp = build_fixture(&seeds);

    let paths = ranked_paths(
        temp.path(),
        "generate embeddings with openai",
        [1.0, 0.0, 0.0, 0.0],
    );

    assert_eq!(
        paths.first().map(String::as_str),
        Some("fixture/embeddings/openai.rs"),
        "the graph-only file whose path matches two task tokens should rank first; got {paths:?}"
    );
    // And it must rank above the semantic match that expanded it.
    let openai = rank_of(&paths, "fixture/embeddings/openai.rs");
    let modrs = rank_of(&paths, "fixture/embeddings/mod.rs");
    assert!(
        openai < modrs,
        "openai.rs (2 path hits) must outrank embeddings/mod.rs (1 path hit): {paths:?}"
    );
}

#[test]
fn patterns_iteratively_overfetch_seeds_and_filter_graph_expansion() {
    let outside_callee = Symbol::make_id("fixture/docs/outside.rs", "outside_callee", None);
    let seeds = [
        Seed {
            path: "fixture/tests/high_score.rs",
            name: "high_score",
            embedding: Some([1.0, 0.0, 0.0, 0.0]),
            calls: None,
        },
        Seed {
            path: "fixture/tests/high_score_two.rs",
            name: "high_score_two",
            embedding: Some([0.99, 0.1, 0.0, 0.0]),
            calls: None,
        },
        Seed {
            path: "fixture/tests/high_score_three.rs",
            name: "high_score_three",
            embedding: Some([0.95, 0.2, 0.0, 0.0]),
            calls: None,
        },
        Seed {
            path: "fixture/src/in_scope.rs",
            name: "in_scope",
            embedding: Some([0.8, 0.6, 0.0, 0.0]),
            calls: Some(outside_callee),
        },
        Seed {
            path: "fixture/docs/outside.rs",
            name: "outside_callee",
            embedding: None,
            calls: None,
        },
    ];
    let temp = build_fixture(&seeds);
    let db = Database::open(&temp.path().join(".ctx/codebase.sqlite")).unwrap();
    let analytics = Analytics::open(temp.path()).unwrap();
    let filter = FilePatternFilter::new(temp.path(), &["fixture/src".to_string()]).unwrap();
    let config = SmartConfig {
        top: 1,
        ..SmartConfig::default()
    };

    let result = smart_context_with_embedding_filtered(
        &db,
        &analytics,
        "find scoped code",
        &Embedding::new(vec![1.0, 0.0, 0.0, 0.0]),
        config,
        &filter,
    )
    .unwrap();
    let paths: Vec<_> = result
        .selected_files
        .iter()
        .map(|file| file.path.as_str())
        .collect();

    assert_eq!(paths, vec!["fixture/src/in_scope.rs"]);
}
