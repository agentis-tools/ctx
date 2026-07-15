//! Diff-aware context generation commands.
//!
//! Handles git diff analysis and PR review context generation.

use std::env;

use crate::cli::OutputFormat;
use crate::commands::format_token_count;
use ctx::analytics;
use ctx::diff::{
    self, diff_context_filtered, format_pr_header, format_summary, get_pr_info, DiffConfig,
};
use ctx::error::{CtxError, Result};
use ctx::index;
use ctx::output;
use ctx::tokens;
use ctx::walker;

/// Run diff-aware context generation.
#[allow(clippy::too_many_arguments)]
pub fn run_diff(
    revision: &str,
    max_tokens: usize,
    depth: i32,
    changes_only: bool,
    staged: bool,
    summary: bool,
    format: OutputFormat,
    show_sizes: bool,
    no_tree: bool,
    patterns: &[String],
) -> Result<()> {
    let root = env::current_dir()?;
    let filter = walker::FilePatternFilter::new(&root, patterns)
        .map_err(|error| CtxError::Other(format!("Invalid file pattern: {error}")))?;

    // Check if index exists (for context expansion)
    let db = match index::open_database(&root) {
        Ok(db) => Some(db),
        Err(_) => {
            if !changes_only {
                eprintln!("Warning: No index found. Run 'ctx index' for context expansion.");
                eprintln!("Using --changes-only mode.\n");
            }
            None
        }
    };

    // Open analytics if we have a database
    let analytics = if db.is_some() {
        analytics::Analytics::open(&root).ok()
    } else {
        None
    };

    // Configure diff context
    let config = DiffConfig {
        max_tokens,
        depth,
        changes_only: changes_only || analytics.is_none(),
        staged,
        summary,
        encoding: tokens::Encoding::default(),
    };

    let revision_display = if staged { "staged changes" } else { revision };
    eprintln!("Analyzing {}...", revision_display);

    // Run diff context analysis
    let result = match (&db, &analytics) {
        (Some(db), Some(analytics)) => {
            diff_context_filtered(revision, db, analytics, config, &filter)
        }
        _ => {
            // Fallback: just get changed files without context expansion
            let changed = diff::get_changed_files_filtered(revision, staged, &filter)?;
            Ok(diff::DiffContext {
                revision: revision.to_string(),
                changed_files: changed.clone(),
                affected_symbols: Vec::new(),
                context_files: changed
                    .iter()
                    .filter(|f| f.change_type != diff::ChangeType::Deleted)
                    .map(|f| diff::ContextFile {
                        path: f.path.clone(),
                        priority: 1.0,
                        reason: diff::ContextReason::Changed(f.change_type),
                        token_count: 0,
                    })
                    .collect(),
                total_tokens: 0,
                truncated: false,
                omitted_count: 0,
            })
        }
    };

    let result = match result {
        Ok(r) => r,
        Err(CtxError::NoChanges) => {
            eprintln!("No changes found.");
            std::process::exit(2);
        }
        Err(CtxError::NotGitRepo) => {
            eprintln!("Error: Not a git repository.");
            std::process::exit(1);
        }
        Err(CtxError::InvalidRevision(r)) => {
            eprintln!("Error: Invalid revision '{}'", r);
            std::process::exit(1);
        }
        Err(e) => return Err(e),
    };

    // The revision had changes -- `get_changed_files` rejects one that did not --
    // so an empty list here means the patterns excluded every one of them. That
    // answers the narrower question rather than failing it, but warn, because in
    // practice it usually means a mistyped pattern.
    if result.changed_files.is_empty() {
        eprintln!("Warning: no changed files match the requested scope.");
    }

    // Show summary if requested
    if summary {
        eprintln!("{}", format_summary(&result));
    }

    eprintln!(
        "Changed {} files, context {} files ({} tokens){}",
        result.changed_files.len(),
        result.context_files.len(),
        result.total_tokens,
        if result.truncated {
            format!(", {} omitted", result.omitted_count)
        } else {
            String::new()
        }
    );

    // Convert to FileEntry for output
    let entries: Vec<walker::FileEntry> = result
        .context_files
        .iter()
        .map(|f| {
            let relative_path = std::path::PathBuf::from(&f.path);
            let absolute_path = root.join(&relative_path);
            let size = std::fs::metadata(&absolute_path)
                .map(|m| m.len())
                .unwrap_or(0);
            walker::FileEntry {
                absolute_path,
                relative_path,
                size,
            }
        })
        .collect();

    if entries.is_empty() {
        eprintln!("No files to include in context.");
        return Ok(());
    }

    // Generate context output
    let output_result =
        output::stream_context(&root, &entries, format.to_lib(), !no_tree, show_sizes)?;
    eprintln!(
        "Generated context: {} files, ~{} tokens",
        output_result.file_count,
        format_token_count(output_result.output_bytes.div_ceil(4))
    );

    Ok(())
}

/// Run PR review context generation.
#[allow(clippy::too_many_arguments)]
pub fn run_review(
    pr: &str,
    repo: Option<&str>,
    include_comments: bool,
    max_tokens: usize,
    depth: i32,
    changes_only: bool,
    summary: bool,
    format: OutputFormat,
    show_sizes: bool,
    no_tree: bool,
) -> Result<()> {
    eprintln!("Fetching PR #{}...", pr);

    // Get PR info from GitHub
    let pr_info = match get_pr_info(pr, repo) {
        Ok(info) => info,
        Err(CtxError::InvalidRevision(r)) => {
            eprintln!("Error: {}", r);
            std::process::exit(3);
        }
        Err(CtxError::Git(e)) if e.contains("not found") => {
            eprintln!("Error: GitHub CLI (gh) not found.");
            eprintln!("Install it from https://cli.github.com/");
            std::process::exit(1);
        }
        Err(e) => return Err(e),
    };

    // Print PR header
    eprintln!("{}", format_pr_header(&pr_info, include_comments));

    // Get the diff for the PR's changes
    // We use the base..head format to get the PR diff
    let revision = format!("{}...{}", pr_info.base, pr_info.head);

    // Run diff with the PR revision
    run_diff(
        &revision,
        max_tokens,
        depth,
        changes_only,
        false, // not staged
        summary,
        format,
        show_sizes,
        no_tree,
        &[".".to_string()],
    )
}
