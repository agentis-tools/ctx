//! `ctx map`: token-budgeted repository map.
//!
//! Emits the most important symbols of the codebase (ranked by PageRank
//! over the resolved symbol graph, see `ctx::rank`) until a token budget is
//! exhausted. Roughly the first 10% of the budget is spent on a compact
//! project tree; the rest lists symbols grouped by file.
//!
//! Tokens are estimated as `ceil(chars / 4)`. The output is deterministic
//! for identical index state, which makes the command suitable for
//! SessionStart hooks that prime an AI assistant with a stable overview.

use std::collections::{BTreeMap, HashSet};
use std::env;
use std::path::PathBuf;

use globset::Glob;

use ctx::db::Database;
use ctx::error::Result;
use ctx::index::open_database;
use ctx::json;
use ctx::rank;
use ctx::tree::generate_tree;
use ctx::walker::FileEntry;

/// Fraction of the character budget reserved for the project tree (1/10).
const TREE_BUDGET_DIVISOR: usize = 10;

/// Output format for `ctx map`.
///
/// This is a plain enum (not a clap `ValueEnum`): on the CLI the format is
/// parsed as `crate::cli::OutputFormat` (which subcommands share with the
/// global `--format` flag) and converted in `main.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapFormat {
    Text,
    Markdown,
    Json,
}

/// A symbol entry selected for the map.
#[derive(Debug, Clone)]
pub struct MapEntry {
    pub file: String,
    pub line: u32,
    pub kind: String,
    pub signature: String,
    pub rank: f64,
}

/// The generated map.
#[derive(Debug)]
pub struct MapResult {
    /// Rendered output (text or markdown; the text rendering is used for
    /// budget accounting when the output format is JSON).
    pub rendered: String,
    /// Estimated token count of the rendered map: `ceil(chars / 4)`.
    pub token_estimate: usize,
    /// The (possibly truncated) project tree, unwrapped.
    pub tree: String,
    /// Selected entries in emit order.
    pub entries: Vec<MapEntry>,
    /// The focus argument, if any was given.
    pub focus: Option<String>,
}

/// Run the map command.
pub fn run_map(budget: usize, focus: Option<&str>, format: MapFormat) -> Result<()> {
    let root = env::current_dir()?;
    let db = open_database(&root)?;
    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    let result = build_map(&db, &root_name, budget, focus, format)?;

    match format {
        MapFormat::Json => {
            let entries: Vec<serde_json::Value> = result
                .entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "file": e.file,
                        "line": e.line,
                        "kind": e.kind,
                        "signature": e.signature,
                        "rank": e.rank,
                    })
                })
                .collect();
            json::emit(
                "map",
                serde_json::json!({
                    "budget": budget,
                    "token_estimate": result.token_estimate,
                    "focus": result.focus,
                    "tree": result.tree,
                    "entries": entries,
                }),
            )
        }
        _ => {
            print!("{}", result.rendered);
            Ok(())
        }
    }
}

/// Build the map for the given budget/focus/format.
///
/// For `MapFormat::Json` the selection and token estimate are computed from
/// the text rendering, so all formats select the same entries as text.
pub fn build_map(
    db: &Database,
    root_name: &str,
    budget: usize,
    focus: Option<&str>,
    format: MapFormat,
) -> Result<MapResult> {
    // JSON accounting uses the text rendering.
    let render_format = match format {
        MapFormat::Markdown => MapFormat::Markdown,
        _ => MapFormat::Text,
    };

    let char_budget = budget.saturating_mul(4);

    // Recompute the rank cache lazily if the index changed since last time.
    if rank::is_stale(db)? {
        rank::compute_and_cache(db)?;
    }
    let mut ranks = rank::load_ranks(db)?;

    // Apply the focus boost (in memory only).
    if let Some(focus_arg) = focus {
        let focus_ids = resolve_focus(db, focus_arg)?;
        if focus_ids.is_empty() {
            eprintln!(
                "Warning: --focus '{}' matched no indexed file or symbol; proceeding without focus",
                focus_arg
            );
        } else {
            rank::apply_focus(db, &mut ranks, &focus_ids)?;
        }
    }

    // Project tree: roughly the first 10% of the budget.
    let tree_reserve = char_budget / TREE_BUDGET_DIVISOR;
    let (tree, tree_section) = build_tree_section(db, root_name, tree_reserve, render_format)?;

    // Rank-ordered symbol entries (F5: stable sort for determinism).
    let mut rows = db.get_map_symbols()?;
    rows.sort_by(|a, b| {
        let ra = ranks.get(&a.id).copied().unwrap_or(0.0);
        let rb = ranks.get(&b.id).copied().unwrap_or(0.0);
        rb.total_cmp(&ra)
            .then_with(|| a.qualified_name.cmp(&b.qualified_name))
            .then_with(|| a.id.cmp(&b.id))
    });

    // Greedy emission: stop before the first entry that would exceed the
    // budget. Consecutive same-file symbols share one file header.
    let mut body = String::new();
    let mut entries: Vec<MapEntry> = Vec::new();
    let mut used = char_len(&tree_section);
    let mut current_file: Option<String> = None;

    for row in rows {
        let entry = MapEntry {
            file: row.file_path.clone(),
            line: row.line_start,
            kind: row.kind.clone(),
            signature: normalize_signature(row.signature.as_deref().unwrap_or(&row.name)),
            rank: ranks.get(&row.id).copied().unwrap_or(0.0),
        };

        let header = if current_file.as_deref() != Some(entry.file.as_str()) {
            render_file_header(&entry.file, render_format)
        } else {
            String::new()
        };
        let line = render_entry_line(&entry, render_format);

        let cost = char_len(&header) + char_len(&line);
        if used + cost > char_budget {
            break;
        }
        used += cost;
        body.push_str(&header);
        body.push_str(&line);
        current_file = Some(entry.file.clone());
        entries.push(entry);
    }

    let rendered = format!("{}{}", tree_section, body);
    let token_estimate = char_len(&rendered).div_ceil(4);

    Ok(MapResult {
        rendered,
        token_estimate,
        tree,
        entries,
        focus: focus.map(|s| s.to_string()),
    })
}

/// Resolve the `--focus` argument to a set of symbol IDs.
///
/// Tries, in order: an exact indexed file path, a path glob over indexed
/// files, then an exact symbol name / qualified name. Returns an empty set
/// when nothing matches.
fn resolve_focus(db: &Database, focus: &str) -> Result<HashSet<String>> {
    let files = db.get_indexed_files()?;

    let mut matched: Vec<&String> = files.iter().filter(|f| f.as_str() == focus).collect();
    if matched.is_empty() {
        if let Ok(glob) = Glob::new(focus) {
            let matcher = glob.compile_matcher();
            matched = files
                .iter()
                .filter(|f| matcher.is_match(f.as_str()))
                .collect();
        }
    }

    let mut ids = HashSet::new();
    if matched.is_empty() {
        ids.extend(db.get_symbol_ids_by_name(focus)?);
    } else {
        for file in matched {
            ids.extend(db.get_symbol_ids_in_file(file)?);
        }
    }
    Ok(ids)
}

/// Build the compact project tree section.
///
/// Paths deeper than two directory levels are collapsed into a synthetic
/// `dir1/dir2/… (N files)` entry. The rendered section (including any
/// format wrapping) never exceeds `reserve` characters; when the tree is
/// too large, whole lines are dropped and replaced by `… (+M more)`.
///
/// Returns `(raw_tree, wrapped_section)`.
fn build_tree_section(
    db: &Database,
    root_name: &str,
    reserve: usize,
    format: MapFormat,
) -> Result<(String, String)> {
    let files = db.get_files_with_sizes()?;
    if files.is_empty() || reserve == 0 {
        return Ok((String::new(), String::new()));
    }

    let entries = collapse_tree_entries(&files);
    let tree = generate_tree(root_name, &entries, false);

    // Wrapping overhead is constant per format.
    let overhead = char_len(&render_tree_wrapper("", format));
    let inner_budget = reserve.saturating_sub(overhead);

    let truncated = truncate_lines(&tree, inner_budget);
    if truncated.is_empty() {
        return Ok((String::new(), String::new()));
    }
    let section = render_tree_wrapper(&truncated, format);
    Ok((truncated, section))
}

/// Collapse indexed file paths for the compact tree: files at most two
/// levels deep are kept; deeper files are aggregated per level-2 directory.
fn collapse_tree_entries(files: &[(String, i64)]) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = Vec::new();
    let mut collapsed: BTreeMap<(String, String), (usize, u64)> = BTreeMap::new();

    for (path, size) in files {
        let components: Vec<&str> = path.split('/').collect();
        if components.len() <= 2 {
            entries.push(FileEntry {
                absolute_path: PathBuf::from(path),
                relative_path: PathBuf::from(path),
                size: (*size).max(0) as u64,
            });
        } else {
            let key = (components[0].to_string(), components[1].to_string());
            let slot = collapsed.entry(key).or_insert((0, 0));
            slot.0 += 1;
            slot.1 += (*size).max(0) as u64;
        }
    }

    for ((dir1, dir2), (count, size)) in collapsed {
        let noun = if count == 1 { "file" } else { "files" };
        entries.push(FileEntry {
            absolute_path: PathBuf::from(format!("{}/{}", dir1, dir2)),
            relative_path: PathBuf::from(format!("{}/{}/… ({} {})", dir1, dir2, count, noun)),
            size,
        });
    }

    entries
}

/// Truncate `text` to at most `max_chars` characters by dropping whole
/// trailing lines and appending `… (+M more)`.
fn truncate_lines(text: &str, max_chars: usize) -> String {
    if char_len(text) <= max_chars {
        return text.to_string();
    }

    let lines: Vec<&str> = text.lines().collect();
    // Prefix sums of line lengths (each line costs its chars + newline).
    let mut prefix = Vec::with_capacity(lines.len() + 1);
    prefix.push(0usize);
    for line in &lines {
        prefix.push(prefix.last().unwrap() + char_len(line) + 1);
    }

    for keep in (0..lines.len()).rev() {
        let omitted = lines.len() - keep;
        let suffix = format!("… (+{} more)\n", omitted);
        if prefix[keep] + char_len(&suffix) <= max_chars {
            let mut out = String::new();
            for line in &lines[..keep] {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(&suffix);
            return out;
        }
    }

    String::new()
}

/// Wrap the raw tree for the output format (adds the trailing blank line
/// that separates the tree from the entries).
fn render_tree_wrapper(tree: &str, format: MapFormat) -> String {
    match format {
        MapFormat::Markdown => format!("```text\n{}```\n\n", tree),
        _ => format!("{}\n", tree),
    }
}

/// Render a file header line.
fn render_file_header(file: &str, format: MapFormat) -> String {
    match format {
        MapFormat::Markdown => format!("## {}\n", file),
        _ => format!("{}\n", file),
    }
}

/// Render a single symbol line.
fn render_entry_line(entry: &MapEntry, format: MapFormat) -> String {
    match format {
        MapFormat::Markdown => format!(
            "- L{}: `{}` [{}]\n",
            entry.line, entry.signature, entry.kind
        ),
        _ => format!("  L{}: {} [{}]\n", entry.line, entry.signature, entry.kind),
    }
}

/// Collapse a (possibly multi-line) signature to a single line.
fn normalize_signature(signature: &str) -> String {
    signature.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Character count (token estimation is `ceil(chars / 4)`).
fn char_len(s: &str) -> usize {
    s.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx::index::Indexer;
    use std::fs;
    use tempfile::TempDir;

    const CALLERS: usize = 20;

    /// Fixture repo: `main` calls 20 `caller_XX` functions, each of which
    /// calls `central_hub`; `zz_lonely_leaf_util` has no callers/callees.
    fn fixture() -> (TempDir, Indexer) {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let mut main_body = String::from("fn main() {\n");
        for i in 0..CALLERS {
            main_body.push_str(&format!("    caller_{:02}();\n", i));
        }
        main_body.push_str("}\n");
        fs::write(root.join("main.rs"), main_body).unwrap();

        fs::write(
            root.join("hub.rs"),
            "pub fn central_hub() -> i32 {\n    42\n}\n",
        )
        .unwrap();

        for i in 0..CALLERS {
            fs::write(
                root.join(format!("caller_{:02}.rs", i)),
                format!(
                    "pub fn caller_{:02}() {{\n    let _ = central_hub();\n}}\n",
                    i
                ),
            )
            .unwrap();
        }

        fs::write(
            root.join("leaf.rs"),
            "pub fn zz_lonely_leaf_util() -> i32 {\n    7\n}\n",
        )
        .unwrap();

        let mut indexer = Indexer::new_in_memory(root).unwrap();
        let result = indexer.index().unwrap();
        assert!(result.files_indexed >= CALLERS + 3);
        (temp, indexer)
    }

    #[test]
    fn test_token_estimate_never_exceeds_budget() {
        let (_temp, indexer) = fixture();
        let db = indexer.database();

        for budget in [50, 100, 150, 300, 800, 2000, 10_000] {
            for format in [MapFormat::Text, MapFormat::Markdown, MapFormat::Json] {
                let map = build_map(db, "project", budget, None, format).unwrap();
                let max = budget + budget / 20; // budget + 5%
                assert!(
                    map.token_estimate <= max,
                    "format {:?} budget {}: estimate {} exceeds {}",
                    format,
                    budget,
                    map.token_estimate,
                    max
                );
                // Sanity: the estimate matches the rendered output.
                assert_eq!(map.token_estimate, map.rendered.chars().count().div_ceil(4));
            }
        }
    }

    #[test]
    fn test_most_called_symbol_appears_at_default_budget() {
        let (_temp, indexer) = fixture();
        let map = build_map(indexer.database(), "project", 2000, None, MapFormat::Text).unwrap();
        assert!(
            map.rendered.contains("central_hub"),
            "hub should appear at the default budget:\n{}",
            map.rendered
        );
        // The hub is the highest-ranked entry.
        assert!(map.entries[0].signature.contains("central_hub"));
    }

    #[test]
    fn test_leaf_utility_absent_at_tight_budget() {
        let (_temp, indexer) = fixture();
        let map = build_map(indexer.database(), "project", 150, None, MapFormat::Text).unwrap();
        assert!(
            map.rendered.contains("central_hub"),
            "hub should still appear at a tight budget:\n{}",
            map.rendered
        );
        assert!(
            !map.rendered.contains("zz_lonely_leaf_util"),
            "uncalled leaf should not appear at a tight budget:\n{}",
            map.rendered
        );
    }

    #[test]
    fn test_focus_glob_surfaces_absent_symbols() {
        let (_temp, indexer) = fixture();
        let db = indexer.database();

        let without = build_map(db, "project", 150, None, MapFormat::Text).unwrap();
        assert!(!without.rendered.contains("zz_lonely_leaf_util"));

        let with = build_map(db, "project", 150, Some("leaf*"), MapFormat::Text).unwrap();
        assert!(
            with.rendered.contains("zz_lonely_leaf_util"),
            "--focus glob should surface the leaf at the same budget:\n{}",
            with.rendered
        );
        assert_eq!(with.focus.as_deref(), Some("leaf*"));
    }

    #[test]
    fn test_focus_without_match_falls_back_to_unfocused() {
        let (_temp, indexer) = fixture();
        let db = indexer.database();

        let unfocused = build_map(db, "project", 300, None, MapFormat::Text).unwrap();
        let no_match = build_map(
            db,
            "project",
            300,
            Some("no_such_thing_xyz"),
            MapFormat::Text,
        )
        .unwrap();
        assert_eq!(unfocused.rendered, no_match.rendered);
    }

    #[test]
    fn test_focus_symbol_name_lookup() {
        let (_temp, indexer) = fixture();
        let db = indexer.database();

        let with = build_map(
            db,
            "project",
            150,
            Some("zz_lonely_leaf_util"),
            MapFormat::Text,
        )
        .unwrap();
        assert!(with.rendered.contains("zz_lonely_leaf_util"));
    }

    #[test]
    fn test_identical_invocations_are_byte_identical() {
        let (_temp, indexer) = fixture();
        let db = indexer.database();

        for format in [MapFormat::Text, MapFormat::Markdown] {
            let first = build_map(db, "project", 500, None, format).unwrap();
            let second = build_map(db, "project", 500, None, format).unwrap();
            assert_eq!(
                first.rendered, second.rendered,
                "repeated {:?} output must be byte-identical",
                format
            );
        }

        // Also byte-identical after a forced recompute of the rank cache.
        db.clear_symbol_rank().unwrap();
        let first = build_map(db, "project", 500, None, MapFormat::Text).unwrap();
        db.clear_symbol_rank().unwrap();
        let second = build_map(db, "project", 500, None, MapFormat::Text).unwrap();
        assert_eq!(first.rendered, second.rendered);
    }

    #[test]
    fn test_map_populates_cache_and_reindex_clears_it() {
        let (temp, mut indexer) = fixture();
        let db_symbols = indexer.database().count_symbols().unwrap();
        assert!(db_symbols > 0);

        // Indexing leaves the cache empty; the first map populates it.
        assert_eq!(indexer.database().count_symbol_ranks().unwrap(), 0);
        build_map(indexer.database(), "project", 500, None, MapFormat::Text).unwrap();
        assert_eq!(indexer.database().count_symbol_ranks().unwrap(), db_symbols);

        // Reindexing a changed file clears the cache.
        fs::write(
            temp.path().join("caller_00.rs"),
            "pub fn caller_00() {\n    let _ = central_hub() + 1;\n}\n",
        )
        .unwrap();
        let result = indexer.index().unwrap();
        assert_eq!(result.files_indexed, 1);
        assert_eq!(indexer.database().count_symbol_ranks().unwrap(), 0);

        // The next map repopulates it.
        build_map(indexer.database(), "project", 500, None, MapFormat::Text).unwrap();
        assert_eq!(
            indexer.database().count_symbol_ranks().unwrap(),
            indexer.database().count_symbols().unwrap()
        );
    }

    #[test]
    fn test_tree_collapses_deep_paths() {
        let files = vec![
            ("Cargo.toml".to_string(), 10),
            ("src/main.rs".to_string(), 20),
            ("src/db/mod.rs".to_string(), 30),
            ("src/db/schema.rs".to_string(), 40),
            ("src/db/deep/nested.rs".to_string(), 50),
        ];
        let entries = collapse_tree_entries(&files);
        let paths: Vec<String> = entries
            .iter()
            .map(|e| e.relative_path.to_string_lossy().to_string())
            .collect();
        assert!(paths.contains(&"Cargo.toml".to_string()));
        assert!(paths.contains(&"src/main.rs".to_string()));
        assert!(paths.contains(&"src/db/… (3 files)".to_string()));
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_truncate_lines_appends_more_marker() {
        let text = "line one\nline two\nline three\n";
        let truncated = truncate_lines(text, 22);
        assert!(truncated.chars().count() <= 22, "got {:?}", truncated);
        assert!(truncated.contains("more)"), "got {:?}", truncated);

        // Fits untouched when under the limit.
        assert_eq!(truncate_lines(text, 100), text);

        // Nothing fits.
        assert_eq!(truncate_lines(text, 3), "");
    }
}
