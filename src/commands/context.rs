//! Context generation commands.
//!
//! Handles the default context generation command and related functionality.

use std::env;
use std::path::Path;
use std::time::Instant;

use crate::cli::Args;
use crate::commands::format_token_count;
use ctx::error::Result;
use ctx::output::{generate_context, stream_context};
use ctx::tokens;
use ctx::walker::{self, discover_files, FileEntry, WalkerConfig};

/// Parse a CLI tokenizer encoding with the canonical user-facing error.
pub fn parse_encoding(value: &str) -> Result<tokens::Encoding> {
    std::str::FromStr::from_str(value).ok().ok_or_else(|| {
        format!(
            "Invalid encoding '{}'. Valid options: cl100k_base, o200k_base, p50k_base",
            value
        )
        .into()
    })
}

/// Run the default context generation command.
pub fn run_context(args: Args) -> Result<()> {
    let start = Instant::now();

    // Determine root directory
    let root = env::current_dir()?;

    // Build walker configuration
    let config = WalkerConfig {
        use_gitignore: !args.no_gitignore,
        use_default_ignores: !args.no_default_ignores,
        custom_ignores: args.ignore_patterns,
        include_patterns: args.patterns,
    };

    // Discover files
    let entries = discover_files(&root, &config)?;

    if entries.is_empty() {
        eprintln!("No files found matching the specified patterns.");
        return Ok(());
    }

    // Parse encoding
    let encoding = parse_encoding(&args.encoding)?;

    // Handle --count-only mode: just count tokens without output
    if args.count_only {
        return run_count_only(&root, &entries, encoding, args.stats, start);
    }

    // Handle --max-tokens mode: filter files to fit within budget
    let entries = if let Some(max_tokens) = args.max_tokens {
        filter_files_by_tokens(&root, &entries, max_tokens, encoding)?
    } else {
        entries
    };

    if entries.is_empty() {
        eprintln!("No files fit within the token budget.");
        return Ok(());
    }

    // Generate context (streaming by default, buffered with --no-stream)
    let result = if args.no_stream {
        let result = generate_context(
            &root,
            &entries,
            args.format.to_lib(),
            !args.no_tree,
            args.show_sizes,
        )?;
        // Output to stdout (only in buffered mode)
        println!("{}", result.content);
        result
    } else {
        stream_context(
            &root,
            &entries,
            args.format.to_lib(),
            !args.no_tree,
            args.show_sizes,
        )?
    };

    // Print stats to stderr (only if --stats flag is passed)
    if args.stats {
        let elapsed = start.elapsed();
        let approx_tokens = result.output_bytes.div_ceil(4);
        eprintln!(
            "Generated context: {} files, {}, ~{} tokens in {:.2?}",
            result.file_count,
            walker::format_size(result.total_size),
            format_token_count(approx_tokens),
            elapsed
        );
    }

    Ok(())
}

/// Run --count-only mode: count tokens in files without generating output.
pub fn run_count_only(
    root: &Path,
    entries: &[FileEntry],
    encoding: tokens::Encoding,
    show_stats: bool,
    start: Instant,
) -> Result<()> {
    let mut total_tokens = 0usize;
    let mut total_chars = 0usize;
    let mut file_count = 0usize;
    let mut skipped_count = 0usize;

    for entry in entries {
        let path = root.join(&entry.relative_path);
        // Use lossy read to match read_file_content behavior in output.rs
        match std::fs::read(&path) {
            Ok(bytes) => {
                let content = String::from_utf8_lossy(&bytes);
                let token_count = tokens::count_tokens_with_encoding(&content, encoding)?;
                total_tokens += token_count;
                total_chars += content.chars().count(); // Use char count, not byte length
                file_count += 1;
            }
            Err(e) => {
                eprintln!(
                    "Warning: could not read {}: {}",
                    entry.relative_path.display(),
                    e
                );
                skipped_count += 1;
            }
        }
    }

    // Output token count summary
    println!("Files: {}", file_count);
    if skipped_count > 0 {
        println!("Skipped (unreadable): {}", skipped_count);
    }
    println!("Characters (UTF-8): {}", total_chars);
    println!("Tokens ({}): {}", encoding.as_str(), total_tokens);

    if show_stats {
        let elapsed = start.elapsed();
        eprintln!("Counted in {:.2?}", elapsed);
    }

    Ok(())
}

/// Filter files to fit within a token budget.
pub fn filter_files_by_tokens(
    root: &Path,
    entries: &[FileEntry],
    max_tokens: usize,
    encoding: tokens::Encoding,
) -> Result<Vec<FileEntry>> {
    // Count tokens for each file
    let mut file_tokens: Vec<(usize, &FileEntry)> = Vec::new();

    for entry in entries {
        let path = root.join(&entry.relative_path);
        // Use lossy read to match read_file_content behavior in output.rs
        if let Ok(bytes) = std::fs::read(&path) {
            let content = String::from_utf8_lossy(&bytes);
            let token_count = tokens::count_tokens_with_encoding(&content, encoding)?;
            file_tokens.push((token_count, entry));
        }
    }

    // Select files that fit within budget (greedy, in order)
    let mut selected = Vec::new();
    let mut total = 0usize;
    let mut omitted = 0usize;

    for (tokens, entry) in file_tokens {
        if total + tokens <= max_tokens {
            total += tokens;
            selected.push(entry.clone());
        } else {
            omitted += 1;
        }
    }

    if omitted > 0 {
        eprintln!(
            "Token budget: {} files included ({} tokens), {} files omitted",
            selected.len(),
            total,
            omitted
        );
    }

    Ok(selected)
}
