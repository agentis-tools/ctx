mod cli;
mod default_ignores;
mod formatter;
mod output;
mod tree;
mod walker;

use std::env;
use std::process;
use std::time::Instant;

use clap::Parser;

use cli::Args;
use output::{generate_context, stream_context};
use walker::{discover_files, WalkerConfig};

fn main() {
    let args = Args::parse();

    if let Err(e) = run(args) {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
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

    // Generate context (streaming by default, buffered with --no-stream)
    let result = if args.no_stream {
        let result = generate_context(
            &root,
            &entries,
            &args.format,
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
            &args.format,
            !args.no_tree,
            args.show_sizes,
        )?
    };

    // Print stats to stderr (only if --stats flag is passed)
    if args.stats {
        let elapsed = start.elapsed();
        eprintln!(
            "Generated context: {} files, {} in {:.2?}",
            result.file_count,
            walker::format_size(result.total_size),
            elapsed
        );
    }

    Ok(())
}
