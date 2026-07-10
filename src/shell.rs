//! Interactive shell for exploring codebases.
//!
//! Provides a REPL interface with command history and tab completion
//! for querying the code intelligence database.

use std::borrow::Cow;
use std::path::PathBuf;

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Config, EditMode, Editor, Helper};

use ctx::analytics::Analytics;
use ctx::db::Database;
use ctx::error::Result;

/// Shell configuration.
#[derive(Debug, Clone)]
pub struct ShellConfig {
    /// Path to history file
    pub history_file: PathBuf,
    /// Disable history
    pub no_history: bool,
    /// Use vi editing mode
    pub vi_mode: bool,
    /// Database path
    pub db_path: PathBuf,
}

impl Default for ShellConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            history_file: home.join(".ctx_history"),
            no_history: false,
            vi_mode: false,
            db_path: PathBuf::from("."),
        }
    }
}

/// Shell helper for completion and hints.
struct ShellHelper {
    commands: Vec<String>,
}

impl ShellHelper {
    fn new() -> Self {
        Self {
            commands: vec![
                "help".to_string(),
                "exit".to_string(),
                "quit".to_string(),
                "find".to_string(),
                "search".to_string(),
                "source".to_string(),
                "explain".to_string(),
                "callers".to_string(),
                "callees".to_string(),
                "deps".to_string(),
                "impact".to_string(),
                "complexity".to_string(),
                "smart".to_string(),
                "audit".to_string(),
                "stats".to_string(),
                "cd".to_string(),
                "pwd".to_string(),
                "history".to_string(),
                "clear".to_string(),
            ],
        }
    }
}

impl Completer for ShellHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        // Simple command completion
        let line_up_to_pos = &line[..pos];
        let words: Vec<&str> = line_up_to_pos.split_whitespace().collect();

        if words.is_empty() || (words.len() == 1 && !line_up_to_pos.ends_with(' ')) {
            // Complete command name
            let prefix = words.first().unwrap_or(&"");
            let start = line_up_to_pos
                .rfind(char::is_whitespace)
                .map(|i| i + 1)
                .unwrap_or(0);

            let matches: Vec<Pair> = self
                .commands
                .iter()
                .filter(|cmd| cmd.starts_with(prefix))
                .map(|cmd| Pair {
                    display: cmd.clone(),
                    replacement: cmd.clone(),
                })
                .collect();

            Ok((start, matches))
        } else {
            // No completion for arguments yet
            Ok((pos, vec![]))
        }
    }
}

impl Hinter for ShellHelper {
    type Hint = String;

    fn hint(&self, _line: &str, _pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        None
    }
}

impl Highlighter for ShellHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        Cow::Borrowed(line)
    }

    fn highlight_char(
        &self,
        _line: &str,
        _pos: usize,
        _kind: rustyline::highlight::CmdKind,
    ) -> bool {
        false
    }
}

impl Validator for ShellHelper {}

impl Helper for ShellHelper {}

/// Run the interactive shell.
pub fn run_shell(config: ShellConfig) -> Result<()> {
    // Open database
    let db = ctx::index::open_database(&config.db_path)?;

    // Open analytics (optional)
    let analytics = Analytics::open(&config.db_path).ok();

    // Configure rustyline
    let rl_config = Config::builder()
        .history_ignore_space(true)
        .edit_mode(if config.vi_mode {
            EditMode::Vi
        } else {
            EditMode::Emacs
        })
        .build();

    let mut rl = Editor::with_config(rl_config)?;
    rl.set_helper(Some(ShellHelper::new()));

    // Load history
    if !config.no_history {
        let _ = rl.load_history(&config.history_file);
    }

    // Current directory context (for filtering)
    let mut current_context: Option<String> = None;

    println!("ctx interactive shell - type 'help' for commands, 'exit' to quit");
    println!();

    loop {
        let prompt = if let Some(ref ctx) = current_context {
            format!("ctx ({})> ", ctx)
        } else {
            "ctx> ".to_string()
        };

        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(line);

                // Execute command
                match execute_command(line, &db, analytics.as_ref(), &mut current_context) {
                    Ok(should_exit) => {
                        if should_exit {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C: just continue
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                // Ctrl-D: exit
                println!();
                break;
            }
            Err(err) => {
                eprintln!("Error: {}", err);
                break;
            }
        }
    }

    // Save history
    if !config.no_history {
        let _ = rl.save_history(&config.history_file);
    }

    Ok(())
}

/// Execute a shell command. Returns true if shell should exit.
fn execute_command(
    line: &str,
    db: &Database,
    analytics: Option<&Analytics>,
    current_context: &mut Option<String>,
) -> Result<bool> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(false);
    }

    let cmd = parts[0].to_lowercase();
    let args = &parts[1..];

    match cmd.as_str() {
        // Built-in commands
        "help" | "?" => {
            print_help();
            Ok(false)
        }
        "exit" | "quit" | "q" => Ok(true),
        "cd" => {
            if args.is_empty() {
                *current_context = None;
                println!("Context cleared");
            } else {
                let path = args[0].trim_end_matches('/');
                *current_context = Some(path.to_string());
                println!("Context: {}/", path);
            }
            Ok(false)
        }
        "pwd" => {
            if let Some(ref ctx) = current_context {
                println!("{}/", ctx);
            } else {
                println!("(root)");
            }
            Ok(false)
        }
        "clear" => {
            // ANSI escape to clear screen
            print!("\x1B[2J\x1B[1;1H");
            Ok(false)
        }
        "history" => {
            println!("(history display not implemented - use arrow keys)");
            Ok(false)
        }

        // Query commands
        "find" => {
            if args.is_empty() {
                eprintln!("Usage: find <pattern> [--kind <kind>]");
            } else {
                cmd_find(db, args, current_context.as_deref())?;
            }
            Ok(false)
        }
        "search" => {
            if args.is_empty() {
                eprintln!("Usage: search <query>");
            } else {
                cmd_search(db, args)?;
            }
            Ok(false)
        }
        "source" => {
            if args.is_empty() {
                eprintln!("Usage: source <symbol>");
            } else {
                cmd_source(db, args, current_context.as_deref())?;
            }
            Ok(false)
        }
        "explain" => {
            if args.is_empty() {
                eprintln!("Usage: explain <symbol>");
            } else {
                cmd_explain(db, args, current_context.as_deref())?;
            }
            Ok(false)
        }
        "callers" => {
            if args.is_empty() {
                eprintln!("Usage: callers <function>");
            } else if let Some(a) = analytics {
                cmd_callers(db, a, args)?;
            } else {
                eprintln!("Analytics not available (run 'ctx index' first)");
            }
            Ok(false)
        }
        "callees" | "deps" => {
            if args.is_empty() {
                eprintln!("Usage: {} <function>", cmd);
            } else if let Some(a) = analytics {
                cmd_callees(db, a, args)?;
            } else {
                eprintln!("Analytics not available (run 'ctx index' first)");
            }
            Ok(false)
        }
        "impact" => {
            if args.is_empty() {
                eprintln!("Usage: impact <symbol>");
            } else if let Some(a) = analytics {
                cmd_impact(a, args)?;
            } else {
                eprintln!("Analytics not available");
            }
            Ok(false)
        }
        "complexity" => {
            if let Some(a) = analytics {
                cmd_complexity(a)?;
            } else {
                eprintln!("Analytics not available");
            }
            Ok(false)
        }
        "stats" => {
            cmd_stats(db)?;
            Ok(false)
        }
        "audit" => {
            cmd_audit(db, analytics)?;
            Ok(false)
        }

        _ => {
            eprintln!(
                "Unknown command: '{}'. Type 'help' for available commands.",
                cmd
            );
            Ok(false)
        }
    }
}

/// Print help message.
fn print_help() {
    println!(
        r#"
Available Commands:

  Built-in:
    help, ?          Show this help message
    exit, quit, q    Exit the shell
    cd <path>        Set file path context for filtering
    pwd              Show current context
    clear            Clear the screen
    history          Show command history

  Query:
    find <pattern>   Find symbols by name pattern
    search <query>   Hybrid search (text + semantic if available)
    source <symbol>  Show source code for a symbol
    explain <symbol> Explain a symbol with its relationships

  Analysis:
    callers <fn>     Show functions that call <fn>
    callees <fn>     Show functions called by <fn>
    impact <symbol>  Show symbols affected by changes to <symbol>
    complexity       Show high-complexity functions
    stats            Show codebase statistics
    audit            Run code quality audit

  Tips:
    - Use Tab for command completion
    - Use Up/Down arrows for history
    - Use Ctrl-R for reverse history search
    - Use Ctrl-D or 'exit' to quit
"#
    );
}

/// Find symbols by pattern.
fn cmd_find(db: &Database, args: &[&str], context: Option<&str>) -> Result<()> {
    let pattern = args[0];
    let limit = 20;

    let symbols = db.find_symbols_filtered(pattern, limit, context, None)?;

    if symbols.is_empty() {
        println!("No symbols found matching '{}'", pattern);
    } else {
        println!("Found {} symbols:", symbols.len());
        for sym in &symbols {
            println!(
                "  {} ({}) - {}:{}",
                sym.name,
                sym.kind.as_str(),
                sym.file_path,
                sym.line_start
            );
        }
    }

    Ok(())
}

/// Hybrid search.
fn cmd_search(db: &Database, args: &[&str]) -> Result<()> {
    let query = args.join(" ");
    let limit = 20;

    let results = db.hybrid_search(&query, limit)?;

    if results.is_empty() {
        println!("No results for '{}'", query);
    } else {
        println!("Search results:");
        for (sym, score, _source) in &results {
            println!(
                "  [{:.2}] {} ({}) - {}:{}",
                score,
                sym.name,
                sym.kind.as_str(),
                sym.file_path,
                sym.line_start
            );
        }
    }

    Ok(())
}

/// Show source code for a symbol.
fn cmd_source(db: &Database, args: &[&str], context: Option<&str>) -> Result<()> {
    let pattern = args[0];

    let symbols = db.find_symbols_filtered(pattern, 1, context, None)?;

    if let Some(sym) = symbols.first() {
        if let Some(ref source) = sym.source {
            println!("// {}:{}", sym.file_path, sym.line_start);
            println!("{}", source);
        } else {
            // Try to read from file on disk
            let file_path = std::path::Path::new(&sym.file_path);
            if file_path.exists() {
                let content = std::fs::read_to_string(file_path)?;
                let lines: Vec<&str> = content.lines().collect();
                let start = (sym.line_start as usize).saturating_sub(1);
                let end = (sym.line_end as usize).min(lines.len());

                println!("// {}:{}-{}", sym.file_path, sym.line_start, sym.line_end);
                for (i, line) in lines[start..end].iter().enumerate() {
                    println!("{:4} | {}", start + i + 1, line);
                }
            } else {
                println!("Source not available for {} ({})", sym.name, sym.file_path);
            }
        }
    } else {
        println!("Symbol not found: {}", pattern);
    }

    Ok(())
}

/// Explain a symbol.
fn cmd_explain(db: &Database, args: &[&str], context: Option<&str>) -> Result<()> {
    let pattern = args[0];

    let symbols = db.find_symbols_filtered(pattern, 1, context, None)?;

    if let Some(sym) = symbols.first() {
        println!("Symbol: {}", sym.name);
        println!("Kind:   {}", sym.kind.as_str());
        println!(
            "File:   {}:{}-{}",
            sym.file_path, sym.line_start, sym.line_end
        );

        if let Some(ref sig) = sym.signature {
            println!("Signature: {}", sig);
        }
        if let Some(ref brief) = sym.brief {
            println!("Brief: {}", brief);
        }
        if let Some(ref doc) = sym.docstring {
            println!("Documentation:\n{}", doc);
        }
    } else {
        println!("Symbol not found: {}", pattern);
    }

    Ok(())
}

/// Show callers of a function.
fn cmd_callers(_db: &Database, analytics: &Analytics, args: &[&str]) -> Result<()> {
    let name = args[0];
    let depth = 2;

    let callers = analytics.impact_analysis(name, depth)?;

    if callers.is_empty() {
        println!("No callers found for '{}'", name);
    } else {
        println!("Callers of '{}' (depth {}):", name, depth);
        for caller in &callers {
            println!(
                "  {} ({}) - {} [distance: {}]",
                caller.name, caller.kind, caller.file_path, caller.distance
            );
        }
    }

    Ok(())
}

/// Show callees of a function.
fn cmd_callees(_db: &Database, analytics: &Analytics, args: &[&str]) -> Result<()> {
    let name = args[0];
    let depth = 2;

    let callees = analytics.call_graph(name, depth)?;

    if callees.is_empty() {
        println!("No callees found for '{}'", name);
    } else {
        println!("Callees of '{}' (depth {}):", name, depth);
        for callee in &callees {
            println!(
                "  {} ({}) - {} [depth: {}]",
                callee.name, callee.kind, callee.file_path, callee.depth
            );
        }
    }

    Ok(())
}

/// Show impact analysis.
fn cmd_impact(analytics: &Analytics, args: &[&str]) -> Result<()> {
    let name = args[0];
    let depth = 3;

    let impact = analytics.impact_analysis(name, depth)?;

    if impact.is_empty() {
        println!("No impact found for '{}'", name);
    } else {
        println!("Impact of changes to '{}' (depth {}):", name, depth);
        for node in &impact {
            let indent = "  ".repeat(node.distance as usize);
            println!(
                "{}{} ({}) - {}",
                indent, node.name, node.kind, node.file_path
            );
        }
    }

    Ok(())
}

/// Show complexity analysis.
fn cmd_complexity(analytics: &Analytics) -> Result<()> {
    let results = analytics.complexity_analysis(10)?;

    let high_complexity: Vec<_> = results
        .iter()
        .filter(|r| r.fan_out > 20 || r.severity == "high" || r.severity == "critical")
        .take(20)
        .collect();

    if high_complexity.is_empty() {
        println!("No high-complexity functions found");
    } else {
        println!("High-complexity functions:");
        for r in high_complexity {
            println!(
                "  {} ({}:{}) - fan-out: {}, fan-in: {} [{}]",
                r.name, r.file_path, r.line, r.fan_out, r.fan_in, r.severity
            );
        }
    }

    Ok(())
}

/// Show codebase statistics.
fn cmd_stats(db: &Database) -> Result<()> {
    let stats = db.get_stats()?;

    println!("Codebase Statistics:");
    println!("  Files:     {}", stats.files);
    println!("  Symbols:   {}", stats.symbols);
    println!("  Functions: {}", stats.functions);
    println!("  Structs:   {}", stats.structs);
    println!("  Enums:     {}", stats.enums);
    println!("  Traits:    {}", stats.traits);
    println!("  Edges:     {}", stats.edges);

    Ok(())
}

/// Run audit command.
fn cmd_audit(db: &Database, analytics: Option<&Analytics>) -> Result<()> {
    use ctx::audit::{run_audit, AuditConfig};

    let config = AuditConfig::default();
    let report = run_audit(db, analytics, &config)?;

    println!("{}", report.format_text());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_helper_completion() {
        let helper = ShellHelper::new();
        assert!(helper.commands.contains(&"find".to_string()));
        assert!(helper.commands.contains(&"exit".to_string()));
        assert!(helper.commands.contains(&"help".to_string()));
    }

    #[test]
    fn test_default_config() {
        let config = ShellConfig::default();
        assert!(!config.no_history);
        assert!(!config.vi_mode);
    }
}
