//! Informational criterion microbenches over the ctx library hot paths.
//!
//! These are NOT part of the CI gate (the gate times the real binary via the
//! perf-harness); they exist for local, iterative optimization work:
//!
//!     cargo bench --manifest-path perf/Cargo.toml
//!
//! Compiled with `default-features = false` on agentis-ctx, so only
//! duckdb-free paths are benched: the incremental no-change `Indexer::index`
//! pass, `Database::find_symbols` (FTS lookup), and
//! `rank::compute_and_cache` (all three are public library API; nothing had
//! to be skipped for visibility).

use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, Criterion};
use ctx::fixture::{self, FixtureSpec};
use ctx::index::{open_database, Indexer};
use ctx::rank;
use ctx::walker::WalkerConfig;

/// Build a tiny fixture repo and index it once; benches then reuse it.
fn setup_fixture() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ctx-perf-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    fixture::generate(&FixtureSpec::tiny(), &dir).expect("fixture generation");
    let mut indexer =
        Indexer::with_config(&dir, false, WalkerConfig::default()).expect("indexer setup");
    indexer.index().expect("initial index");
    dir
}

fn bench_hotpaths(c: &mut Criterion) {
    let dir = setup_fixture();

    // (a) Incremental index() with nothing changed: the fixed cost every
    // hook-path invocation pays before doing real work.
    let mut indexer =
        Indexer::with_config(&dir, false, WalkerConfig::default()).expect("indexer setup");
    c.bench_function("indexer_incremental_no_change", |b| {
        b.iter(|| indexer.index().expect("incremental index"))
    });
    drop(indexer);

    // (b) Symbol FTS lookup.
    let db = open_database(&dir).expect("open database");
    c.bench_function("db_find_symbols", |b| {
        b.iter(|| db.find_symbols("f_0001", 10).expect("find_symbols"))
    });

    // (c) PageRank compute + cache write.
    c.bench_function("rank_compute_and_cache", |b| {
        b.iter(|| rank::compute_and_cache(&db).expect("rank compute"))
    });

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

criterion_group!(benches, bench_hotpaths);
criterion_main!(benches);
