//! Analysis-related MCP tools.

use rmcp::model::{CallToolResult, Content, ErrorCode, Tool};
use serde_json::Value;

use super::{schema_for, CallGraphParams, SmartContextParams};
use crate::mcp::server::CtxServer;

/// Helper to create an internal error.
fn internal_error(msg: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::new(ErrorCode::INTERNAL_ERROR, msg.into(), None)
}

/// Helper to create an invalid params error.
#[allow(dead_code)]
fn invalid_params(msg: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::new(ErrorCode::INVALID_PARAMS, msg.into(), None)
}

/// Create the get_callers tool definition.
pub fn get_callers_tool() -> Tool {
    Tool::new(
        "get_callers",
        "Find all functions that call a given function. \
         Useful for understanding the impact of changes and the call hierarchy.",
        schema_for::<CallGraphParams>(),
    )
}

/// Create the get_callees tool definition.
pub fn get_callees_tool() -> Tool {
    Tool::new(
        "get_callees",
        "Find all functions called by a given function. \
         Useful for understanding dependencies and what a function relies on.",
        schema_for::<CallGraphParams>(),
    )
}

/// Create the smart_context tool definition.
pub fn smart_context_tool() -> Tool {
    Tool::new(
        "smart_context",
        "Intelligently select relevant files for a given task using semantic search \
         and call graph analysis. Returns the most relevant code for implementing \
         a feature, fixing a bug, or understanding a concept.",
        schema_for::<SmartContextParams>(),
    )
}

/// Execute the get_callers tool.
pub async fn get_callers(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let params: CallGraphParams = parse_params(args)?;

    // Find the function first
    let symbols = server.with_db(|db| {
        db.find_symbols_filtered(&params.function, 100, params.file.as_deref(), Some("function"))
    }).map_err(|e| internal_error(e.to_string()))?;

    if symbols.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(format!(
            "Function '{}' not found",
            params.function
        ))]));
    }

    let sym = &symbols[0];
    let sym_name = sym.name.clone();

    // Get incoming edges (callers)
    let edges = server.with_db(|db| db.get_incoming_edges(&sym_name))
        .map_err(|e| internal_error(e.to_string()))?;

    if edges.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(format!(
            "No callers found for '{}'",
            sym.name
        ))]));
    }

    let mut output = format!(
        "Functions that call '{}' ({}):\n\n",
        sym.name,
        edges.len()
    );

    for edge in &edges {
        let source_id = edge.source_id.clone();
        if let Ok(Some(caller)) = server.with_db(|db| db.get_symbol(&source_id)) {
            output.push_str(&format!(
                "- {} ({}:{})\n",
                caller.name,
                caller.file_path,
                edge.line.unwrap_or(caller.line_start)
            ));
            if let Some(ref ctx) = edge.context {
                output.push_str(&format!("  Call: {}\n", ctx));
            }
        }
    }

    Ok(CallToolResult::success(vec![Content::text(output)]))
}

/// Execute the get_callees tool.
pub async fn get_callees(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let params: CallGraphParams = parse_params(args)?;

    // Find the function first
    let symbols = server.with_db(|db| {
        db.find_symbols_filtered(&params.function, 100, params.file.as_deref(), Some("function"))
    }).map_err(|e| internal_error(e.to_string()))?;

    if symbols.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(format!(
            "Function '{}' not found",
            params.function
        ))]));
    }

    let sym = &symbols[0];
    let sym_id = sym.id.clone();

    // Get outgoing edges (callees)
    let edges = server.with_db(|db| db.get_outgoing_edges(&sym_id))
        .map_err(|e| internal_error(e.to_string()))?;

    if edges.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(format!(
            "No function calls found in '{}'",
            sym.name
        ))]));
    }

    let mut output = format!(
        "Functions called by '{}' ({}):\n\n",
        sym.name,
        edges.len()
    );

    for edge in &edges {
        output.push_str(&format!(
            "- {} [{}] (line {})\n",
            edge.target_name,
            edge.kind.as_str(),
            edge.line.unwrap_or(0)
        ));
    }

    Ok(CallToolResult::success(vec![Content::text(output)]))
}

/// Execute the smart_context tool.
pub async fn smart_context(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    use crate::embeddings::local::LocalProvider;
    use crate::smart::{smart_context as do_smart_context, SmartConfig};
    use crate::tokens::Encoding;

    let params: SmartContextParams = parse_params(args)?;

    // Check if embeddings exist
    let embedding_count = server.with_db(|db| db.count_embeddings())
        .map_err(|e| internal_error(e.to_string()))?;

    if embedding_count == 0 {
        return Err(internal_error(
            "No embeddings found. Run 'ctx embed' first to generate embeddings."
        ));
    }

    // Check if analytics is available
    let has_analytics = server.with_analytics(|_| ()).is_some();
    if !has_analytics {
        return Err(internal_error(
            "Analytics not available. Run 'ctx index' first."
        ));
    }

    // Create embedding provider
    let provider = LocalProvider::new().map_err(|e| {
        internal_error(format!("Failed to initialize embedding model: {}", e))
    })?;

    // Configure smart context
    let config = SmartConfig {
        max_tokens: params.max_tokens.unwrap_or(8000),
        depth: params.depth.unwrap_or(2),
        top: params.top.unwrap_or(10),
        encoding: Encoding::default(),
    };

    // Run smart context selection
    // We need to get both db and analytics at the same time
    // Since we can't hold both locks, we'll do this in a more complex way
    let result = {
        let db = server.db.lock().unwrap();
        let analytics = server.analytics.as_ref()
            .ok_or_else(|| internal_error("Analytics not available"))?
            .lock().unwrap();
        
        do_smart_context(&db, &analytics, &provider, &params.task, config)
    }.map_err(|e| internal_error(format!("Smart context selection failed: {}", e)))?;

    if result.selected_files.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(format!(
            "No relevant files found for task: \"{}\"",
            params.task
        ))]));
    }

    // Format output
    let mut output = format!(
        "Smart context for: \"{}\"\n\n",
        params.task
    );
    output.push_str(&format!(
        "Selected {} files ({} tokens){}:\n\n",
        result.selected_files.len(),
        result.total_tokens,
        if result.truncated {
            format!(", {} omitted due to token limit", result.omitted_count)
        } else {
            String::new()
        }
    ));

    for file in &result.selected_files {
        output.push_str(&format!(
            "- {} (relevance: {:.0}%, {} tokens)\n",
            file.path,
            file.relevance_score * 100.0,
            file.token_count
        ));
        for reason in &file.reasons {
            output.push_str(&format!("  - {:?}\n", reason));
        }
    }

    // Include the actual file contents if they fit
    output.push_str("\n---\n\nSelected file contents:\n\n");

    let root = server.root();
    for file in &result.selected_files {
        let path = root.join(&file.path);
        if let Ok(content) = std::fs::read_to_string(&path) {
            output.push_str(&format!("// === {} ===\n\n", file.path));
            output.push_str(&content);
            output.push_str("\n\n");
        }
    }

    Ok(CallToolResult::success(vec![Content::text(output)]))
}

/// Parse tool parameters from JSON.
fn parse_params<T: serde::de::DeserializeOwned>(
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<T, rmcp::ErrorData> {
    let args = args.ok_or_else(|| invalid_params("Missing required parameters"))?;

    serde_json::from_value(Value::Object(args.clone()))
        .map_err(|e| invalid_params(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_callers_tool_definition() {
        let tool = get_callers_tool();
        assert_eq!(tool.name.as_ref(), "get_callers");
        assert!(tool.description.is_some());
    }

    #[test]
    fn test_get_callees_tool_definition() {
        let tool = get_callees_tool();
        assert_eq!(tool.name.as_ref(), "get_callees");
        assert!(tool.description.is_some());
    }

    #[test]
    fn test_smart_context_tool_definition() {
        let tool = smart_context_tool();
        assert_eq!(tool.name.as_ref(), "smart_context");
        assert!(tool.description.is_some());
    }
}
