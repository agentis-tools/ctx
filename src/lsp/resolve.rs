//! Stage B: resolve leftover cross-file references with the language server.
//!
//! After the cheap SQL passes in [`Database::resolve_edge_targets`], edges
//! whose target could not be determined statically still have
//! `target_id IS NULL`. For files handled by an `lsp` or `hybrid` backend we
//! ask the language server `textDocument/definition` at each recorded call
//! site and update the edge directly.
//!
//! Targets are written with [`Database::set_edge_target`] — a plain `UPDATE`
//! by edge rowid. They must never be routed through the `store_edges` insert
//! path, whose id rewriting would corrupt a cross-file target id into
//! `<current_file>::name`.

use std::collections::{BTreeMap, HashSet};

use crate::db::Database;

use super::{path_to_uri, uri_to_path, FileBackend, LspManager};

/// Resolve unresolved edges for the given files (index-relative paths) via
/// `textDocument/definition`. Returns the number of edges resolved. Never
/// fails: any server or IO problem simply leaves edges unresolved.
pub fn resolve_edges_with_lsp(
    db: &Database,
    mgr: &mut LspManager,
    changed_files: &HashSet<String>,
    verbose: bool,
) -> usize {
    if changed_files.is_empty() {
        return 0;
    }

    let unresolved = match db.unresolved_edges_with_location() {
        Ok(edges) => edges,
        Err(e) => {
            if verbose {
                eprintln!("Warning: could not list unresolved edges: {e}");
            }
            return 0;
        }
    };
    if unresolved.is_empty() {
        return 0;
    }

    // Group the candidate edges per (language, source file); only files
    // changed this run with an lsp/hybrid backend participate.
    // (edge_id, 1-based line, 0-based col) per edge.
    type EdgeSite = (i64, u32, u32);
    let mut per_language: BTreeMap<String, BTreeMap<String, Vec<EdgeSite>>> = BTreeMap::new();
    for edge in &unresolved {
        if !changed_files.contains(&edge.source_file) {
            continue;
        }
        let language = match mgr.backend_for(std::path::Path::new(&edge.source_file)) {
            FileBackend::Lsp(lang) | FileBackend::Hybrid(lang) => lang,
            _ => continue,
        };
        per_language
            .entry(language)
            .or_default()
            .entry(edge.source_file.clone())
            .or_default()
            .push((edge.edge_id, edge.line, edge.col));
    }

    let root = mgr.root().to_path_buf();
    let mut resolved = 0usize;

    for (language, files) in per_language {
        'files: for (rel_path, edges) in files {
            let abs_path = root.join(&rel_path);
            let Ok(text) = std::fs::read_to_string(&abs_path) else {
                continue;
            };
            let uri = path_to_uri(&abs_path);

            // One didOpen per source file, then a definition request per edge.
            let Some(client) = mgr.client_for_stage_b(&language) else {
                break; // server unusable: skip the rest of this language
            };
            if client.did_open(&uri, &language, &text).is_err() {
                break;
            }

            for (edge_id, line, col) in edges {
                // ctx lines are 1-based; LSP positions are 0-based.
                let target = match client.definition(&uri, line.saturating_sub(1), col) {
                    Ok(target) => target,
                    Err(e) => {
                        if verbose {
                            eprintln!(
                                "Warning: definition lookup failed in {rel_path}:{line}: {e}"
                            );
                        }
                        if client.failure().is_some() {
                            // Server died: stop Stage B for this language.
                            break 'files;
                        }
                        continue;
                    }
                };

                let Some((target_uri, target_line0)) = target else {
                    continue;
                };

                if let Some(target_id) = map_target(db, &root, &target_uri, target_line0, verbose) {
                    match db.set_edge_target(edge_id, &target_id) {
                        Ok(true) => resolved += 1,
                        Ok(false) => {}
                        Err(e) => {
                            if verbose {
                                eprintln!("Warning: failed to update edge {edge_id}: {e}");
                            }
                        }
                    }
                }
            }

            client.did_close(&uri);
        }
    }

    resolved
}

/// Map a definition target (`uri` + 0-based line) to an indexed symbol id.
///
/// The target must be a local file under the project root and already present
/// in the index; anything else (stdlib, dependencies, generated files) is
/// skipped.
fn map_target(
    db: &Database,
    root: &std::path::Path,
    target_uri: &str,
    target_line0: u32,
    verbose: bool,
) -> Option<String> {
    let target_path = uri_to_path(target_uri)?;
    // Canonicalize to survive symlinked roots (e.g. /tmp on macOS).
    let target_path = target_path.canonicalize().ok()?;
    let rel = target_path
        .strip_prefix(root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");

    // Only resolve against files that are actually indexed.
    match db.get_file_hash(&rel) {
        Ok(Some(_)) => {}
        _ => return None,
    }

    let line = target_line0 + 1;
    match db.symbol_id_at_line(&rel, line) {
        Ok(Some(id)) => Some(id),
        Ok(None) => None,
        Err(e) => {
            if verbose {
                eprintln!("Warning: symbol lookup failed for {rel}:{line}: {e}");
            }
            None
        }
    }
}
