// CLI-specific modules
mod cli;
mod commands;
mod shell;

use std::process::ExitCode;

use clap::Parser;

use cli::{Args, Command};
use ctx::error::Result;
use ctx::exit::Outcome;

/// Exit codes: 0 = clean, 1 = findings, 2 = operational error.
fn main() -> ExitCode {
    // The OS-provided main thread stack is too small on some platforms (notably
    // Windows, which defaults to ~1 MiB) for this program's parsing/graph-walking
    // call depth; run on a thread with a larger, explicit stack instead.
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(run_main)
        .expect("failed to spawn main worker thread")
        .join()
        .expect("main worker thread panicked")
}

fn run_main() -> ExitCode {
    // Same rationale as the main thread above: give rayon's global pool (used by
    // `ctx index --parallel`) an explicit stack size instead of the platform default.
    let _ = rayon::ThreadPoolBuilder::new()
        .stack_size(16 * 1024 * 1024)
        .build_global();

    let args = Args::parse();

    match run(args) {
        Ok(Outcome::Clean) => ExitCode::SUCCESS,
        Ok(Outcome::Findings) => ExitCode::from(1),
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::from(2)
        }
    }
}

fn run(args: Args) -> Result<Outcome> {
    // Global machine-readable output flag (see docs/json-output.md)
    let json = args.json;

    // Handle subcommands
    let result: Result<()> = match args.command {
        Some(Command::Index {
            watch,
            verbose,
            force,
            parallel,
            no_gitignore,
            no_default_ignores,
            ignore_patterns,
            include_patterns,
        }) => {
            let config = commands::IndexConfig::new(
                watch,
                verbose,
                force,
                parallel,
                no_gitignore,
                no_default_ignores,
                ignore_patterns,
                include_patterns,
            );
            commands::run_index(config)
        }
        Some(Command::Query { query }) => commands::run_query(query, json),
        Some(Command::Search {
            query,
            limit,
            output,
        }) => {
            let output = if json { "json".to_string() } else { output };
            commands::run_search(&query, limit, &output)
        }
        Some(Command::Source { symbol, file, kind }) => {
            commands::run_source(&symbol, file.as_deref(), kind.as_deref())
        }
        Some(Command::Explain { symbol, file, kind }) => {
            commands::run_explain(&symbol, file.as_deref(), kind.as_deref(), json)
        }
        Some(Command::Embed {
            force,
            verbose,
            batch_size,
            openai,
            watch,
        }) => {
            if watch {
                commands::run_embed_watch(verbose, batch_size, openai)
            } else {
                commands::run_embed(force, verbose, batch_size, openai)
            }
        }
        Some(Command::Semantic {
            query,
            limit,
            output,
            openai,
        }) => {
            let output = if json { "json".to_string() } else { output };
            commands::run_semantic(&query, limit, &output, openai)
        }
        Some(Command::Complexity {
            threshold,
            warnings_only,
            output,
        }) => commands::run_complexity(threshold, warnings_only, &output),
        Some(Command::Duplicates {
            similarity,
            min_lines,
            output,
        }) => commands::run_duplicates(similarity, min_lines, &output),
        Some(Command::Graph {
            output,
            by_file,
            filter,
            depth,
        }) => commands::run_graph(&output, by_file, filter, depth),
        Some(Command::Smart {
            task,
            max_tokens,
            depth,
            top,
            explain,
            dry_run,
            openai,
            format,
            show_sizes,
            no_tree,
        }) => commands::run_smart(
            &task, max_tokens, depth, top, explain, dry_run, openai, format, show_sizes, no_tree,
        ),
        Some(Command::Diff {
            revision,
            max_tokens,
            depth,
            changes_only,
            staged,
            summary,
            format,
            show_sizes,
            no_tree,
        }) => commands::run_diff(
            &revision,
            max_tokens,
            depth,
            changes_only,
            staged,
            summary,
            format,
            show_sizes,
            no_tree,
        ),
        Some(Command::Review {
            pr,
            repo,
            include_comments,
            max_tokens,
            depth,
            changes_only,
            summary,
            format,
            show_sizes,
            no_tree,
        }) => commands::run_review(
            &pr,
            repo.as_deref(),
            include_comments,
            max_tokens,
            depth,
            changes_only,
            summary,
            format,
            show_sizes,
            no_tree,
        ),
        Some(Command::Check {
            rules,
            against,
            list,
        }) => {
            // Quality command: returns Outcome natively (0 clean / 1 findings).
            return commands::run_check(rules, against, list, json);
        }
        Some(Command::Audit {
            output_format,
            min_score,
            categories,
            incremental,
        }) => commands::run_audit(&output_format, min_score, categories, incremental),
        Some(Command::Shell {
            history,
            no_history,
            vi,
        }) => commands::run_shell(history, no_history, vi),
        #[cfg(feature = "mcp")]
        Some(Command::Serve { mcp }) => commands::run_serve(mcp),
        None => commands::run_context(args),
    };

    // No command reports findings yet; quality commands built on top of this
    // convention will return Outcome::Findings to exit with code 1.
    result.map(|_| Outcome::Clean)
}
