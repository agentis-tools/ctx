//! PageRank scoring over the symbol graph.
//!
//! Powers `ctx map`: symbols that many other symbols reference (via
//! resolved `calls`, `imports`, `extends`, and `implements` edges) rank
//! higher and are emitted first when the token budget is tight.
//!
//! Scores are cached in the `symbol_rank` table. The indexer clears the
//! cache whenever the index changes; [`is_stale`] detects a cleared (or
//! partially cascaded) cache so `ctx map` can lazily recompute.
//!
//! Determinism: node order is the ascending symbol-ID order, edges are
//! deduplicated and sorted, and all floating-point accumulation happens in
//! that fixed order, so identical index state always yields bit-identical
//! ranks.

use std::collections::{HashMap, HashSet};

use crate::db::Database;
use crate::error::Result;

/// PageRank damping factor.
pub const DAMPING: f64 = 0.85;

/// Convergence threshold on the L1 delta between iterations.
pub const CONVERGENCE: f64 = 1e-6;

/// Maximum number of power iterations.
pub const MAX_ITERATIONS: usize = 100;

/// Multiplier applied to focused symbols and their 1-hop neighbors.
pub const FOCUS_BOOST: f64 = 10.0;

/// Check whether the cached ranks are stale.
///
/// The cache is stale when the number of cached scores differs from the
/// number of symbols: the indexer deletes all rows on reindex, and foreign
/// key cascades remove rows for deleted symbols.
pub fn is_stale(db: &Database) -> Result<bool> {
    Ok(db.count_symbol_ranks()? != db.count_symbols()?)
}

/// Compute PageRank over the symbol graph and cache the scores in the
/// `symbol_rank` table (replacing any existing cache) in one transaction.
pub fn compute_and_cache(db: &Database) -> Result<()> {
    let ids = db.get_all_symbol_ids()?;
    let ranks = compute_ranks(&ids, &db.get_rank_edges()?);
    let rows: Vec<(String, f64)> = ids.into_iter().zip(ranks).collect();
    db.store_symbol_ranks(&rows)?;
    Ok(())
}

/// Load the cached ranks as a symbol-ID -> rank map.
pub fn load_ranks(db: &Database) -> Result<HashMap<String, f64>> {
    Ok(db.load_symbol_ranks()?.into_iter().collect())
}

/// Boost focused symbols and their 1-hop neighbors (both directions) by
/// [`FOCUS_BOOST`], then renormalize so ranks sum to 1 again.
///
/// The boost is applied in memory only; the persisted cache is untouched.
pub fn apply_focus(
    db: &Database,
    ranks: &mut HashMap<String, f64>,
    focus_ids: &HashSet<String>,
) -> Result<()> {
    if focus_ids.is_empty() || ranks.is_empty() {
        return Ok(());
    }

    // Expand the focus set with 1-hop neighbors in both directions.
    let mut boosted: HashSet<String> = focus_ids.clone();
    for (source, target) in db.get_rank_edges()? {
        if focus_ids.contains(&source) {
            boosted.insert(target.clone());
        }
        if focus_ids.contains(&target) {
            boosted.insert(source);
        }
    }

    for id in &boosted {
        if let Some(rank) = ranks.get_mut(id) {
            *rank *= FOCUS_BOOST;
        }
    }

    // Renormalize, summing in fixed (sorted-key) order for determinism.
    let mut keys: Vec<&String> = ranks.keys().collect();
    keys.sort();
    let total: f64 = keys.iter().map(|k| ranks[*k]).sum();
    if total > 0.0 {
        for rank in ranks.values_mut() {
            *rank /= total;
        }
    }

    Ok(())
}

/// Power-iteration PageRank with dangling-node mass redistribution:
///
/// ```text
/// r'[j] = (1 - d + d * dangling_mass) / N + d * sum_{i -> j} r[i] / outdeg(i)
/// ```
///
/// `ids` must be sorted ascending; edges referencing unknown IDs are ignored.
/// All accumulation happens in fixed node-index order, so the result is a
/// deterministic function of the inputs.
fn compute_ranks(ids: &[String], edges: &[(String, String)]) -> Vec<f64> {
    let n = ids.len();
    if n == 0 {
        return Vec::new();
    }
    let nf = n as f64;

    // Dense index map (ids are sorted ascending).
    let index: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();

    // Build CSR adjacency (offsets + sorted, deduplicated targets).
    let mut pairs: Vec<(usize, usize)> = edges
        .iter()
        .filter_map(|(s, t)| Some((*index.get(s.as_str())?, *index.get(t.as_str())?)))
        .collect();
    pairs.sort_unstable();
    pairs.dedup();

    let mut offsets = vec![0usize; n + 1];
    for &(source, _) in &pairs {
        offsets[source + 1] += 1;
    }
    for i in 0..n {
        offsets[i + 1] += offsets[i];
    }
    let targets: Vec<usize> = pairs.iter().map(|&(_, t)| t).collect();

    let mut rank = vec![1.0 / nf; n];
    let mut next = vec![0.0f64; n];

    for _ in 0..MAX_ITERATIONS {
        // Dangling mass, accumulated in fixed node order.
        let mut dangling_mass = 0.0f64;
        for i in 0..n {
            if offsets[i] == offsets[i + 1] {
                dangling_mass += rank[i];
            }
        }

        let base = (1.0 - DAMPING + DAMPING * dangling_mass) / nf;
        next.fill(base);

        // Distribute rank along edges in fixed source order.
        for i in 0..n {
            let out = &targets[offsets[i]..offsets[i + 1]];
            if out.is_empty() {
                continue;
            }
            let share = DAMPING * rank[i] / out.len() as f64;
            for &j in out {
                next[j] += share;
            }
        }

        let mut delta = 0.0f64;
        for i in 0..n {
            delta += (next[i] - rank[i]).abs();
        }
        std::mem::swap(&mut rank, &mut next);
        if delta < CONVERGENCE {
            break;
        }
    }

    rank
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Edge, EdgeKind, FileRecord, Symbol, SymbolKind, Visibility};

    fn insert_symbol(db: &Database, file: &str, name: &str, line: u32) -> String {
        let id = format!("{}::{}@{}", file, name, line);
        db.insert_symbol(&Symbol {
            id: id.clone(),
            file_path: file.to_string(),
            name: name.to_string(),
            qualified_name: None,
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: Some(format!("fn {}()", name)),
            brief: None,
            docstring: None,
            line_start: line,
            line_end: line + 2,
            col_start: 0,
            col_end: 1,
            parent_id: None,
            source: None,
        })
        .unwrap();
        id
    }

    fn insert_call(db: &Database, source_id: &str, target_id: &str) {
        db.insert_edge(&Edge {
            source_id: source_id.to_string(),
            target_id: Some(target_id.to_string()),
            target_name: target_id.rsplit("::").next().unwrap().to_string(),
            kind: EdgeKind::Calls,
            line: Some(1),
            col: None,
            context: None,
        })
        .unwrap();
    }

    /// 5-node graph: a, b, c, d all call hub; hub is dangling.
    fn hub_graph() -> (Database, String) {
        let db = Database::open_in_memory().unwrap();
        db.upsert_file(
            &FileRecord {
                path: "src/lib.rs".to_string(),
                content_hash: "h".to_string(),
                size_bytes: 100,
                language: Some("rust".to_string()),
                last_indexed: 0,
            },
            None,
        )
        .unwrap();

        let hub = insert_symbol(&db, "src/lib.rs", "hub", 1);
        for (i, name) in ["a", "b", "c", "d"].iter().enumerate() {
            let id = insert_symbol(&db, "src/lib.rs", name, 10 + i as u32 * 10);
            insert_call(&db, &id, &hub);
        }
        (db, hub)
    }

    #[test]
    fn test_hub_ranks_first() {
        let (db, hub) = hub_graph();
        compute_and_cache(&db).unwrap();
        let ranks = load_ranks(&db).unwrap();
        assert_eq!(ranks.len(), 5);

        let hub_rank = ranks[&hub];
        for (id, rank) in &ranks {
            assert!(rank.is_finite(), "rank for {} is not finite", id);
            if id != &hub {
                assert!(
                    hub_rank > *rank,
                    "hub ({}) should outrank {} ({})",
                    hub_rank,
                    id,
                    rank
                );
            }
        }
    }

    #[test]
    fn test_dangling_node_mass_is_redistributed() {
        // The hub has no outgoing edges (dangling); with redistribution the
        // total probability mass stays 1.
        let (db, _) = hub_graph();
        compute_and_cache(&db).unwrap();
        let ranks = load_ranks(&db).unwrap();
        let total: f64 = ranks.values().sum();
        assert!(
            (total - 1.0).abs() < 1e-6,
            "ranks should sum to 1, got {}",
            total
        );
    }

    #[test]
    fn test_graph_without_edges_is_uniform() {
        let db = Database::open_in_memory().unwrap();
        db.upsert_file(
            &FileRecord {
                path: "src/lib.rs".to_string(),
                content_hash: "h".to_string(),
                size_bytes: 100,
                language: Some("rust".to_string()),
                last_indexed: 0,
            },
            None,
        )
        .unwrap();
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            insert_symbol(&db, "src/lib.rs", name, 10 + i as u32 * 10);
        }

        compute_and_cache(&db).unwrap();
        let ranks = load_ranks(&db).unwrap();
        assert_eq!(ranks.len(), 3);
        for rank in ranks.values() {
            assert!((rank - 1.0 / 3.0).abs() < 1e-9);
        }
    }

    #[test]
    fn test_ranks_are_deterministic_across_computes() {
        let (db, _) = hub_graph();

        compute_and_cache(&db).unwrap();
        let first = load_ranks(&db).unwrap();

        compute_and_cache(&db).unwrap();
        let second = load_ranks(&db).unwrap();

        assert_eq!(first.len(), second.len());
        for (id, rank) in &first {
            assert_eq!(
                rank.to_bits(),
                second[id].to_bits(),
                "rank for {} differs between computes",
                id
            );
        }
    }

    #[test]
    fn test_is_stale_and_cache_lifecycle() {
        let (db, _) = hub_graph();
        assert!(is_stale(&db).unwrap(), "empty cache should be stale");

        compute_and_cache(&db).unwrap();
        assert!(!is_stale(&db).unwrap(), "fresh cache should not be stale");

        db.clear_symbol_rank().unwrap();
        assert!(is_stale(&db).unwrap(), "cleared cache should be stale");
    }

    #[test]
    fn test_focus_boost_raises_neighbors_and_renormalizes() {
        let (db, hub) = hub_graph();
        compute_and_cache(&db).unwrap();
        let mut ranks = load_ranks(&db).unwrap();
        let unfocused = ranks.clone();

        // Focus on `a`: `a` and its 1-hop neighbor `hub` get boosted.
        let a_id = "src/lib.rs::a@10".to_string();
        let focus: HashSet<String> = [a_id.clone()].into_iter().collect();
        apply_focus(&db, &mut ranks, &focus).unwrap();

        let total: f64 = ranks.values().sum();
        assert!((total - 1.0).abs() < 1e-9, "boosted ranks should sum to 1");

        // `a` was tied with b/c/d before; after the boost it must outrank them.
        assert!(ranks[&a_id] > ranks["src/lib.rs::b@20"]);
        // The hub (a's neighbor) keeps its lead.
        assert!(ranks[&hub] > ranks["src/lib.rs::b@20"]);
        // Relative boost for `a` vs an unfocused peer is 10x.
        let ratio = (ranks[&a_id] / ranks["src/lib.rs::b@20"])
            / (unfocused[&a_id] / unfocused["src/lib.rs::b@20"]);
        assert!((ratio - FOCUS_BOOST).abs() < 1e-9);
    }
}
