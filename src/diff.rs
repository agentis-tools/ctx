//! Diff-aware context generation for code review and change understanding.
//!
//! This module provides functionality to:
//! - Parse git diff output to identify changed files
//! - Find symbols affected by changes
//! - Expand context using call graph analysis
//! - Generate context focused on code changes
//!
//! # Usage
//!
//! ```ignore
//! let config = DiffConfig::default();
//! let result = diff_context("HEAD~1", &db, &analytics, config)?;
//! for file in result.changed_files {
//!     println!("{}: {:?}", file.path, file.change_type);
//! }
//! ```

use std::collections::{HashMap, HashSet};
use std::process::Command;

use crate::analytics::Analytics;
use crate::db::Database;
use crate::error::{CtxError, Result};
use crate::tokens::{count_tokens_with_encoding, select_by_token_budget, Encoding, HasTokenCount};
use crate::walker::FilePatternFilter;

/// Configuration for diff-aware context generation.
#[derive(Debug, Clone)]
pub struct DiffConfig {
    /// Maximum tokens in output
    pub max_tokens: usize,
    /// Call graph context depth
    pub depth: i32,
    /// Only include changed files (no context expansion)
    pub changes_only: bool,
    /// Include staged changes only
    pub staged: bool,
    /// Include change summary in output
    #[allow(dead_code)] // Config option for future use
    pub summary: bool,
    /// Tokenizer encoding
    pub encoding: Encoding,
}

impl Default for DiffConfig {
    fn default() -> Self {
        Self {
            max_tokens: 8000,
            depth: 1,
            changes_only: false,
            staged: false,
            summary: false,
            encoding: Encoding::default(),
        }
    }
}

/// Type of change for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeType {
    /// File was added
    Added,
    /// File was modified
    Modified,
    /// File was deleted
    Deleted,
    /// File was renamed
    Renamed,
    /// File was copied
    Copied,
    /// Unknown change type
    Unknown,
}

impl ChangeType {
    /// Parse change type from git status letter.
    pub fn from_git_status(status: char) -> Self {
        match status {
            'A' => ChangeType::Added,
            'M' => ChangeType::Modified,
            'D' => ChangeType::Deleted,
            'R' => ChangeType::Renamed,
            'C' => ChangeType::Copied,
            _ => ChangeType::Unknown,
        }
    }

    /// Get a human-readable description.
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeType::Added => "added",
            ChangeType::Modified => "modified",
            ChangeType::Deleted => "deleted",
            ChangeType::Renamed => "renamed",
            ChangeType::Copied => "copied",
            ChangeType::Unknown => "unknown",
        }
    }
}

/// A changed file with metadata.
#[derive(Debug, Clone)]
pub struct ChangedFile {
    /// File path (relative to repo root)
    pub path: String,
    /// Type of change
    pub change_type: ChangeType,
    /// Lines added
    pub lines_added: usize,
    /// Lines removed
    pub lines_removed: usize,
    /// Line ranges that changed (start, end) - 1-indexed
    pub changed_ranges: Vec<(u32, u32)>,
    /// Original path (for renames)
    pub original_path: Option<String>,
}

impl ChangedFile {
    /// Create a new changed file.
    pub fn new(path: String, change_type: ChangeType) -> Self {
        Self {
            path,
            change_type,
            lines_added: 0,
            lines_removed: 0,
            changed_ranges: Vec::new(),
            original_path: None,
        }
    }
}

/// A file selected for context with relevance information.
#[derive(Debug, Clone)]
pub struct ContextFile {
    /// File path
    pub path: String,
    /// Priority score (1.0 = changed, 0.8 = direct caller/callee, 0.6 = indirect)
    pub priority: f32,
    /// Reason for inclusion
    pub reason: ContextReason,
    /// Token count
    pub token_count: usize,
}

impl HasTokenCount for ContextFile {
    fn token_count(&self) -> usize {
        self.token_count
    }
}

/// Reason why a file is included in context.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Variant data used for debug output
pub enum ContextReason {
    /// File was directly changed
    Changed(ChangeType),
    /// Calls a changed symbol
    CallsChanged { symbol: String, depth: i32 },
    /// Called by a changed symbol
    CalledByChanged { symbol: String, depth: i32 },
}

impl ContextReason {
    /// Get a human-readable description.
    #[allow(dead_code)] // Public API for future use
    pub fn description(&self) -> String {
        match self {
            ContextReason::Changed(ct) => format!("changed ({})", ct.as_str()),
            ContextReason::CallsChanged { symbol, depth } => {
                format!("calls changed symbol '{}' (depth {})", symbol, depth)
            }
            ContextReason::CalledByChanged { symbol, depth } => {
                format!("called by changed symbol '{}' (depth {})", symbol, depth)
            }
        }
    }
}

/// A symbol affected by changes.
#[derive(Debug, Clone)]
pub struct AffectedSymbol {
    /// Symbol name
    pub name: String,
    /// File containing the symbol
    pub file_path: String,
    /// Line number
    #[allow(dead_code)] // Used for debug output
    pub line: u32,
    /// Symbol kind
    pub kind: String,
}

/// Result of diff context analysis.
#[derive(Debug, Clone)]
pub struct DiffContext {
    /// The revision that was analyzed
    pub revision: String,
    /// Changed files with their metadata
    pub changed_files: Vec<ChangedFile>,
    /// Symbols affected by changes
    pub affected_symbols: Vec<AffectedSymbol>,
    /// Files selected for context
    pub context_files: Vec<ContextFile>,
    /// Total token count
    pub total_tokens: usize,
    /// Whether output was truncated
    pub truncated: bool,
    /// Number of files omitted
    pub omitted_count: usize,
}

/// Check if current directory is a git repository.
pub fn is_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get the list of changed files for a revision.
pub fn get_changed_files(revision: &str, staged: bool) -> Result<Vec<ChangedFile>> {
    if !is_git_repo() {
        return Err(CtxError::NotGitRepo);
    }

    // Build the git diff command
    let mut args = vec!["diff", "--name-status"];

    if staged {
        args.push("--staged");
    } else {
        args.push(revision);
    }

    let output = Command::new("git").args(&args).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("unknown revision") || stderr.contains("bad revision") {
            return Err(CtxError::InvalidRevision(revision.to_string()));
        }
        return Err(CtxError::git(stderr.to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let files = parse_diff_output(&stdout);

    if files.is_empty() {
        return Err(CtxError::NoChanges);
    }

    // Get detailed diff info for each file
    let mut detailed_files = Vec::new();
    for mut file in files {
        if file.change_type != ChangeType::Deleted {
            if let Ok(ranges) = get_diff_line_ranges(revision, &file.path, staged) {
                file.changed_ranges = ranges;
            }
            if let Ok((added, removed)) = get_diff_stats(revision, &file.path, staged) {
                file.lines_added = added;
                file.lines_removed = removed;
            }
        }
        detailed_files.push(file);
    }

    Ok(detailed_files)
}

/// Get changed files restricted to positional file patterns.
///
/// A rename is in scope when either its original or destination path matches,
/// mirroring how users reason about path-limited git diffs. Deleted paths are
/// retained in the changed-file summary even though they cannot be emitted as
/// context content.
///
/// Returns an empty list when the revision has changes that the scope excludes;
/// that is a legitimate answer to a narrower question, not a failure. A revision
/// with no changes at all still fails with [`CtxError::NoChanges`], because
/// `get_changed_files` rejects it before the scope is applied -- so an empty
/// list here always means the patterns excluded everything.
pub fn get_changed_files_filtered(
    revision: &str,
    staged: bool,
    filter: &FilePatternFilter,
) -> Result<Vec<ChangedFile>> {
    let mut files = get_changed_files(revision, staged)?;
    files.retain(|file| {
        filter.matches(&file.path)
            || file
                .original_path
                .as_deref()
                .is_some_and(|path| filter.matches(path))
    });
    Ok(files)
}

/// Parse git diff --name-status output.
pub fn parse_diff_output(output: &str) -> Vec<ChangedFile> {
    let mut files = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Format: "M\tpath" or "R100\told_path\tnew_path"
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.is_empty() {
            continue;
        }

        let status_str = parts[0];
        let status_char = status_str.chars().next().unwrap_or('?');
        let change_type = ChangeType::from_git_status(status_char);

        let (path, original_path) = if change_type == ChangeType::Renamed && parts.len() >= 3 {
            (parts[2].to_string(), Some(parts[1].to_string()))
        } else if parts.len() >= 2 {
            (parts[1].to_string(), None)
        } else {
            continue;
        };

        let mut file = ChangedFile::new(path, change_type);
        file.original_path = original_path;
        files.push(file);
    }

    files
}

/// Get the changed line ranges for a file.
fn get_diff_line_ranges(revision: &str, path: &str, staged: bool) -> Result<Vec<(u32, u32)>> {
    let mut args = vec!["diff", "-U0", "--no-color"];

    if staged {
        args.push("--staged");
    } else {
        args.push(revision);
    }

    args.push("--");
    args.push(path);

    let output = Command::new("git").args(&args).output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_diff_hunks(&stdout))
}

/// Parse diff hunks to extract line ranges.
fn parse_diff_hunks(diff_output: &str) -> Vec<(u32, u32)> {
    let mut ranges = Vec::new();

    for line in diff_output.lines() {
        // Hunk header: @@ -old_start,old_count +new_start,new_count @@
        if line.starts_with("@@") {
            if let Some(new_range) = parse_hunk_header(line) {
                ranges.push(new_range);
            }
        }
    }

    ranges
}

/// Parse a hunk header to get the new file line range.
fn parse_hunk_header(header: &str) -> Option<(u32, u32)> {
    // @@ -1,5 +1,7 @@ or @@ -1 +1,7 @@
    let parts: Vec<&str> = header.split_whitespace().collect();

    for part in parts {
        if part.starts_with('+') && !part.starts_with("+++") {
            let range_str = &part[1..];
            let range_parts: Vec<&str> = range_str.split(',').collect();

            let start: u32 = range_parts.first()?.parse().ok()?;
            let count: u32 = range_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);

            if count == 0 {
                return None; // Pure deletion, no new lines
            }

            return Some((start, start + count - 1));
        }
    }

    None
}

/// Get diff statistics (lines added, removed) for a file.
fn get_diff_stats(revision: &str, path: &str, staged: bool) -> Result<(usize, usize)> {
    let mut args = vec!["diff", "--numstat"];

    if staged {
        args.push("--staged");
    } else {
        args.push(revision);
    }

    args.push("--");
    args.push(path);

    let output = Command::new("git").args(&args).output()?;

    if !output.status.success() {
        return Ok((0, 0));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let added: usize = parts[0].parse().unwrap_or(0);
            let removed: usize = parts[1].parse().unwrap_or(0);
            return Ok((added, removed));
        }
    }

    Ok((0, 0))
}

/// Find symbols that were affected by changes in a file.
pub fn find_symbols_in_lines(
    db: &Database,
    file_path: &str,
    changed_ranges: &[(u32, u32)],
) -> Result<Vec<AffectedSymbol>> {
    let symbols = db.find_symbols_in_file(file_path)?;

    let mut affected = Vec::new();

    for symbol in symbols {
        // Check if any changed range overlaps with the symbol's lines
        for &(start, end) in changed_ranges {
            if ranges_overlap(symbol.line_start, symbol.line_end, start, end) {
                affected.push(AffectedSymbol {
                    name: symbol.name.clone(),
                    file_path: symbol.file_path.clone(),
                    line: symbol.line_start,
                    kind: symbol.kind.as_str().to_string(),
                });
                break;
            }
        }
    }

    Ok(affected)
}

/// Check if two line ranges overlap.
fn ranges_overlap(s1_start: u32, s1_end: u32, s2_start: u32, s2_end: u32) -> bool {
    s1_start <= s2_end && s2_start <= s1_end
}

/// Generate diff-aware context.
pub fn diff_context(
    revision: &str,
    db: &Database,
    analytics: &Analytics,
    config: DiffConfig,
) -> Result<DiffContext> {
    let root = std::env::current_dir().unwrap_or_default();
    let filter = FilePatternFilter::all(&root);
    diff_context_filtered(revision, db, analytics, config, &filter)
}

/// Generate diff-aware context restricted to positional file patterns.
///
/// Changed files are filtered before symbol lookup. Directly changed rename
/// destinations remain context when the original path matched; graph-expanded
/// files must independently match the filter.
pub fn diff_context_filtered(
    revision: &str,
    db: &Database,
    analytics: &Analytics,
    config: DiffConfig,
    filter: &FilePatternFilter,
) -> Result<DiffContext> {
    // 1. Get changed files
    let changed_files = get_changed_files_filtered(revision, config.staged, filter)?;

    // 2. Find affected symbols
    let mut affected_symbols = Vec::new();
    for file in &changed_files {
        if file.change_type != ChangeType::Deleted && !file.changed_ranges.is_empty() {
            if let Ok(symbols) = find_symbols_in_lines(db, &file.path, &file.changed_ranges) {
                affected_symbols.extend(symbols);
            }
        }
    }

    // 3. Build context file list
    let mut context_files: HashMap<String, ContextFile> = HashMap::new();

    // Add changed files first (priority 1.0)
    for file in &changed_files {
        if file.change_type != ChangeType::Deleted {
            context_files.insert(
                file.path.clone(),
                ContextFile {
                    path: file.path.clone(),
                    priority: 1.0,
                    reason: ContextReason::Changed(file.change_type),
                    token_count: 0,
                },
            );
        }
    }

    // 4. Expand context using call graph (if not changes_only)
    if !config.changes_only {
        for symbol in &affected_symbols {
            expand_context_for_symbol(&mut context_files, analytics, symbol, config.depth, filter);
        }
    }

    // 5. Count tokens for each file
    let root = std::env::current_dir().unwrap_or_default();
    for ctx_file in context_files.values_mut() {
        let path = root.join(&ctx_file.path);
        if let Ok(content) = std::fs::read_to_string(&path) {
            ctx_file.token_count =
                count_tokens_with_encoding(&content, config.encoding).unwrap_or(0);
        }
    }

    // 6. Sort by priority and path, then select files within token limit
    let mut files: Vec<ContextFile> = context_files.into_values().collect();
    sort_context_files(&mut files);

    let (selected, total_tokens, omitted) = select_by_token_budget(files, config.max_tokens);

    Ok(DiffContext {
        revision: revision.to_string(),
        changed_files,
        affected_symbols,
        context_files: selected,
        total_tokens,
        truncated: omitted > 0,
        omitted_count: omitted,
    })
}

/// Order context files deterministically before greedy token-budget selection.
fn sort_context_files(files: &mut [ContextFile]) {
    files.sort_by(|a, b| {
        b.priority
            .total_cmp(&a.priority)
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// Expand context for a symbol using call graph analysis.
fn expand_context_for_symbol(
    context_files: &mut HashMap<String, ContextFile>,
    analytics: &Analytics,
    symbol: &AffectedSymbol,
    depth: i32,
    filter: &FilePatternFilter,
) {
    // Find callers (impact analysis)
    if let Ok(callers) = analytics.impact_analysis(&symbol.name, depth) {
        for caller in callers {
            if !filter.matches(&caller.file_path) {
                continue;
            }
            let priority = 0.8 / (caller.distance as f32).max(1.0);
            add_context_file(
                context_files,
                &caller.file_path,
                priority,
                ContextReason::CalledByChanged {
                    symbol: symbol.name.clone(),
                    depth: caller.distance,
                },
            );
        }
    }

    // Find callees (call graph)
    if let Ok(callees) = analytics.call_graph(&symbol.name, depth) {
        for callee in callees {
            if !filter.matches(&callee.file_path) {
                continue;
            }
            let priority = 0.6 / (callee.depth as f32).max(1.0);
            add_context_file(
                context_files,
                &callee.file_path,
                priority,
                ContextReason::CallsChanged {
                    symbol: symbol.name.clone(),
                    depth: callee.depth,
                },
            );
        }
    }
}

/// Add a context file, keeping highest priority.
fn add_context_file(
    context_files: &mut HashMap<String, ContextFile>,
    path: &str,
    priority: f32,
    reason: ContextReason,
) {
    if let Some(existing) = context_files.get_mut(path) {
        // Keep higher priority reason
        if priority > existing.priority {
            existing.priority = priority;
            existing.reason = reason;
        }
    } else {
        context_files.insert(
            path.to_string(),
            ContextFile {
                path: path.to_string(),
                priority,
                reason,
                token_count: 0,
            },
        );
    }
}

/// Format a summary of changes for output.
pub fn format_summary(context: &DiffContext) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "Changes in {}: {} files changed\n\n",
        context.revision,
        context.changed_files.len()
    ));

    output.push_str("Changed files:\n");
    for file in &context.changed_files {
        let stats = if file.lines_added > 0 || file.lines_removed > 0 {
            format!("+{} -{}", file.lines_added, file.lines_removed)
        } else {
            file.change_type.as_str().to_string()
        };
        output.push_str(&format!("  {} ({})\n", file.path, stats));
    }

    if !context.affected_symbols.is_empty() {
        output.push_str("\nAffected symbols:\n");
        let mut seen = HashSet::new();
        for symbol in &context.affected_symbols {
            let key = format!("{}:{}", symbol.file_path, symbol.name);
            if seen.insert(key) {
                output.push_str(&format!(
                    "  {} ({}) in {}\n",
                    symbol.name, symbol.kind, symbol.file_path
                ));
            }
        }
    }

    output.push_str(&format!(
        "\nContext: {} files ({} tokens)",
        context.context_files.len(),
        context.total_tokens
    ));

    if context.truncated {
        output.push_str(&format!(", {} omitted", context.omitted_count));
    }

    output.push('\n');
    output
}

/// PR information from GitHub.
#[derive(Debug, Clone)]
pub struct PrInfo {
    /// PR number
    pub number: u64,
    /// PR title
    pub title: String,
    /// PR description/body
    pub body: String,
    /// Author login
    pub author: String,
    /// Base branch
    pub base: String,
    /// Head branch
    pub head: String,
    /// Changed files
    pub files: Vec<PrFile>,
    /// Comments
    pub comments: Vec<PrComment>,
}

/// A file changed in a PR.
#[derive(Debug, Clone)]
pub struct PrFile {
    /// File path
    pub path: String,
    /// Additions
    pub additions: usize,
    /// Deletions
    pub deletions: usize,
}

/// A comment on a PR.
#[derive(Debug, Clone)]
pub struct PrComment {
    /// Author login
    pub author: String,
    /// Comment body
    pub body: String,
    /// File path (if line comment)
    pub path: Option<String>,
    /// Line number (if line comment)
    pub line: Option<u32>,
}

/// Get PR information using gh CLI.
pub fn get_pr_info(pr: &str, repo: Option<&str>) -> Result<PrInfo> {
    let mut args = vec![
        "pr",
        "view",
        pr,
        "--json",
        "number,title,body,author,baseRefName,headRefName,files,comments",
    ];

    if let Some(r) = repo {
        args.push("--repo");
        args.push(r);
    }

    let output = Command::new("gh").args(&args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CtxError::git(
                "GitHub CLI (gh) not found. Install it from https://cli.github.com/".to_string(),
            )
        } else {
            CtxError::Io(e)
        }
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("Could not resolve") || stderr.contains("not found") {
            return Err(CtxError::InvalidRevision(format!("PR #{} not found", pr)));
        }
        return Err(CtxError::git(stderr.to_string()));
    }

    // Parse JSON response
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| CtxError::git(format!("Failed to parse gh output: {}", e)))?;

    Ok(PrInfo {
        number: json["number"].as_u64().unwrap_or(0),
        title: json["title"].as_str().unwrap_or("").to_string(),
        body: json["body"].as_str().unwrap_or("").to_string(),
        author: json["author"]["login"].as_str().unwrap_or("").to_string(),
        base: json["baseRefName"].as_str().unwrap_or("main").to_string(),
        head: json["headRefName"].as_str().unwrap_or("").to_string(),
        files: parse_pr_files(&json["files"]),
        comments: parse_pr_comments(&json["comments"]),
    })
}

/// Parse PR files from JSON.
fn parse_pr_files(json: &serde_json::Value) -> Vec<PrFile> {
    let mut files = Vec::new();
    if let Some(arr) = json.as_array() {
        for item in arr {
            files.push(PrFile {
                path: item["path"].as_str().unwrap_or("").to_string(),
                additions: item["additions"].as_u64().unwrap_or(0) as usize,
                deletions: item["deletions"].as_u64().unwrap_or(0) as usize,
            });
        }
    }
    files
}

/// Parse PR comments from JSON.
fn parse_pr_comments(json: &serde_json::Value) -> Vec<PrComment> {
    let mut comments = Vec::new();
    if let Some(arr) = json.as_array() {
        for item in arr {
            comments.push(PrComment {
                author: item["author"]["login"].as_str().unwrap_or("").to_string(),
                body: item["body"].as_str().unwrap_or("").to_string(),
                path: item["path"].as_str().map(|s| s.to_string()),
                line: item["line"].as_u64().map(|n| n as u32),
            });
        }
    }
    comments
}

/// Format PR info for context output.
pub fn format_pr_header(pr: &PrInfo, include_comments: bool) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "PR #{}: {} (by @{})\n",
        pr.number, pr.title, pr.author
    ));
    output.push_str(&format!("Branch: {} -> {}\n", pr.head, pr.base));

    if !pr.body.is_empty() {
        output.push_str("\nDescription:\n");
        for line in pr.body.lines().take(10) {
            output.push_str(&format!("  {}\n", line));
        }
        if pr.body.lines().count() > 10 {
            output.push_str("  ...\n");
        }
    }

    output.push_str(&format!("\nFiles changed: {}\n", pr.files.len()));
    for file in &pr.files {
        output.push_str(&format!(
            "  {} (+{} -{})\n",
            file.path, file.additions, file.deletions
        ));
    }

    if include_comments && !pr.comments.is_empty() {
        output.push_str(&format!("\nComments ({}):\n", pr.comments.len()));
        for comment in &pr.comments {
            let location = match (&comment.path, comment.line) {
                (Some(p), Some(l)) => format!(" at {}:{}", p, l),
                _ => String::new(),
            };
            output.push_str(&format!("  @{}{}: ", comment.author, location));
            let preview: String = comment.body.chars().take(80).collect();
            output.push_str(&preview);
            if comment.body.len() > 80 {
                output.push_str("...");
            }
            output.push('\n');
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_change_type_from_git_status() {
        assert_eq!(ChangeType::from_git_status('A'), ChangeType::Added);
        assert_eq!(ChangeType::from_git_status('M'), ChangeType::Modified);
        assert_eq!(ChangeType::from_git_status('D'), ChangeType::Deleted);
        assert_eq!(ChangeType::from_git_status('R'), ChangeType::Renamed);
        assert_eq!(ChangeType::from_git_status('C'), ChangeType::Copied);
        assert_eq!(ChangeType::from_git_status('X'), ChangeType::Unknown);
    }

    #[test]
    fn test_parse_diff_output() {
        let output = "M\tsrc/main.rs\nA\tsrc/new.rs\nD\tsrc/old.rs\n";
        let files = parse_diff_output(output);

        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].change_type, ChangeType::Modified);
        assert_eq!(files[1].path, "src/new.rs");
        assert_eq!(files[1].change_type, ChangeType::Added);
        assert_eq!(files[2].path, "src/old.rs");
        assert_eq!(files[2].change_type, ChangeType::Deleted);
    }

    #[test]
    fn test_parse_diff_output_rename() {
        let output = "R100\tsrc/old_name.rs\tsrc/new_name.rs\n";
        let files = parse_diff_output(output);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/new_name.rs");
        assert_eq!(files[0].change_type, ChangeType::Renamed);
        assert_eq!(files[0].original_path, Some("src/old_name.rs".to_string()));
    }

    #[test]
    fn test_parse_hunk_header() {
        // Standard format
        assert_eq!(parse_hunk_header("@@ -1,5 +1,7 @@ fn main"), Some((1, 7)));

        // Single line
        assert_eq!(parse_hunk_header("@@ -1 +1 @@"), Some((1, 1)));

        // Addition at end of file
        assert_eq!(parse_hunk_header("@@ -10,0 +11,5 @@"), Some((11, 15)));

        // Pure deletion (count 0)
        assert_eq!(parse_hunk_header("@@ -1,5 +1,0 @@"), None);
    }

    #[test]
    fn test_parse_diff_hunks() {
        let diff = r#"diff --git a/src/main.rs b/src/main.rs
index abc123..def456 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,7 @@
 fn main() {
+    // New comment
+    setup();
     run();
@@ -10,3 +12,5 @@
 fn helper() {
+    // More changes
"#;
        let ranges = parse_diff_hunks(diff);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0], (1, 7));
        assert_eq!(ranges[1], (12, 16));
    }

    #[test]
    fn test_ranges_overlap() {
        // Overlapping
        assert!(ranges_overlap(1, 10, 5, 15));
        assert!(ranges_overlap(5, 15, 1, 10));

        // Touching
        assert!(ranges_overlap(1, 5, 5, 10));

        // Not overlapping
        assert!(!ranges_overlap(1, 5, 6, 10));
        assert!(!ranges_overlap(6, 10, 1, 5));
    }

    #[test]
    fn test_context_reason_description() {
        let changed = ContextReason::Changed(ChangeType::Modified);
        assert!(changed.description().contains("modified"));

        let calls = ContextReason::CallsChanged {
            symbol: "foo".to_string(),
            depth: 2,
        };
        assert!(calls.description().contains("foo"));
        assert!(calls.description().contains("2"));
    }

    #[test]
    fn test_select_by_token_budget() {
        let files = vec![
            ContextFile {
                path: "a.rs".to_string(),
                priority: 1.0,
                reason: ContextReason::Changed(ChangeType::Modified),
                token_count: 100,
            },
            ContextFile {
                path: "b.rs".to_string(),
                priority: 0.8,
                reason: ContextReason::Changed(ChangeType::Added),
                token_count: 200,
            },
            ContextFile {
                path: "c.rs".to_string(),
                priority: 0.6,
                reason: ContextReason::Changed(ChangeType::Modified),
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
    }

    #[test]
    fn test_context_files_sort_by_priority_then_path() {
        let mut files = vec![
            context_file("low.rs", 0.6, 1),
            context_file("z_high.rs", 1.0, 1),
            context_file("middle.rs", 0.8, 1),
            context_file("a_high.rs", 1.0, 1),
        ];

        sort_context_files(&mut files);

        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["a_high.rs", "z_high.rs", "middle.rs", "low.rs"]);
    }

    #[test]
    fn test_context_budget_selection_is_permutation_invariant() {
        let files = [
            context_file("a_large.rs", 1.0, 200),
            context_file("b_small.rs", 1.0, 100),
            context_file("c_small.rs", 1.0, 100),
        ];
        let permutations = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];

        for permutation in permutations {
            let mut permuted: Vec<ContextFile> = permutation
                .into_iter()
                .map(|index| files[index].clone())
                .collect();
            sort_context_files(&mut permuted);

            let (selected, total, omitted) = select_by_token_budget(permuted, 200);
            let paths: Vec<&str> = selected.iter().map(|file| file.path.as_str()).collect();
            assert_eq!(paths, ["a_large.rs"]);
            assert_eq!(total, 200);
            assert_eq!(omitted, 2);
        }
    }

    fn context_file(path: &str, priority: f32, token_count: usize) -> ContextFile {
        ContextFile {
            path: path.to_string(),
            priority,
            reason: ContextReason::Changed(ChangeType::Modified),
            token_count,
        }
    }

    #[test]
    fn test_diff_config_default() {
        let config = DiffConfig::default();
        assert_eq!(config.max_tokens, 8000);
        assert_eq!(config.depth, 1);
        assert!(!config.changes_only);
        assert!(!config.staged);
        assert!(!config.summary);
    }
}
