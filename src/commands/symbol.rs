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
    json: bool,
) -> Result<()> {
    let root = env::current_dir()?;
    let db = index::open_database(&root)?;

    // Search with filters - get more results for disambiguation
    let symbols = db.find_symbols_filtered(symbol, 100, file_pattern, kind_filter)?;

    if symbols.is_empty() {
        if json {
            return ctx::json::emit("explain", explain_not_found_data(&[]));
        }
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
        if json {
            return ctx::json::emit("explain", explain_not_found_data(&symbols));
        }
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

    if json {
        let callers = resolved_callers(&db, sym)?;
        let deps = db.get_outgoing_edges(&sym.id)?;
        return ctx::json::emit("explain", explain_data(sym, callers.len(), deps.len()));
    }

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
    let callers = resolved_callers(&db, sym)?;
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
    let (calls, relationships): (Vec<_>, Vec<_>) = deps
        .iter()
        .partition(|edge| edge.kind == ctx::db::EdgeKind::Calls);
    if !calls.is_empty() {
        println!("\nCalls ({}):", calls.len());
        for edge in calls.iter().take(10) {
            println!("  {} [{}]", edge.target_name, edge.kind.as_str());
        }
        if calls.len() > 10 {
            println!("  ... and {} more", calls.len() - 10);
        }
    }
    if !relationships.is_empty() {
        println!("\nRelationships ({}):", relationships.len());
        for edge in relationships.iter().take(10) {
            println!("  {} [{}]", edge.target_name, edge.kind.as_str());
        }
        if relationships.len() > 10 {
            println!("  ... and {} more", relationships.len() - 10);
        }
    }

    Ok(())
}

/// Return only actual, resolved call edges to the selected symbol.
///
/// Other incoming relationships, including function-item `uses` references,
/// are evidence but are not callers.
fn resolved_callers(db: &ctx::db::Database, sym: &ctx::db::Symbol) -> Result<Vec<ctx::db::Edge>> {
    Ok(db
        .get_incoming_edges(&sym.id)?
        .into_iter()
        .filter(|edge| {
            edge.kind == ctx::db::EdgeKind::Calls
                && edge.target_id.as_deref() == Some(sym.id.as_str())
        })
        .collect())
}

/// Build the `explain` JSON payload for a resolved symbol.
fn explain_data(
    sym: &ctx::db::Symbol,
    callers_count: usize,
    deps_count: usize,
) -> serde_json::Value {
    serde_json::json!({
        "symbol": ctx::json::SymbolRef::from(sym),
        "visibility": sym.visibility.as_str(),
        "signature": sym.signature,
        "brief": sym.brief,
        "docstring": sym.docstring,
        "callers_count": callers_count,
        "deps_count": deps_count,
        "ambiguous": [],
    })
}

/// Build the `explain` JSON payload when no unique symbol was resolved.
///
/// `ambiguous` is empty when the symbol was simply not found.
fn explain_not_found_data(ambiguous: &[ctx::db::Symbol]) -> serde_json::Value {
    serde_json::json!({
        "symbol": serde_json::Value::Null,
        "visibility": serde_json::Value::Null,
        "signature": serde_json::Value::Null,
        "brief": serde_json::Value::Null,
        "docstring": serde_json::Value::Null,
        "callers_count": 0,
        "deps_count": 0,
        "ambiguous": ambiguous
            .iter()
            .map(ctx::json::SymbolRef::from)
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx::db::{Symbol, SymbolKind, Visibility};

    #[test]
    fn test_explain_data_payload() {
        let sym = Symbol {
            id: "src/a.rs::run".to_string(),
            file_path: "src/a.rs".to_string(),
            name: "run".to_string(),
            qualified_name: Some("App::run".to_string()),
            kind: SymbolKind::Method,
            visibility: Visibility::Public,
            signature: Some("fn run(&self) -> Result<()>".to_string()),
            brief: Some("Run the app".to_string()),
            docstring: None,
            line_start: 7,
            line_end: 30,
            col_start: 0,
            col_end: 1,
            parent_id: None,
            source: None,
        };

        let data = explain_data(&sym, 3, 5);
        assert_eq!(data["symbol"]["name"], "run");
        assert_eq!(data["symbol"]["qualified_name"], "App::run");
        assert_eq!(data["symbol"]["kind"], "method");
        assert_eq!(data["visibility"], "public");
        assert_eq!(data["signature"], "fn run(&self) -> Result<()>");
        assert_eq!(data["callers_count"], 3);
        assert_eq!(data["deps_count"], 5);
        assert_eq!(data["ambiguous"], serde_json::json!([]));

        let not_found = explain_not_found_data(&[]);
        assert!(not_found["symbol"].is_null());
        assert_eq!(not_found["callers_count"], 0);

        let ambiguous = explain_not_found_data(std::slice::from_ref(&sym));
        assert_eq!(ambiguous["ambiguous"][0]["name"], "run");
    }
}
