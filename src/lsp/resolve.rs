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
//!
//! Two safeguards keep Stage B from writing a *wrong* target (worse than an
//! unresolved one, since it is never corrected):
//!
//! - The definition request is aimed at the callee identifier, not the start
//!   of the call expression tree-sitter records (for `obj.helper()` that is
//!   the receiver `obj`, whose definition is a different symbol entirely).
//! - The resolved symbol's name must match the edge's `target_name`; on a
//!   mismatch the edge is left unresolved.

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
    // (edge_id, 1-based line, 0-based col, target_name) per edge.
    type EdgeSite = (i64, u32, u32, String);
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
            .push((edge.edge_id, edge.line, edge.col, edge.target_name.clone()));
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
            let lines: Vec<&str> = text.lines().collect();

            // One didOpen per source file, then a definition request per edge.
            let Some(client) = mgr.client_for_stage_b(&language) else {
                break; // server unusable: skip the rest of this language
            };
            if client.did_open(&uri, &language, &text).is_err() {
                break;
            }

            for (edge_id, line, col, target_name) in edges {
                // Aim at the callee identifier: tree-sitter stores the start
                // of the whole call expression, which for method calls is the
                // receiver — asking definition there resolves the wrong symbol.
                let col = lines
                    .get(line.saturating_sub(1) as usize)
                    .map(|line_text| callee_column(line_text, &target_name, col))
                    .unwrap_or(col);

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

                if let Some(target_id) =
                    map_target(db, &root, &target_uri, target_line0, &target_name, verbose)
                {
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

/// Column of the callee identifier for a definition request: the first
/// occurrence of `target_name` at-or-after the stored column of the source
/// line. Falls back to the stored column when the name does not appear
/// (macro-generated code, stale text). Byte-index based, consistent with the
/// columns tree-sitter records.
fn callee_column(line_text: &str, target_name: &str, stored_col: u32) -> u32 {
    if target_name.is_empty() {
        return stored_col;
    }
    line_text
        .match_indices(target_name)
        .map(|(i, _)| i)
        .find(|&i| i >= stored_col as usize)
        .map(|i| i as u32)
        .unwrap_or(stored_col)
}

/// Whether a resolved symbol plausibly is the callee named by the edge:
/// exact name match, or a qualified tail match (`X.name` / `X::name`) for
/// methods whose symbol name carries the container.
fn name_matches(name: &str, qualified_name: Option<&str>, target_name: &str) -> bool {
    if target_name.is_empty() {
        return false;
    }
    let dot_tail = format!(".{target_name}");
    let path_tail = format!("::{target_name}");
    let candidate_matches = |candidate: &str| {
        candidate == target_name
            || candidate.ends_with(&dot_tail)
            || candidate.ends_with(&path_tail)
    };
    candidate_matches(name) || qualified_name.is_some_and(candidate_matches)
}

/// Map a definition target (`uri` + 0-based line) to an indexed symbol id.
///
/// The target must be a local file under the project root, already present in
/// the index, and the symbol found at the target line must be named like the
/// edge's `target_name`; anything else (stdlib, dependencies, generated
/// files, receiver/wrong-symbol answers) is skipped so the edge stays
/// unresolved instead of pointing at the wrong symbol.
fn map_target(
    db: &Database,
    root: &std::path::Path,
    target_uri: &str,
    target_line0: u32,
    target_name: &str,
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
    let id = match db.symbol_id_at_line(&rel, line) {
        Ok(Some(id)) => id,
        Ok(None) => return None,
        Err(e) => {
            if verbose {
                eprintln!("Warning: symbol lookup failed for {rel}:{line}: {e}");
            }
            return None;
        }
    };

    // Safety guard: never write a target whose name does not match the
    // recorded callee name. A wrong target_id is worse than NULL — it is
    // never corrected by later runs.
    let symbol = db.get_symbol(&id).ok()??;
    if name_matches(&symbol.name, symbol.qualified_name.as_deref(), target_name) {
        Some(id)
    } else {
        if verbose {
            eprintln!(
                "Warning: definition for `{target_name}` resolved to `{}` at {rel}:{line}; \
                 leaving edge unresolved",
                symbol.name
            );
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callee_column_targets_identifier_after_receiver() {
        // Stored col 11 = start of `obj`; `helper` begins at col 15.
        let line = "    return obj.helper()";
        assert_eq!(callee_column(line, "helper", 11), 15);
        // Bare call: the name sits exactly at the stored column.
        assert_eq!(callee_column("    return helper()", "helper", 11), 11);
        // Earlier occurrences (before the stored col) are ignored.
        assert_eq!(callee_column("helper = obj.helper()", "helper", 9), 13);
        // Name not found at-or-after the column: keep the stored column.
        assert_eq!(callee_column("    return mangled()", "helper", 11), 11);
        // Column past the end of the line: keep the stored column.
        assert_eq!(callee_column("x()", "helper", 40), 40);
        assert_eq!(callee_column("", "helper", 0), 0);
    }

    #[test]
    fn name_guard_accepts_exact_and_qualified_tail_matches() {
        assert!(name_matches("helper", None, "helper"));
        assert!(name_matches("Greeter.helper", None, "helper"));
        assert!(name_matches("helper", Some("Greeter.helper"), "helper"));
        assert!(name_matches("helper", Some("mod::helper"), "helper"));

        // Wrong symbol (e.g. the receiver's enclosing method): rejected.
        assert!(!name_matches("main", None, "helper"));
        assert!(!name_matches("main", Some("app.main"), "helper"));
        // Suffix-of-identifier is not a tail match.
        assert!(!name_matches("do_helper", None, "helper"));
        assert!(!name_matches("", None, "helper"));
        assert!(!name_matches("helper", None, ""));
    }
}
