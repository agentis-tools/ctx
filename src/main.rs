// CLI-specific modules
mod cli;
mod commands;
mod shell;

use std::process::ExitCode;

use clap::Parser;

use cli::{Args, Command, OutputFormat};
use commands::MapFormat;
use ctx::error::Result;
use ctx::exit::Outcome;

/// Resolve the embedding provider from the CLI flags and the project's
/// `.ctx/config.toml` default (flag > `--openai` > config > built-in default).
fn resolve_embed_provider(
    flag: Option<ctx::embeddings::Provider>,
    openai: bool,
) -> ctx::embeddings::Provider {
    let config_default = std::env::current_dir()
        .ok()
        .and_then(|cwd| ctx::config::CtxConfig::load(&cwd).embedding.provider);
    ctx::embeddings::Provider::resolve(flag, openai, config_default)
}

/// Exit codes: 0 = clean, 1 = findings, 2 = operational error,
/// 3 = version requirement not met (`ctx harness compat` only).
fn main() -> ExitCode {
    // Hidden test mode: when CTX_INTERNAL_MOCK_LSP points at a scenario file,
    // the binary acts as a scripted mock language server over stdio (used by
    // the LSP backend integration tests). Checked before clap parsing so no
    // CLI surface is involved.
    if let Some(path) = std::env::var_os("CTX_INTERNAL_MOCK_LSP") {
        ctx::lsp::mock::run_stdio_mock(std::path::Path::new(&path));
        return ExitCode::SUCCESS;
    }

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
    let json = args.json;
    // The passive update check never runs for update-related invocations
    // (`ctx self-update`, `ctx --version [--check]`); its remaining
    // suppression rules (TTY, --json, env vars, 24h cache) live in
    // `ctx::update::passive_check`.
    let skip_passive_check =
        args.version || matches!(args.command, Some(Command::SelfUpdate { .. }));

    let exit = match run(args) {
        Ok(outcome) => ExitCode::from(outcome.code()),
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::from(2)
        }
    };

    // Passive update notice (stderr only, at most one network call per 24h,
    // silent on failure; see docs/commands/self-update.md). Never automatic:
    // this only prints a notice, it never installs anything.
    if !skip_passive_check {
        ctx::update::passive_check(json);
    }

    exit
}

fn run(args: Args) -> Result<Outcome> {
    // Global machine-readable output flag (see docs/json-output.md)
    let json = args.json;
    let patterns = args.patterns.clone();

    // Custom --version handling: clap's auto flag is disabled (it would
    // exit before `--check` could run). `ctx --version` prints the same
    // line the auto flag did; `--check` adds a release comparison.
    if args.version {
        return commands::run_version(args.check, json);
    }

    // Handle subcommands
    let result: Result<()> = match args.command {
        Some(Command::Index {
            watch,
            verbose,
            force,
            // Deprecated no-op: parallel indexing is now the default.
            parallel: _,
            serial,
            no_gitignore,
            no_default_ignores,
            ignore_patterns,
            include_patterns,
        }) => {
            // The global positional patterns (`ctx index src`) scope the
            // index just like `-p`; a bare `.` is the unscoped default.
            let include_patterns =
                commands::merge_include_patterns(args.patterns, include_patterns);
            let config = commands::IndexConfig::new(
                watch,
                verbose,
                force,
                serial,
                no_gitignore,
                no_default_ignores,
                ignore_patterns,
                include_patterns,
            );
            commands::run_index(config)
        }
        Some(Command::Query { query }) => commands::run_query(query, json),
        Some(Command::Sql {
            query,
            file,
            output,
            json: json_flag,
            max_rows,
            timeout,
            fail_on_rows,
            schema,
            snapshots,
        }) => {
            // `ctx sql` owns its exit code (0 clean / 1 with --fail-on-rows / 2 error),
            // so it returns an Outcome directly like the other quality commands.
            return commands::run_sql(commands::SqlArgs {
                query,
                file,
                format: output,
                json: json || json_flag,
                max_rows,
                timeout,
                fail_on_rows,
                schema,
                snapshots,
            });
        }
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
            provider,
            openai,
            watch,
            serial,
        }) => {
            let provider = resolve_embed_provider(provider, openai);
            if watch {
                commands::run_embed_watch(verbose, batch_size, provider, serial)
            } else {
                commands::run_embed(force, verbose, batch_size, provider, serial)
            }
        }
        Some(Command::Semantic {
            query,
            limit,
            output,
            provider,
            openai,
        }) => {
            let provider = resolve_embed_provider(provider, openai);
            let output = if json { "json".to_string() } else { output };
            commands::run_semantic(&query, limit, &output, provider)
        }
        Some(Command::Similar {
            query,
            limit,
            keyword,
            provider,
            openai,
        }) => {
            let provider = resolve_embed_provider(provider, openai);
            // `similar` participates in the Outcome convention directly:
            // Clean on success, Err (exit 2) when embeddings are missing.
            return commands::run_similar(&query, limit, keyword, provider, json, &patterns);
        }
        Some(Command::Complexity {
            threshold,
            warnings_only,
            output,
        }) => commands::run_complexity(threshold, warnings_only, &output),
        Some(Command::Duplicates {
            threshold,
            min_tokens,
            against,
            fail_on_found,
        }) => {
            // Quality command: returns its own Outcome (Findings with
            // --fail-on-found when pairs are reported).
            return commands::run_duplicates(
                threshold,
                min_tokens,
                against.as_deref(),
                json,
                fail_on_found,
            );
        }
        Some(Command::Map {
            budget,
            focus,
            format,
        }) => {
            // The global --json flag forces JSON format for consistency.
            let format = if json {
                Ok(MapFormat::Json)
            } else {
                match format {
                    OutputFormat::Text => Ok(MapFormat::Text),
                    OutputFormat::Markdown | OutputFormat::Md => Ok(MapFormat::Markdown),
                    OutputFormat::Json => Ok(MapFormat::Json),
                    OutputFormat::Xml | OutputFormat::Plain => Err(ctx::error::CtxError::Other(
                        "ctx map supports --format text, markdown, or json".to_string(),
                    )),
                }
            };
            format.and_then(|format| commands::run_map(budget, focus.as_deref(), format))
        }
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
            provider,
            openai,
            format,
            show_sizes,
            no_tree,
        }) => {
            let provider = resolve_embed_provider(provider, openai);
            commands::run_smart(
                &task, max_tokens, depth, top, explain, dry_run, provider, format, show_sizes,
                no_tree, &patterns,
            )
        }
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
            &patterns,
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
        Some(Command::Hotspots {
            since,
            limit,
            by,
            min_churn,
            against,
        }) => commands::run_hotspots(&since, limit, by, min_churn, against.as_deref(), json),
        Some(Command::Check {
            rules,
            against,
            list,
        }) => {
            // Quality command: returns Outcome natively (0 clean / 1 findings).
            return commands::run_check(rules, against, list, json);
        }
        Some(Command::Score { against, fail_on }) => {
            // Quality command: returns Outcome natively (0 clean / 1 when a
            // --fail-on condition is met).
            return commands::run_score(&against, fail_on.as_deref(), json);
        }
        Some(Command::Audit {
            output_format,
            min_score,
            categories,
            incremental,
        }) => commands::run_audit(&output_format, min_score, categories, incremental),
        Some(Command::Snapshot {
            cmd,
            force,
            churn_window,
        }) => {
            // Snapshot command: returns its own Outcome (always Clean on
            // success; stub builds and git/IO failures map to exit 2).
            return commands::run_snapshot(cmd, force, &churn_window, json);
        }
        Some(Command::Harness { cmd }) => {
            // Harness command: returns its own Outcome (doctor exits 1 on
            // problems; compat exits 3 on version mismatch).
            return commands::run_harness(cmd, json);
        }
        Some(Command::Lsp { cmd }) => {
            // LSP registry command: returns its own Outcome (doctor exits 1
            // when a configured server fails its health probe).
            return commands::run_lsp(cmd, json);
        }
        Some(Command::SelfUpdate { version }) => {
            // Update command: returns its own Outcome (Clean when updated or
            // already up to date; any failure maps to exit 2 in main).
            return commands::run_self_update(version.as_deref(), json);
        }
        Some(Command::Shell {
            history,
            no_history,
            vi,
        }) => commands::run_shell(history, no_history, vi),
        #[cfg(feature = "mcp")]
        Some(Command::Serve { mcp }) => commands::run_serve(mcp),
        None => commands::run_context(args),
    };

    // Commands routed through this fallthrough never report findings;
    // quality commands (e.g. `duplicates`) return early with their own
    // Outcome above.
    result.map(|_| Outcome::Clean)
}
