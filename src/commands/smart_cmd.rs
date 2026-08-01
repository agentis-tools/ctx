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
use ctx::smart::{
    format_dry_run, format_explain, smart_context_filtered_with_options, SmartConfig,
};
use ctx::walker;

/// Run smart context selection.
#[allow(clippy::too_many_arguments)]
pub fn run_smart(
    task: &str,
    max_tokens: usize,
    include_oversized_top: bool,
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
    if max_tokens == 0 {
        return Err(ctx::error::CtxError::Other(
            "--max-tokens must be greater than zero".to_string(),
        ));
    }

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

    // Count-only has always retained the highest-ranked file even when it is
    // larger than the requested content budget. Keep that compatibility while
    // normal smart output defaults to a strict budget.
    let result = smart_context_filtered_with_options(
        &db,
        &analytics,
        provider.as_ref(),
        task,
        config,
        &filter,
        include_oversized_top || count_only,
    )?;

    if result.selected_files.is_empty() {
        if result.truncated {
            return Err(ctx::error::CtxError::Other(format!(
                "no selected file fits within --max-tokens {}; raise the budget or use --include-oversized-top",
                max_tokens
            )));
        }
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

    // Render before writing so --max-tokens is a hard limit on the complete
    // stdout document, including wrappers and the optional project tree.
    let (output_result, rendered_omitted) = if entries.is_empty() {
        eprintln!("No files to include in context.");
        return Ok(());
    } else {
        fit_rendered_budget(
            &root,
            &entries,
            format.to_lib(),
            !no_tree,
            show_sizes,
            max_tokens,
            encoding,
            include_oversized_top,
        )?
    };

    if rendered_omitted > 0 {
        eprintln!(
            "Rendered token budget: {} file{} omitted to keep stdout within {} tokens",
            rendered_omitted,
            if rendered_omitted == 1 { "" } else { "s" },
            max_tokens
        );
    }
    print!("{}", output_result.content);

    eprintln!(
        "Generated context: {} files, ~{} tokens",
        output_result.file_count,
        format_token_count(output_result.output_bytes.div_ceil(4))
    );

    Ok(())
}

/// Render candidates incrementally and keep only packs whose complete output
/// fits the requested tokenizer budget. Streaming cannot know whether a later
/// tree or closing block would cross the hard limit before bytes are written.
fn fit_rendered_budget(
    root: &std::path::Path,
    entries: &[walker::FileEntry],
    format: ctx::formatter::OutputFormat,
    include_tree: bool,
    show_sizes: bool,
    max_tokens: usize,
    encoding: ctx::tokens::Encoding,
    include_oversized_top: bool,
) -> Result<(ctx::output::ContextResult, usize)> {
    if include_oversized_top {
        let rendered =
            output::render_stream_context(root, entries, format, include_tree, show_sizes)?;
        return Ok((rendered, 0));
    }

    let mut selected = Vec::new();
    let mut omitted = 0;
    for entry in entries {
        let mut candidate = selected.clone();
        candidate.push(entry.clone());
        let rendered =
            output::render_stream_context(root, &candidate, format, include_tree, show_sizes)?;
        let tokens = ctx::tokens::count_tokens_with_encoding(&rendered.content, encoding)
            .map_err(ctx::error::CtxError::token_count)?;
        if tokens <= max_tokens {
            selected.push(entry.clone());
        } else {
            omitted += 1;
        }
    }

    if selected.is_empty() {
        return Err(ctx::error::CtxError::token_count(format!(
            "rendered context framing exceeds --max-tokens {}; raise the budget or use --include-oversized-top",
            max_tokens
        )));
    }

    let rendered =
        output::render_stream_context(root, &selected, format, include_tree, show_sizes)?;
    Ok((rendered, omitted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn entry(root: &std::path::Path, path: &str) -> walker::FileEntry {
        let absolute_path = root.join(path);
        walker::FileEntry {
            size: std::fs::metadata(&absolute_path).unwrap().len(),
            absolute_path,
            relative_path: PathBuf::from(path),
        }
    }

    #[test]
    fn rendered_budget_drops_files_to_fit_wrappers() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn a() { let value = 1; }\n").unwrap();
        std::fs::write(temp.path().join("b.rs"), "fn b() { let value = 2; }\n").unwrap();
        let entries = vec![entry(temp.path(), "a.rs"), entry(temp.path(), "b.rs")];
        let one = output::render_stream_context(
            temp.path(),
            &entries[..1],
            ctx::formatter::OutputFormat::Plain,
            true,
            false,
        )
        .unwrap();
        let one_tokens =
            ctx::tokens::count_tokens_with_encoding(&one.content, ctx::tokens::Encoding::default())
                .unwrap();
        let all = output::render_stream_context(
            temp.path(),
            &entries,
            ctx::formatter::OutputFormat::Plain,
            true,
            false,
        )
        .unwrap();
        let all_tokens =
            ctx::tokens::count_tokens_with_encoding(&all.content, ctx::tokens::Encoding::default())
                .unwrap();

        let (rendered, omitted) = fit_rendered_budget(
            temp.path(),
            &entries,
            ctx::formatter::OutputFormat::Plain,
            true,
            false,
            one_tokens,
            ctx::tokens::Encoding::default(),
            false,
        )
        .unwrap();
        assert_eq!(omitted, 1);
        assert!(!rendered.content.contains("b.rs"));
        assert!(all_tokens > one_tokens);
        assert!(
            ctx::tokens::count_tokens_with_encoding(
                &rendered.content,
                ctx::tokens::Encoding::default()
            )
            .unwrap()
                <= one_tokens
        );
    }
}
