//! Symbol inspection commands.
//!
//! Handles source code display and symbol explanation.

use std::env;

use ctx::error::Result;
use ctx::index;

/// Get source code for a symbol.
pub fn run_source(
    symbol: &str,
    file_pattern: Option<&str>,
    kind_filter: Option<&str>,
) -> Result<()> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Try to find by exact ID first
    if let Some(src) = db.get_source(symbol)? {
        println!("// Source: {}", symbol);
        println!("{}", src);
        return Ok(());
    }

    // Search with filters - get more results for disambiguation
    let symbols = db.find_symbols_filtered(symbol, 100, file_pattern, kind_filter)?;

    if symbols.is_empty() {
        eprintln!("Symbol '{}' not found", symbol);
        if file_pattern.is_some() || kind_filter.is_some() {
            eprintln!(
                "Try removing filters or use 'ctx query find {}' to see all matches",
                symbol
            );
        }
        return Ok(());
    }

    // If multiple symbols match and no filters, show disambiguation help
    if symbols.len() > 1 && file_pattern.is_none() && kind_filter.is_none() {
        eprintln!(
            "Found {} symbols named '{}'. Use --file or --kind to disambiguate:\n",
            symbols.len(),
            symbol
        );
        for s in symbols.iter().take(10) {
            eprintln!(
                "  {} ({}) - {}:{}",
                s.name,
                s.kind.as_str(),
                s.file_path,
                s.line_start
            );
        }
        if symbols.len() > 10 {
            eprintln!("  ... and {} more", symbols.len() - 10);
        }
        eprintln!(
            "\nExample: ctx source {} --file \"{}\"",
            symbol, symbols[0].file_path
        );
        return Ok(());
    }

    // Get the first matching symbol's source
    let sym = &symbols[0];
    match db.get_source(&sym.id)? {
        Some(src) => {
            println!("// Source: {}", sym.id);
            println!("{}", src);
        }
        None => {
            eprintln!("Source code not available for '{}'", sym.id);
        }
    }

    Ok(())
}

/// Explain a symbol with its relationships.
pub fn run_explain(
    symbol: &str,
    file_pattern: Option<&str>,
    kind_filter: Option<&str>,
) -> Result<()> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Search with filters - get more results for disambiguation
    let symbols = db.find_symbols_filtered(symbol, 100, file_pattern, kind_filter)?;

    if symbols.is_empty() {
        eprintln!("Symbol '{}' not found", symbol);
        if file_pattern.is_some() || kind_filter.is_some() {
            eprintln!(
                "Try removing filters or use 'ctx query find {}' to see all matches",
                symbol
            );
        }
        return Ok(());
    }

    // If multiple symbols match and no filters, show disambiguation help
    if symbols.len() > 1 && file_pattern.is_none() && kind_filter.is_none() {
        eprintln!(
            "Found {} symbols named '{}'. Use --file or --kind to disambiguate:\n",
            symbols.len(),
            symbol
        );
        for s in symbols.iter().take(10) {
            eprintln!(
                "  {} ({}) - {}:{}",
                s.name,
                s.kind.as_str(),
                s.file_path,
                s.line_start
            );
        }
        if symbols.len() > 10 {
            eprintln!("  ... and {} more", symbols.len() - 10);
        }
        eprintln!(
            "\nExample: ctx explain {} --file \"{}\"",
            symbol, symbols[0].file_path
        );
        return Ok(());
    }

    let sym = &symbols[0];

    println!("Symbol: {}", sym.name);
    println!("{}", "=".repeat(60));
    println!("Kind:       {}", sym.kind.as_str());
    println!("File:       {}:{}", sym.file_path, sym.line_start);
    println!("Visibility: {}", sym.visibility.as_str());

    if let Some(ref sig) = sym.signature {
        println!("\nSignature:");
        println!("  {}", sig);
    }

    if let Some(ref brief) = sym.brief {
        println!("\nDescription:");
        println!("  {}", brief);
    }

    // Show callers
    let callers = db.get_incoming_edges(&sym.name)?;
    if !callers.is_empty() {
        println!("\nCalled by ({}):", callers.len());
        for edge in callers.iter().take(10) {
            if let Some(caller) = db.get_symbol(&edge.source_id)? {
                println!(
                    "  {} ({}:{})",
                    caller.name,
                    caller.file_path,
                    edge.line.unwrap_or(0)
                );
            }
        }
        if callers.len() > 10 {
            println!("  ... and {} more", callers.len() - 10);
        }
    }

    // Show dependencies
    let deps = db.get_outgoing_edges(&sym.id)?;
    if !deps.is_empty() {
        println!("\nCalls ({}):", deps.len());
        for edge in deps.iter().take(10) {
            println!("  {} [{}]", edge.target_name, edge.kind.as_str());
        }
        if deps.len() > 10 {
            println!("  ... and {} more", deps.len() - 10);
        }
    }

    Ok(())
}
