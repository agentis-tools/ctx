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

use std::collections::HashMap;
use std::path::Path;

use crate::analytics::{Analytics, CallGraphNode, ImpactNode};
use crate::db::Database;
use crate::embeddings::{semantic_search, Embedding, EmbeddingProvider, SearchResult};
use crate::error::{CtxError, Result};
use crate::tokens::{count_file_tokens, select_by_token_budget, Encoding, HasTokenCount};

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
    // 1. Embed the task description
    let task_embedding = provider.embed(task)?;

    smart_context_with_embedding(db, analytics, task, &task_embedding, config)
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
    // 2. Semantic search for matching symbols
    let matches = semantic_search(db, task_embedding, config.top)?;

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
        expand_symbol(&mut files, analytics, result, config.depth)?;
    }

    // 4. Convert to vector and count tokens
    let mut selections: Vec<FileSelection> = files.into_values().collect();

    // Count tokens for each file
    for selection in &mut selections {
        selection.token_count = count_file_token_safe(&selection.path, config.encoding);
    }

    // 5. Rank files by relevance
    rank_files(&mut selections);

    // 6. Apply token limit
    let (selected, total_tokens, omitted) = select_by_token_budget(selections, config.max_tokens);

    Ok(SmartContext {
        task: task.to_string(),
        selected_files: selected,
        total_tokens,
        truncated: omitted > 0,
        omitted_count: omitted,
    })
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
) -> Result<()> {
    // Expand callers (who calls this symbol - impact analysis)
    if let Ok(callers) = analytics.impact_analysis(&result.symbol_id, depth) {
        for caller in callers {
            add_file_from_impact(files, &caller, &result.name);
        }
    }

    // Expand callees (what does this symbol call - call graph)
    if let Ok(callees) = analytics.call_graph(&result.symbol_id, depth) {
        for callee in callees {
            add_file_from_call_graph(files, &callee, &result.name);
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

/// Rank files by relevance score (descending).
fn rank_files(files: &mut [FileSelection]) {
    files.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
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
            },
            FileSelection {
                path: "b.rs".to_string(),
                relevance_score: 0.8,
                reasons: vec![SelectionReason::Explicit],
                token_count: 200,
            },
            FileSelection {
                path: "c.rs".to_string(),
                relevance_score: 0.5,
                reasons: vec![SelectionReason::Explicit],
                token_count: 150,
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
                },
                FileSelection {
                    path: "src/lib.rs".to_string(),
                    relevance_score: 0.7,
                    reasons: vec![SelectionReason::Calls {
                        callee: "helper".to_string(),
                        depth: 1,
                    }],
                    token_count: 300,
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
            },
            FileSelection {
                path: "high.rs".to_string(),
                relevance_score: 0.9,
                reasons: vec![SelectionReason::Explicit],
                token_count: 100,
            },
            FileSelection {
                path: "mid.rs".to_string(),
                relevance_score: 0.5,
                reasons: vec![SelectionReason::Explicit],
                token_count: 100,
            },
        ];

        rank_files(&mut files);

        assert_eq!(files[0].path, "high.rs");
        assert_eq!(files[1].path, "mid.rs");
        assert_eq!(files[2].path, "low.rs");
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
