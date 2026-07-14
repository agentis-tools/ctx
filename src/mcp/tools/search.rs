//! Search-related MCP tools.

use rmcp::model::{CallToolResult, ContentBlock, ErrorCode, Tool};
use serde_json::Value;

use super::{parse_params, schema_for, DefinitionParams, ReferencesParams, SearchParams};
use crate::mcp::server::CtxServer;

/// Helper to create an internal error.
fn internal_error(msg: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::new(ErrorCode::INTERNAL_ERROR, msg.into(), None)
}

/// Create the search_symbols tool definition.
pub fn search_symbols_tool() -> Tool {
    Tool::new(
        "search_symbols",
        "Search for symbols (functions, structs, enums, etc.) by name pattern. \
         Supports partial matching and can filter by kind and file path.",
        schema_for::<SearchParams>(),
    )
}

/// Create the get_definition tool definition.
pub fn get_definition_tool() -> Tool {
    Tool::new(
        "get_definition",
        "Get the full source code definition of a symbol. \
         Returns the complete implementation including signature, body, and documentation.",
        schema_for::<DefinitionParams>(),
    )
}

/// Create the find_references tool definition.
pub fn find_references_tool() -> Tool {
    Tool::new(
        "find_references",
        "Find all places where a symbol is referenced or called. \
         Useful for understanding how a function or type is used throughout the codebase.",
        schema_for::<ReferencesParams>(),
    )
}

/// Execute the search_symbols tool.
pub async fn search_symbols(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let params: SearchParams = parse_params(args)?;

    let limit = params.limit.unwrap_or(20);

    let symbols = server
        .with_db(|db| {
            db.find_symbols_filtered(
                &params.query,
                limit,
                params.file.as_deref(),
                params.kind.as_deref(),
            )
        })
        .map_err(|e| internal_error(e.to_string()))?;

    if symbols.is_empty() {
        return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "No symbols found matching '{}'",
            params.query
        ))]));
    }

    // Format results
    let mut output = format!(
        "Found {} symbols matching '{}':\n\n",
        symbols.len(),
        params.query
    );
    for symbol in &symbols {
        output.push_str(&format!(
            "- {} ({}) in {}:{}\n",
            symbol.name,
            symbol.kind.as_str(),
            symbol.file_path,
            symbol.line_start
        ));
        if let Some(ref sig) = symbol.signature {
            output.push_str(&format!("  Signature: {}\n", sig));
        }
        if let Some(ref brief) = symbol.brief {
            output.push_str(&format!("  Description: {}\n", brief));
        }
        output.push('\n');
    }

    Ok(CallToolResult::success(vec![ContentBlock::text(output)]))
}

/// Execute the get_definition tool.
pub async fn get_definition(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let params: DefinitionParams = parse_params(args)?;

    // First, try to find the symbol
    let symbols = server
        .with_db(|db| {
            db.find_symbols_filtered(
                &params.symbol,
                100,
                params.file.as_deref(),
                params.kind.as_deref(),
            )
        })
        .map_err(|e| internal_error(e.to_string()))?;

    if symbols.is_empty() {
        return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Symbol '{}' not found",
            params.symbol
        ))]));
    }

    // If multiple matches and no filters, show disambiguation
    if symbols.len() > 1 && params.file.is_none() && params.kind.is_none() {
        let mut output = format!(
            "Found {} symbols named '{}'. Please narrow your search using 'file' or 'kind' parameters:\n\n",
            symbols.len(),
            params.symbol
        );
        for s in symbols.iter().take(10) {
            output.push_str(&format!(
                "- {} ({}) in {}:{}\n",
                s.name,
                s.kind.as_str(),
                s.file_path,
                s.line_start
            ));
        }
        if symbols.len() > 10 {
            output.push_str(&format!("... and {} more\n", symbols.len() - 10));
        }
        return Ok(CallToolResult::success(vec![ContentBlock::text(output)]));
    }

    // Get the source for the first matching symbol
    let sym = &symbols[0];
    let sym_id = sym.id.clone();
    let source = server
        .with_db(|db| db.get_source(&sym_id))
        .map_err(|e| internal_error(e.to_string()))?;

    match source {
        Some(src) => {
            let mut output = format!(
                "// {} ({}) - {}:{}\n",
                sym.name,
                sym.kind.as_str(),
                sym.file_path,
                sym.line_start
            );
            if let Some(ref brief) = sym.brief {
                output.push_str(&format!("// {}\n", brief));
            }
            output.push('\n');
            output.push_str(&src);
            Ok(CallToolResult::success(vec![ContentBlock::text(output)]))
        }
        None => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Source code not available for '{}'",
            sym.name
        ))])),
    }
}

/// Execute the find_references tool.
pub async fn find_references(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let params: ReferencesParams = parse_params(args)?;

    // Find the symbol first
    let symbols = server
        .with_db(|db| db.find_symbols_filtered(&params.symbol, 100, params.file.as_deref(), None))
        .map_err(|e| internal_error(e.to_string()))?;

    if symbols.is_empty() {
        return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Symbol '{}' not found",
            params.symbol
        ))]));
    }

    // Get incoming edges (places that reference this symbol)
    let sym = &symbols[0];
    let sym_name = sym.name.clone();
    let edges = server
        .with_db(|db| db.get_incoming_edges(&sym_name))
        .map_err(|e| internal_error(e.to_string()))?;

    if edges.is_empty() {
        return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "No references found for '{}'",
            sym.name
        ))]));
    }

    let mut output = format!("Found {} references to '{}':\n\n", edges.len(), sym.name);

    for edge in &edges {
        let source_id = edge.source_id.clone();
        if let Ok(Some(source_sym)) = server.with_db(|db| db.get_symbol(&source_id)) {
            output.push_str(&format!(
                "- {} ({}:{})\n",
                source_sym.name,
                source_sym.file_path,
                edge.line.unwrap_or(source_sym.line_start)
            ));
            if let Some(ref ctx) = edge.context {
                output.push_str(&format!("  Context: {}\n", ctx));
            }
        }
    }

    Ok(CallToolResult::success(vec![ContentBlock::text(output)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    use crate::index::Indexer;

    /// Create a test database with some symbols for testing.
    fn setup_test_project() -> (TempDir, CtxServer) {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().to_path_buf();

        // Create a simple Rust file with testable symbols
        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            src_dir.join("lib.rs"),
            r#"
/// A greeting function.
pub fn hello_world() -> String {
    "Hello, World!".to_string()
}

/// Another function that calls hello_world.
pub fn greet() -> String {
    hello_world()
}

/// A simple struct.
pub struct Greeter {
    name: String,
}

impl Greeter {
    pub fn new(name: String) -> Self {
        Self { name }
    }

    pub fn greet(&self) -> String {
        format!("Hello, {}!", self.name)
    }
}
"#,
        )
        .unwrap();

        // Index the project using Indexer
        let mut indexer =
            Indexer::with_config(&root, false, crate::walker::WalkerConfig::default()).unwrap();
        indexer.index().unwrap();

        let server = CtxServer::new(root).unwrap();
        (temp_dir, server)
    }

    /// Helper to extract text from a content block.
    fn get_text_content(result: &CallToolResult) -> &str {
        let content = &result.content[0];
        match content {
            rmcp::model::ContentBlock::Text(text) => &text.text,
            _ => panic!("Expected text content"),
        }
    }

    #[test]
    fn test_search_symbols_tool_definition() {
        let tool = search_symbols_tool();
        assert_eq!(tool.name.as_ref(), "search_symbols");
        assert!(tool.description.is_some());
    }

    #[test]
    fn test_get_definition_tool_definition() {
        let tool = get_definition_tool();
        assert_eq!(tool.name.as_ref(), "get_definition");
        assert!(tool.description.is_some());
    }

    #[test]
    fn test_find_references_tool_definition() {
        let tool = find_references_tool();
        assert_eq!(tool.name.as_ref(), "find_references");
        assert!(tool.description.is_some());
    }

    #[tokio::test]
    async fn test_mcp_search_symbols_tool() {
        let (_temp_dir, server) = setup_test_project();

        // Test searching for "hello"
        let args = json!({
            "query": "hello"
        });
        let args_map = args.as_object().unwrap();

        let result = search_symbols(&server, Some(args_map)).await;
        assert!(result.is_ok(), "search_symbols should succeed");

        let result = result.unwrap();
        let text = get_text_content(&result);
        assert!(
            text.contains("hello_world"),
            "Result should contain hello_world function: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_search_symbols_with_kind_filter() {
        let (_temp_dir, server) = setup_test_project();

        // Test searching for structs only
        let args = json!({
            "query": "Greeter",
            "kind": "struct"
        });
        let args_map = args.as_object().unwrap();

        let result = search_symbols(&server, Some(args_map)).await;
        assert!(result.is_ok());

        let result = result.unwrap();
        let text = get_text_content(&result);
        assert!(
            text.contains("Greeter") && text.contains("struct"),
            "Result should contain Greeter struct: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_search_symbols_no_results() {
        let (_temp_dir, server) = setup_test_project();

        let args = json!({
            "query": "nonexistent_symbol_xyz"
        });
        let args_map = args.as_object().unwrap();

        let result = search_symbols(&server, Some(args_map)).await;
        assert!(result.is_ok());

        let result = result.unwrap();
        let text = get_text_content(&result);
        assert!(
            text.contains("No symbols found"),
            "Should indicate no symbols found: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_get_definition_tool() {
        let (_temp_dir, server) = setup_test_project();

        // First search to verify symbol exists and get its exact name
        let search_args = json!({
            "query": "hello_world"
        });
        let search_result = search_symbols(&server, Some(search_args.as_object().unwrap())).await;
        assert!(search_result.is_ok(), "Search should succeed");
        let search_unwrapped = search_result.unwrap();
        let search_text = get_text_content(&search_unwrapped);
        assert!(
            search_text.contains("hello_world"),
            "Should find hello_world: {}",
            search_text
        );

        // Now get definition
        let args = json!({
            "symbol": "hello_world",
            "kind": "function"
        });
        let args_map = args.as_object().unwrap();

        let result = get_definition(&server, Some(args_map)).await;
        assert!(result.is_ok(), "get_definition should succeed");

        let result = result.unwrap();
        let text = get_text_content(&result);
        // Should contain the function signature and body, or at least the symbol info
        assert!(
            text.contains("hello_world"),
            "Should contain hello_world: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_missing_params_error() {
        let (_temp_dir, server) = setup_test_project();

        // Call with no args should fail
        let result = search_symbols(&server, None).await;
        assert!(result.is_err(), "Should fail with missing params");

        let err = result.unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }
}
