//! Smart context selection command.
//!
//! Handles AI-powered intelligent file selection for context generation.

use std::env;

use crate::cli::OutputFormat;
use crate::commands::format_token_count;
use ctx::analytics;
use ctx::embeddings::EmbeddingProvider;
use ctx::error::Result;
use ctx::index;
use ctx::output;
use ctx::smart::{format_dry_run, format_explain, smart_context, SmartConfig};
use ctx::tokens;
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
    use_openai: bool,
    format: OutputFormat,
    show_sizes: bool,
    no_tree: bool,
) -> Result<()> {
    use ctx::embeddings;

    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Check if we have embeddings
    let embedding_count = db.count_embeddings()?;
    if embedding_count == 0 {
        eprintln!("No embeddings found. Run 'ctx embed' first to generate embeddings.");
        return Ok(());
    }

    // Create embedding provider
    let provider: Box<dyn EmbeddingProvider> = if use_openai {
        use embeddings::openai::OpenAIProvider;
        let p = OpenAIProvider::from_env().map_err(|_| {
            "OPENAI_API_KEY environment variable not set.\n\
             Set it with: export OPENAI_API_KEY=sk-..."
        })?;
        Box::new(p)
    } else {
        use embeddings::local::LocalProvider;
        let p = LocalProvider::new()?;
        Box::new(p)
    };

    // Check for embedding dimension mismatch
    let query_dim = provider.dimension();
    if let Ok(metadata) = db.get_embedding_metadata() {
        for (stored_provider, _model, stored_dim, count) in &metadata {
            let stored_dim = *stored_dim as usize;
            if stored_dim != query_dim {
                eprintln!("Warning: Embedding dimension mismatch detected!");
                eprintln!(
                    "  Stored: {} embeddings from '{}' with dimension {}",
                    count, stored_provider, stored_dim
                );
                eprintln!(
                    "  Query:  Using '{}' with dimension {}",
                    provider.name(),
                    query_dim
                );
                eprintln!(
                    "  Results may be inaccurate. Re-run 'ctx embed{}' to regenerate embeddings.",
                    if use_openai { " --openai" } else { "" }
                );
                eprintln!();
            }
        }
    }

    // Open analytics for call graph expansion
    let analytics = analytics::Analytics::open(&root)?;

    // Configure and run smart context selection
    // For dry-run, don't limit tokens - show all relevant files
    let effective_max_tokens = if dry_run { usize::MAX } else { max_tokens };
    let config = SmartConfig {
        max_tokens: effective_max_tokens,
        depth,
        top,
        encoding: tokens::Encoding::default(),
    };

    eprintln!("Analyzing task: \"{}\"...", task);

    let result = smart_context(&db, &analytics, provider.as_ref(), task, config)?;

    if result.selected_files.is_empty() {
        eprintln!("No relevant files found for: \"{}\"", task);
        std::process::exit(2);
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
