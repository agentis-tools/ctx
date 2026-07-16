//! Smart context selection for LLM workflows.
//!
//! This module provides intelligent file selection based on:
//! - Semantic search using embeddings
//! - Call graph expansion (callers and callees)
//! - Token budget management
//!
//! # Usage
//!
//! ```ignore
//! let config = SmartConfig::default();
//! let result = smart_context(&db, &analytics, &provider, "add caching", config)?;
//! for file in result.selected_files {
//!     println!("{}: {} tokens", file.path, file.token_count);
//! }
//! ```

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::analytics::{Analytics, CallGraphNode, ImpactNode};
use crate::db::Database;
use crate::embeddings::{semantic_search, Embedding, EmbeddingProvider, SearchResult};
use crate::error::{CtxError, Result};
use crate::tokens::{count_file_tokens, select_by_token_budget, Encoding, HasTokenCount};
use crate::utils::lexical_tokens;
use crate::walker::FilePatternFilter;

/// Configuration for smart context selection.
#[derive(Debug, Clone)]
pub struct SmartConfig {
    /// Maximum tokens in output
    pub max_tokens: usize,
    /// Call graph expansion depth
    pub depth: i32,
    /// Number of initial semantic matches to find
    pub top: usize,
    /// Tokenizer encoding to use
    pub encoding: Encoding,
}

impl Default for SmartConfig {
    fn default() -> Self {
        Self {
            max_tokens: 8000,
            depth: 2,
            top: 10,
            encoding: Encoding::default(),
        }
    }
}

/// Reason why a file was selected.
#[derive(Debug, Clone)]
pub enum SelectionReason {
    /// Direct semantic match to task embedding
    SemanticMatch {
        /// The symbol that matched
        symbol: String,
        /// Similarity score (0.0-1.0)
        score: f32,
    },
    /// Called by a matched symbol
    CalledBy {
        /// The symbol that calls this file's code
        caller: String,
        /// Depth in call graph
        depth: i32,
    },
    /// Calls a matched symbol
    Calls {
        /// The symbol that this file calls
        callee: String,
        /// Depth in call graph
        depth: i32,
    },
    /// In the same module as a matched symbol
    #[allow(dead_code)] // Used in tests and pattern matching
    SameModule {
        /// The related symbol
        symbol: String,
    },
    /// User explicitly requested this file
    #[allow(dead_code)] // Used in tests and pattern matching
    Explicit,
}

impl SelectionReason {
    /// Get a human-readable description of the reason.
    pub fn description(&self) -> String {
        match self {
            SelectionReason::SemanticMatch { symbol, score } => {
                format!("SemanticMatch: \"{}\" (score: {:.2})", symbol, score)
            }
            SelectionReason::CalledBy { caller, depth } => {
                format!("CalledBy: {} (depth {})", caller, depth)
            }
            SelectionReason::Calls { callee, depth } => {
                format!("Calls: {} (depth {})", callee, depth)
            }
            SelectionReason::SameModule { symbol } => {
                format!("SameModule: shares context with {}", symbol)
            }
            SelectionReason::Explicit => "Explicit: user requested".to_string(),
        }
    }

    /// Get the relevance weight for this reason type.
    fn weight(&self) -> f32 {
        match self {
            SelectionReason::SemanticMatch { score, .. } => *score,
            SelectionReason::CalledBy { depth, .. } => 0.8 / (*depth as f32).max(1.0),
            SelectionReason::Calls { depth, .. } => 0.7 / (*depth as f32).max(1.0),
            SelectionReason::SameModule { .. } => 0.5,
            SelectionReason::Explicit => 1.0,
        }
    }
}

/// A selected file with relevance information.
#[derive(Debug, Clone)]
pub struct FileSelection {
    /// File path (relative to project root)
    pub path: String,
    /// Relevance score (0.0-1.0)
    pub relevance_score: f32,
    /// Why this file was selected
    pub reasons: Vec<SelectionReason>,
    /// Token count for this file
    pub token_count: usize,
    /// Number of distinct task tokens that appear in this file's path or in the
    /// names of the symbols that selected it (see [`FileSelection::compute_lexical_score`]).
    /// A high-precision relevance signal layered on top of the semantic/graph tiers.
    pub lexical_score: f32,
}

impl HasTokenCount for FileSelection {
    fn token_count(&self) -> usize {
        self.token_count
    }
}

impl FileSelection {
    /// Create a new file selection.
    fn new(path: String, reason: SelectionReason) -> Self {
        let score = reason.weight();
        Self {
            path,
            relevance_score: score,
            reasons: vec![reason],
            token_count: 0,
            lexical_score: 0.0,
        }
    }

    /// Add another reason for selection, updating the relevance score.
    fn add_reason(&mut self, reason: SelectionReason) {
        let new_weight = reason.weight();
        // Take the maximum weight as the relevance score
        if new_weight > self.relevance_score {
            self.relevance_score = new_weight;
        }
        self.reasons.push(reason);
    }

    /// The best (maximum) direct semantic-match score among this file's reasons,
    /// or `None` if the file was pulled in only through call-graph expansion.
    ///
    /// Files that directly match the task embedding are ranked ahead of graph-only
    /// files (see [`rank_files`]), so this must be read independently of
    /// `relevance_score`, which `add_reason` collapses to the max weight across
    /// reasons (and which the flat 0.7/0.8 call-graph constants would otherwise
    /// dominate over a compressed semantic score).
    fn best_semantic_score(&self) -> Option<f32> {
        self.reasons
            .iter()
            .filter_map(|r| match r {
                SelectionReason::SemanticMatch { score, .. } => Some(*score),
                _ => None,
            })
            .reduce(f32::max)
    }

    /// Count the distinct `task_tokens` that appear in this file's **path**.
    ///
    /// This is the lexical relevance signal: when a task word ("openai",
    /// "solidity", "sql") is literally present in a candidate's path, that is
    /// strong, high-precision evidence the file is on-topic even if the embedding
    /// ranked it low. Returns 0.0 when there is no overlap.
    ///
    /// Path only — deliberately *not* symbol names. Matching symbol names is
    /// low-precision: ubiquitous identifiers (notably `ctx`, the tool's own name,
    /// which appears as a test helper across `tests/*_cli.rs`) would score a hit
    /// on nearly every file and drown out the genuinely on-topic one.
    fn compute_lexical_score(&self, task_tokens: &BTreeSet<String>) -> f32 {
        if task_tokens.is_empty() {
            return 0.0;
        }
        let path_tokens = lexical_tokens(&self.path);
        task_tokens.intersection(&path_tokens).count() as f32
    }
}

/// Result of smart context selection.
#[derive(Debug, Clone)]
pub struct SmartContext {
    /// The task description used for selection
    #[allow(dead_code)] // Part of public API
    pub task: String,
    /// Selected files with their relevance information
    pub selected_files: Vec<FileSelection>,
    /// Total token count of selected files
    pub total_tokens: usize,
    /// Whether selection was truncated by token limit
    pub truncated: bool,
    /// Number of files omitted due to token limit
    pub omitted_count: usize,
}

/// Select files relevant to a task using semantic search and call graph analysis.
///
/// # Algorithm
///
/// 1. Embed the task description
/// 2. Find top-N symbols matching the embedding
/// 3. For each matched symbol:
///    - Add the file containing the symbol
///    - Expand call graph (callers + callees) to specified depth
///    - Add files from expanded symbols
/// 4. Deduplicate files, keeping highest relevance
/// 5. Count tokens for each file
/// 6. Sort by relevance score descending
/// 7. Include files until token limit is reached
pub fn smart_context(
    db: &Database,
    analytics: &Analytics,
    provider: &dyn EmbeddingProvider,
    task: &str,
    config: SmartConfig,
) -> Result<SmartContext> {
    let root = std::env::current_dir().unwrap_or_default();
    let filter = FilePatternFilter::all(&root);
    smart_context_filtered(db, analytics, provider, task, config, &filter)
}

/// Select files relevant to a task, restricted to positional file patterns.
///
/// Filtering happens before semantic seeds are limited and is also applied to
/// every call-graph neighbor, so expansion cannot escape the requested scope.
pub fn smart_context_filtered(
    db: &Database,
    analytics: &Analytics,
    provider: &dyn EmbeddingProvider,
    task: &str,
    config: SmartConfig,
    filter: &FilePatternFilter,
) -> Result<SmartContext> {
    // 1. Embed the task description
    let task_embedding = provider.embed(task)?;

    smart_context_with_embedding_filtered(db, analytics, task, &task_embedding, config, filter)
}

/// Select files relevant to a task using a pre-computed embedding.
///
/// This variant is useful when the embedding has been computed asynchronously
/// (e.g., in the MCP server with OpenAI's async API) to avoid blocking the async runtime.
///
/// See [`smart_context`] for the full algorithm description.
pub fn smart_context_with_embedding(
    db: &Database,
    analytics: &Analytics,
    task: &str,
    task_embedding: &Embedding,
    config: SmartConfig,
) -> Result<SmartContext> {
    let root = std::env::current_dir().unwrap_or_default();
    let filter = FilePatternFilter::all(&root);
    smart_context_with_embedding_filtered(db, analytics, task, task_embedding, config, &filter)
}

/// Pre-computed-embedding variant of [`smart_context_filtered`].
pub fn smart_context_with_embedding_filtered(
    db: &Database,
    analytics: &Analytics,
    task: &str,
    task_embedding: &Embedding,
    config: SmartConfig,
    filter: &FilePatternFilter,
) -> Result<SmartContext> {
    // 2. Semantic search for matching symbols
    let matches = semantic_matches_in_scope(db, task_embedding, config.top, filter)?;

    if matches.is_empty() {
        return Err(CtxError::NoMatches);
    }

    // 3. Build file selection map
    let mut files: HashMap<String, FileSelection> = HashMap::new();

    for result in &matches {
        // Add the file containing the matched symbol
        add_file(
            &mut files,
            &result.file_path,
            SelectionReason::SemanticMatch {
                symbol: result.name.clone(),
                score: result.score,
            },
        );

        // Expand call graph
        expand_symbol(&mut files, analytics, result, config.depth, filter)?;
    }

    // 4. Convert to vector and count tokens
    let mut selections: Vec<FileSelection> = files.into_values().collect();

    // Count tokens for each file
    for selection in &mut selections {
        selection.token_count = count_file_token_safe(&selection.path, config.encoding);
    }

    // 4b. Lexical relevance: reward candidates whose path or matched symbol names
    // contain the task's own tokens. This rescues on-topic files the embedding
    // ranked low (e.g. `embeddings/openai.rs` for "…openai") without any tunable
    // float weight — the score is a plain distinct-token-hit count.
    let task_tokens = lexical_tokens(task);
    for selection in &mut selections {
        selection.lexical_score = selection.compute_lexical_score(&task_tokens);
    }

    // 5. Rank files by relevance
    rank_files(&mut selections);

    // 6. Apply token limit, but never silently drop the single most-relevant file.
    let (selected, total_tokens, omitted) =
        select_with_guaranteed_top(selections, config.max_tokens);

    Ok(SmartContext {
        task: task.to_string(),
        selected_files: selected,
        total_tokens,
        truncated: omitted > 0,
        omitted_count: omitted,
    })
}

/// Fetch semantic matches in bounded batches until filtering yields `limit`
/// results or the index is exhausted.
fn semantic_matches_in_scope(
    db: &Database,
    task_embedding: &Embedding,
    limit: usize,
    filter: &FilePatternFilter,
) -> Result<Vec<SearchResult>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let mut fetch = limit.saturating_mul(3).max(limit);
    loop {
        let raw = semantic_search(db, task_embedding, fetch)?;
        let exhausted = raw.len() < fetch;
        let matches: Vec<_> = raw
            .into_iter()
            .filter(|result| filter.matches(&result.file_path))
            .take(limit)
            .collect();

        if matches.len() == limit || exhausted {
            return Ok(matches);
        }

        let next = fetch.saturating_mul(2);
        if next == fetch {
            return Ok(matches);
        }
        fetch = next;
    }
}

/// Apply the token budget while guaranteeing the top-ranked file is always
/// included — even when that file alone exceeds the budget.
///
/// The greedy first-fit selector would otherwise skip an oversized rank-1 file
/// (e.g. a 9k-token parser larger than the whole 8k budget) and backfill with
/// smaller, less-relevant files, so the command returns everything *except* the
/// file the task is really about. Here the top file is force-included, then the
/// remainder is first-fit against the leftover budget via the shared
/// [`select_by_token_budget`].
fn select_with_guaranteed_top(
    ranked: Vec<FileSelection>,
    max_tokens: usize,
) -> (Vec<FileSelection>, usize, usize) {
    let mut iter = ranked.into_iter();
    let Some(top) = iter.next() else {
        return (Vec::new(), 0, 0);
    };
    let top_tokens = top.token_count;
    let rest: Vec<FileSelection> = iter.collect();
    // Budget the remainder against whatever is left after the guaranteed top file
    // (saturating: an oversized top file leaves a zero budget for the rest).
    let remaining_budget = max_tokens.saturating_sub(top_tokens);
    let (mut selected_rest, rest_tokens, omitted) = select_by_token_budget(rest, remaining_budget);

    let mut selected = Vec::with_capacity(selected_rest.len() + 1);
    selected.push(top);
    selected.append(&mut selected_rest);
    (selected, top_tokens + rest_tokens, omitted)
}

/// Add a file to the selection map, merging reasons if already present.
fn add_file(files: &mut HashMap<String, FileSelection>, path: &str, reason: SelectionReason) {
    if let Some(existing) = files.get_mut(path) {
        existing.add_reason(reason);
    } else {
        files.insert(
            path.to_string(),
            FileSelection::new(path.to_string(), reason),
        );
    }
}

/// Expand a symbol's call graph and add related files.
fn expand_symbol(
    files: &mut HashMap<String, FileSelection>,
    analytics: &Analytics,
    result: &SearchResult,
    depth: i32,
    filter: &FilePatternFilter,
) -> Result<()> {
    // Expand callers (who calls this symbol - impact analysis)
    if let Ok(callers) = analytics.impact_analysis(&result.symbol_id, depth) {
        for caller in callers {
            if filter.matches(&caller.file_path) {
                add_file_from_impact(files, &caller, &result.name);
            }
        }
    }

    // Expand callees (what does this symbol call - call graph)
    if let Ok(callees) = analytics.call_graph(&result.symbol_id, depth) {
        for callee in callees {
            if filter.matches(&callee.file_path) {
                add_file_from_call_graph(files, &callee, &result.name);
            }
        }
    }

    Ok(())
}

/// Add a file from impact analysis result.
fn add_file_from_impact(
    files: &mut HashMap<String, FileSelection>,
    node: &ImpactNode,
    callee_name: &str,
) {
    add_file(
        files,
        &node.file_path,
        SelectionReason::CalledBy {
            caller: node.name.clone(),
            depth: node.distance,
        },
    );

    // If in the same file as the callee, also add SameModule reason
    if let Some(existing) = files.get(&node.file_path) {
        if existing
            .reasons
            .iter()
            .any(|r| matches!(r, SelectionReason::SemanticMatch { .. }))
        {
            // Already has a semantic match, skip SameModule
        } else {
            // Check if this is the same module (we'll add SameModule reason separately if needed)
            let _ = callee_name; // Suppress unused warning
        }
    }
}

/// Add a file from call graph result.
fn add_file_from_call_graph(
    files: &mut HashMap<String, FileSelection>,
    node: &CallGraphNode,
    caller_name: &str,
) {
    add_file(
        files,
        &node.file_path,
        SelectionReason::Calls {
            callee: node.name.clone(),
            depth: node.depth,
        },
    );

    let _ = caller_name; // Suppress unused warning
}

/// Ranking key for a file: `(is_low_relevance, lexical_score, tier_score)`.
///
/// - **Relevance tier** (`is_low_relevance`): a file is in the top tier if it has
///   a direct semantic match **or** a lexical hit (a task token in its path/name).
///   Graph-only files with no lexical hit sort last, so a generic call-graph hub
///   (flat weight 0.8) never displaces an on-topic file.
/// - **`lexical_score`**: within a tier, more task-token hits rank first — this is
///   what surfaces `embeddings/openai.rs` for "…openai" over a higher-scored but
///   off-topic semantic match.
/// - **`tier_score`**: the semantic score for semantic matches, else
///   `relevance_score`; scores are only ever compared within the same scale.
fn rank_key(f: &FileSelection) -> (bool, f32, f32) {
    let sem = f.best_semantic_score();
    let is_low_relevance = sem.is_none() && f.lexical_score == 0.0;
    let tier_score = sem.unwrap_or(f.relevance_score);
    (is_low_relevance, f.lexical_score, tier_score)
}

/// Rank files for selection. The ordering is tiered and fully deterministic:
///
/// 1. Relevant files (semantic match or lexical hit) rank above files pulled in
///    only through call-graph expansion.
/// 2. Within a tier: more lexical hits first, then higher tier score.
/// 3. Ties break by `path` ascending. Because the input is collected from a
///    `HashMap`, this tie-break is what makes the command deterministic across runs
///    (many files share the same 0.5/0.7/0.8 weight and the same lexical score).
fn rank_files(files: &mut [FileSelection]) {
    files.sort_by(|a, b| {
        let a_key = rank_key(a);
        let b_key = rank_key(b);
        a_key
            .0
            .cmp(&b_key.0) // false (relevant) sorts before true (low-relevance)
            .then_with(|| {
                b_key
                    .1
                    .partial_cmp(&a_key.1) // lexical_score descending
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                b_key
                    .2
                    .partial_cmp(&a_key.2) // tier_score descending
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// Count tokens in a file, returning 0 on error.
fn count_file_token_safe(path: &str, encoding: Encoding) -> usize {
    count_file_tokens(Path::new(path), encoding)
        .map(|tc| tc.count)
        .unwrap_or(0)
}

/// Format the smart context result for display.
pub fn format_explain(result: &SmartContext) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "Selected {} files ({} tokens):\n\n",
        result.selected_files.len(),
        result.total_tokens
    ));

    for (i, file) in result.selected_files.iter().enumerate() {
        output.push_str(&format!(
            "{}. {} ({} tokens)\n",
            i + 1,
            file.path,
            file.token_count
        ));

        for reason in &file.reasons {
            output.push_str(&format!("   - {}\n", reason.description()));
        }
        output.push('\n');
    }

    if result.truncated {
        output.push_str(&format!(
            "({} files omitted due to token limit)\n",
            result.omitted_count
        ));
    }

    output
}

/// Format the smart context result for dry-run mode.
pub fn format_dry_run(result: &SmartContext) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "Would select {} files ({} tokens):\n",
        result.selected_files.len(),
        result.total_tokens
    ));

    for file in &result.selected_files {
        let primary_reason = file
            .reasons
            .first()
            .map(|r| match r {
                SelectionReason::SemanticMatch { .. } => "SemanticMatch",
                SelectionReason::CalledBy { .. } => "CalledBy",
                SelectionReason::Calls { .. } => "Calls",
                SelectionReason::SameModule { .. } => "SameModule",
                SelectionReason::Explicit => "Explicit",
            })
            .unwrap_or("Unknown");

        output.push_str(&format!(
            "  {} ({} tokens) - {}\n",
            file.path, file.token_count, primary_reason
        ));
    }

    if result.truncated {
        output.push_str(&format!(
            "\n({} files would be omitted due to token limit)\n",
            result.omitted_count
        ));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_reason_weight() {
        let semantic = SelectionReason::SemanticMatch {
            symbol: "test".to_string(),
            score: 0.9,
        };
        assert!((semantic.weight() - 0.9).abs() < 0.001);

        let called_by = SelectionReason::CalledBy {
            caller: "main".to_string(),
            depth: 1,
        };
        assert!((called_by.weight() - 0.8).abs() < 0.001);

        let called_by_depth2 = SelectionReason::CalledBy {
            caller: "main".to_string(),
            depth: 2,
        };
        assert!((called_by_depth2.weight() - 0.4).abs() < 0.001);

        let calls = SelectionReason::Calls {
            callee: "helper".to_string(),
            depth: 1,
        };
        assert!((calls.weight() - 0.7).abs() < 0.001);

        let same_module = SelectionReason::SameModule {
            symbol: "related".to_string(),
        };
        assert!((same_module.weight() - 0.5).abs() < 0.001);

        let explicit = SelectionReason::Explicit;
        assert!((explicit.weight() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_file_selection_add_reason() {
        let mut selection = FileSelection::new(
            "src/main.rs".to_string(),
            SelectionReason::Calls {
                callee: "helper".to_string(),
                depth: 1,
            },
        );

        assert!((selection.relevance_score - 0.7).abs() < 0.001);
        assert_eq!(selection.reasons.len(), 1);

        // Add a higher-weight reason
        selection.add_reason(SelectionReason::SemanticMatch {
            symbol: "main".to_string(),
            score: 0.95,
        });

        assert!((selection.relevance_score - 0.95).abs() < 0.001);
        assert_eq!(selection.reasons.len(), 2);
    }

    #[test]
    fn test_select_by_token_budget() {
        let files = vec![
            FileSelection {
                path: "a.rs".to_string(),
                relevance_score: 1.0,
                reasons: vec![SelectionReason::Explicit],
                token_count: 100,
                lexical_score: 0.0,
            },
            FileSelection {
                path: "b.rs".to_string(),
                relevance_score: 0.8,
                reasons: vec![SelectionReason::Explicit],
                token_count: 200,
                lexical_score: 0.0,
            },
            FileSelection {
                path: "c.rs".to_string(),
                relevance_score: 0.5,
                reasons: vec![SelectionReason::Explicit],
                token_count: 150,
                lexical_score: 0.0,
            },
        ];

        // All fit
        let (selected, total, omitted) = select_by_token_budget(files.clone(), 500);
        assert_eq!(selected.len(), 3);
        assert_eq!(total, 450);
        assert_eq!(omitted, 0);

        // Only first two fit
        let (selected, total, omitted) = select_by_token_budget(files.clone(), 300);
        assert_eq!(selected.len(), 2);
        assert_eq!(total, 300);
        assert_eq!(omitted, 1);

        // Only first fits
        let (selected, total, omitted) = select_by_token_budget(files, 150);
        assert_eq!(selected.len(), 1);
        assert_eq!(total, 100);
        assert_eq!(omitted, 2);
    }

    #[test]
    fn test_format_dry_run() {
        let result = SmartContext {
            task: "add caching".to_string(),
            selected_files: vec![
                FileSelection {
                    path: "src/main.rs".to_string(),
                    relevance_score: 0.9,
                    reasons: vec![SelectionReason::SemanticMatch {
                        symbol: "main".to_string(),
                        score: 0.9,
                    }],
                    token_count: 500,
                    lexical_score: 0.0,
                },
                FileSelection {
                    path: "src/lib.rs".to_string(),
                    relevance_score: 0.7,
                    reasons: vec![SelectionReason::Calls {
                        callee: "helper".to_string(),
                        depth: 1,
                    }],
                    token_count: 300,
                    lexical_score: 0.0,
                },
            ],
            total_tokens: 800,
            truncated: false,
            omitted_count: 0,
        };

        let output = format_dry_run(&result);
        assert!(output.contains("Would select 2 files"));
        assert!(output.contains("src/main.rs"));
        assert!(output.contains("SemanticMatch"));
    }

    #[test]
    fn test_smart_config_default() {
        let config = SmartConfig::default();
        assert_eq!(config.max_tokens, 8000);
        assert_eq!(config.depth, 2);
        assert_eq!(config.top, 10);
    }

    #[test]
    fn test_add_file_merges_reasons() {
        let mut files: HashMap<String, FileSelection> = HashMap::new();

        // First add
        add_file(
            &mut files,
            "src/main.rs",
            SelectionReason::SemanticMatch {
                symbol: "main".to_string(),
                score: 0.9,
            },
        );
        assert_eq!(files.len(), 1);
        assert!((files.get("src/main.rs").unwrap().relevance_score - 0.9).abs() < 0.001);

        // Second add to same file - should merge
        add_file(
            &mut files,
            "src/main.rs",
            SelectionReason::CalledBy {
                caller: "run".to_string(),
                depth: 1,
            },
        );
        assert_eq!(files.len(), 1); // Still only 1 file
        assert_eq!(files.get("src/main.rs").unwrap().reasons.len(), 2);
        // Score should be max of the two
        assert!((files.get("src/main.rs").unwrap().relevance_score - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_rank_files_sorts_by_relevance() {
        let mut files = vec![
            FileSelection {
                path: "low.rs".to_string(),
                relevance_score: 0.3,
                reasons: vec![SelectionReason::Explicit],
                token_count: 100,
                lexical_score: 0.0,
            },
            FileSelection {
                path: "high.rs".to_string(),
                relevance_score: 0.9,
                reasons: vec![SelectionReason::Explicit],
                token_count: 100,
                lexical_score: 0.0,
            },
            FileSelection {
                path: "mid.rs".to_string(),
                relevance_score: 0.5,
                reasons: vec![SelectionReason::Explicit],
                token_count: 100,
                lexical_score: 0.0,
            },
        ];

        rank_files(&mut files);

        assert_eq!(files[0].path, "high.rs");
        assert_eq!(files[1].path, "mid.rs");
        assert_eq!(files[2].path, "low.rs");
    }

    /// A file that directly matches the task embedding must rank above a file that
    /// was only pulled in through call-graph expansion, even though the graph
    /// neighbour's flat weight (0.8) is numerically larger than the compressed
    /// semantic score (0.5). This is the core relevance regression.
    #[test]
    fn test_semantic_match_ranks_above_graph_only() {
        let mut files = vec![
            FileSelection {
                path: "graph_hub.rs".to_string(),
                relevance_score: 0.8, // CalledBy depth 1
                reasons: vec![SelectionReason::CalledBy {
                    caller: "run".to_string(),
                    depth: 1,
                }],
                token_count: 100,
                lexical_score: 0.0,
            },
            FileSelection {
                path: "on_topic.rs".to_string(),
                relevance_score: 0.5,
                reasons: vec![SelectionReason::SemanticMatch {
                    symbol: "run_sql".to_string(),
                    score: 0.5,
                }],
                token_count: 100,
                lexical_score: 0.0,
            },
        ];

        rank_files(&mut files);

        assert_eq!(
            files[0].path, "on_topic.rs",
            "semantic match must outrank a higher-weighted graph-only file"
        );
        assert_eq!(files[1].path, "graph_hub.rs");
    }

    /// Within the semantic tier, higher score wins; equal scores break by path so
    /// the order is stable.
    #[test]
    fn test_semantic_tier_orders_by_score_then_path() {
        let mk = |path: &str, score: f32| FileSelection {
            path: path.to_string(),
            relevance_score: score,
            reasons: vec![SelectionReason::SemanticMatch {
                symbol: "s".to_string(),
                score,
            }],
            token_count: 100,
            lexical_score: 0.0,
        };
        // Two files tie at 0.6; "a.rs" must precede "b.rs" by path.
        let mut files = vec![mk("b.rs", 0.6), mk("c.rs", 0.4), mk("a.rs", 0.6)];

        rank_files(&mut files);

        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["a.rs", "b.rs", "c.rs"]
        );
    }

    /// Ranking must not depend on the (HashMap-derived) input order: the same set of
    /// selections presented in any order yields the same ranked paths. This is what
    /// makes `ctx smart` deterministic across runs.
    #[test]
    fn test_rank_files_is_deterministic() {
        let make_set = || {
            vec![
                FileSelection {
                    path: "z_graph.rs".to_string(),
                    relevance_score: 0.8,
                    reasons: vec![SelectionReason::CalledBy {
                        caller: "run".to_string(),
                        depth: 1,
                    }],
                    token_count: 100,
                    lexical_score: 0.0,
                },
                FileSelection {
                    path: "a_graph.rs".to_string(),
                    relevance_score: 0.8, // ties with z_graph.rs -> path breaks it
                    reasons: vec![SelectionReason::CalledBy {
                        caller: "run".to_string(),
                        depth: 1,
                    }],
                    token_count: 100,
                    lexical_score: 0.0,
                },
                FileSelection {
                    path: "seed.rs".to_string(),
                    relevance_score: 0.5,
                    reasons: vec![SelectionReason::SemanticMatch {
                        symbol: "seed".to_string(),
                        score: 0.5,
                    }],
                    token_count: 100,
                    lexical_score: 0.0,
                },
            ]
        };

        let mut a = make_set();
        rank_files(&mut a);
        let order_a: Vec<String> = a.iter().map(|f| f.path.clone()).collect();

        // Present the same set reversed; ranking must produce the identical order.
        let mut b = make_set();
        b.reverse();
        rank_files(&mut b);
        let order_b: Vec<String> = b.iter().map(|f| f.path.clone()).collect();

        assert_eq!(order_a, order_b);
        // Semantic seed first, then graph files tie-broken by path.
        assert_eq!(order_a, vec!["seed.rs", "a_graph.rs", "z_graph.rs"]);
    }

    /// A graph-only file whose path/name matches a task token (lexical_score > 0)
    /// is promoted into the relevant tier, above a semantic match that shares no
    /// task token. This is what surfaces `embeddings/openai.rs` for "…openai".
    #[test]
    fn test_lexical_promotes_graph_only_file() {
        let mut files = vec![
            FileSelection {
                path: "off_topic_semantic.rs".to_string(),
                relevance_score: 0.6,
                reasons: vec![SelectionReason::SemanticMatch {
                    symbol: "unrelated".to_string(),
                    score: 0.6,
                }],
                token_count: 100,
                lexical_score: 0.0,
            },
            FileSelection {
                path: "embeddings/openai.rs".to_string(),
                relevance_score: 0.7, // Calls, graph-only
                reasons: vec![SelectionReason::Calls {
                    callee: "embed".to_string(),
                    depth: 1,
                }],
                token_count: 100,
                lexical_score: 2.0, // "embeddings" + "openai"
            },
        ];

        rank_files(&mut files);

        assert_eq!(
            files[0].path, "embeddings/openai.rs",
            "a lexical hit must outrank a semantic match with no task-token overlap"
        );
    }

    /// Within the relevant tier, more lexical hits rank first; equal lexical scores
    /// fall back to tier score, then path.
    #[test]
    fn test_lexical_orders_within_tier() {
        let mk = |path: &str, score: f32, lex: f32| FileSelection {
            path: path.to_string(),
            relevance_score: score,
            reasons: vec![SelectionReason::SemanticMatch {
                symbol: "s".to_string(),
                score,
            }],
            token_count: 100,
            lexical_score: lex,
        };
        // two_hits has the most lexical overlap and must lead despite a lower score.
        let mut files = vec![
            mk("high_score_no_lexical.rs", 0.9, 0.0),
            mk("two_hits.rs", 0.4, 2.0),
            mk("one_hit.rs", 0.4, 1.0),
        ];

        rank_files(&mut files);

        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["two_hits.rs", "one_hit.rs", "high_score_no_lexical.rs"]
        );
    }

    /// `compute_lexical_score` counts distinct task tokens present in a file's
    /// path (only), and does not count symbol names — so ubiquitous identifiers
    /// like `ctx` cannot inflate the score.
    #[test]
    fn test_compute_lexical_score() {
        let sel = FileSelection {
            path: "src/embeddings/openai.rs".to_string(),
            relevance_score: 0.5,
            // Symbol name deliberately contains "ctx"; it must NOT be counted.
            reasons: vec![SelectionReason::SemanticMatch {
                symbol: "ctx_embed".to_string(),
                score: 0.5,
            }],
            token_count: 100,
            lexical_score: 0.0,
        };
        // "embeddings" + "openai" from the path overlap the task; "with" is a stopword.
        let task = lexical_tokens("generate embeddings with openai");
        assert!((sel.compute_lexical_score(&task) - 2.0).abs() < 0.001);
        // "ctx" appears in the symbol but not the path, so it does not match.
        let ctx_task = lexical_tokens("ctx tooling");
        assert!((sel.compute_lexical_score(&ctx_task)).abs() < 0.001);
        // No overlap -> 0.
        let other = lexical_tokens("parse solidity contracts");
        assert!((sel.compute_lexical_score(&other)).abs() < 0.001);
    }

    /// The rank-1 file is always included even when it alone exceeds the budget,
    /// rather than being silently dropped in favour of smaller lower-ranked files.
    #[test]
    fn test_budget_includes_oversized_top_file() {
        let ranked = vec![
            FileSelection {
                path: "huge_top.rs".to_string(),
                relevance_score: 0.9,
                reasons: vec![SelectionReason::SemanticMatch {
                    symbol: "s".to_string(),
                    score: 0.9,
                }],
                token_count: 9000, // larger than the whole budget
                lexical_score: 1.0,
            },
            FileSelection {
                path: "small_next.rs".to_string(),
                relevance_score: 0.5,
                reasons: vec![SelectionReason::SemanticMatch {
                    symbol: "s".to_string(),
                    score: 0.5,
                }],
                token_count: 500,
                lexical_score: 0.0,
            },
        ];

        let (selected, total, omitted) = select_with_guaranteed_top(ranked, 8000);

        assert_eq!(selected.len(), 1, "only the oversized top file is included");
        assert_eq!(selected[0].path, "huge_top.rs");
        assert_eq!(total, 9000);
        assert_eq!(omitted, 1, "the smaller file is omitted for lack of budget");
    }

    /// When the top file fits, the remainder is first-fit against the leftover
    /// budget (same behaviour as before for the non-oversized case).
    #[test]
    fn test_budget_first_fits_remainder() {
        let ranked = vec![
            FileSelection {
                path: "top.rs".to_string(),
                relevance_score: 0.9,
                reasons: vec![SelectionReason::Explicit],
                token_count: 3000,
                lexical_score: 0.0,
            },
            FileSelection {
                path: "mid.rs".to_string(),
                relevance_score: 0.8,
                reasons: vec![SelectionReason::Explicit],
                token_count: 4000,
                lexical_score: 0.0,
            },
            FileSelection {
                path: "tail.rs".to_string(),
                relevance_score: 0.7,
                reasons: vec![SelectionReason::Explicit],
                token_count: 2000, // would overflow the 8000 budget after top+mid
                lexical_score: 0.0,
            },
        ];

        let (selected, total, omitted) = select_with_guaranteed_top(ranked, 8000);

        assert_eq!(
            selected.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["top.rs", "mid.rs"]
        );
        assert_eq!(total, 7000);
        assert_eq!(omitted, 1);
    }

    #[test]
    fn test_selection_reason_description() {
        let semantic = SelectionReason::SemanticMatch {
            symbol: "test".to_string(),
            score: 0.9,
        };
        assert!(semantic.description().contains("SemanticMatch"));
        assert!(semantic.description().contains("test"));

        let called_by = SelectionReason::CalledBy {
            caller: "main".to_string(),
            depth: 2,
        };
        assert!(called_by.description().contains("CalledBy"));
        assert!(called_by.description().contains("main"));
        assert!(called_by.description().contains("2"));
    }
}
