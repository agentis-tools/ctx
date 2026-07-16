//! Smart context selection command.
//!
//! Handles AI-powered intelligent file selection for context generation.

use std::env;
use std::time::Instant;

use crate::cli::OutputFormat;
use crate::commands::format_token_count;
use ctx::analytics;
use ctx::embeddings::{self, Provider};
use ctx::error::Result;
use ctx::index;
use ctx::output;
use ctx::smart::{format_dry_run, format_explain, smart_context_filtered, SmartConfig};
use ctx::walker;

/// Run smart context selection.
#[allow(clippy::too_many_arguments)]
pub fn run_smart(
    task: &str,
    max_tokens: usize,
    depth: i32,
    top: usize,
    explain: bool,
    dry_run: bool,
    provider: Provider,
    format: OutputFormat,
    show_sizes: bool,
    no_tree: bool,
    patterns: &[String],
    count_only: bool,
    encoding: &str,
    stats: bool,
) -> Result<()> {
    let start = Instant::now();
    let encoding = super::context::parse_encoding(encoding)?;
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;
    let filter = walker::FilePatternFilter::new(&root, patterns)
        .map_err(|error| ctx::error::CtxError::Other(format!("Invalid file pattern: {error}")))?;

    // Check if we have embeddings
    let embedding_count = db.count_embeddings()?;
    if embedding_count == 0 {
        eprintln!("No embeddings found. Run 'ctx embed' first to generate embeddings.");
        return Ok(());
    }

    if provider == Provider::Local {
        eprintln!("Initializing local embedding model (first run downloads ~90MB)...");
    }
    let provider =
        embeddings::build_provider(provider, &ctx::config::CtxConfig::load(&root).embedding)?;

    // Warn if the query provider/dimension differs from the index.
    embeddings::warn_index_mismatch(&db, provider.as_ref());

    // Open analytics for call graph expansion
    let analytics = analytics::Analytics::open(&root)?;

    // Configure and run smart context selection
    // For dry-run, don't limit tokens - show all relevant files
    let effective_max_tokens = if dry_run && !count_only {
        usize::MAX
    } else {
        max_tokens
    };
    let config = SmartConfig {
        max_tokens: effective_max_tokens,
        depth,
        top,
        encoding,
    };

    eprintln!("Analyzing task: \"{}\"...", task);

    let result = smart_context_filtered(&db, &analytics, provider.as_ref(), task, config, &filter)?;

    if result.selected_files.is_empty() {
        eprintln!("No relevant files found for: \"{}\"", task);
        std::process::exit(2);
    }

    eprintln!(
        "Selected {} files ({} tokens){}",
        result.selected_files.len(),
        result.total_tokens,
        if result.truncated {
            format!(", {} omitted", result.omitted_count)
        } else {
            String::new()
        }
    );

    // If the single most-relevant file alone exceeds the budget it is included
    // anyway (rather than silently dropped); tell the user why little else fits.
    if let Some(top) = result.selected_files.first() {
        if top.token_count > max_tokens {
            eprintln!(
                "note: {} ({} tokens) exceeds the {}-token budget; included alone \
                 — raise --max-tokens to include more",
                top.path, top.token_count, max_tokens
            );
        }
    }

    // Convert selected files to FileEntry format for context generation
    let entries: Vec<walker::FileEntry> = result
        .selected_files
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

    if count_only {
        return super::context::run_count_only(&root, &entries, encoding, stats, start);
    }

    // Handle dry-run mode
    if dry_run {
        println!("{}", format_dry_run(&result));
        return Ok(());
    }

    // Handle explain mode (show reasoning then context)
    if explain {
        eprintln!("{}", format_explain(&result));
    }

    // Generate context output
    let output_result = if entries.is_empty() {
        eprintln!("No files to include in context.");
        return Ok(());
    } else {
        output::stream_context(&root, &entries, format.to_lib(), !no_tree, show_sizes)?
    };

    eprintln!(
        "Generated context: {} files, ~{} tokens",
        output_result.file_count,
        format_token_count(output_result.output_bytes.div_ceil(4))
    );

    Ok(())
}
