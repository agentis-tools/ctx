//! Analysis-related MCP tools.

use std::collections::{HashSet, VecDeque};

use rmcp::model::{CallToolResult, ContentBlock, ErrorCode, Tool};
use serde_json::Value;

use super::{
    bounded_mcp_depth, bounded_mcp_output, bounded_mcp_top, parse_params, schema_for,
    CallGraphParams, SmartContextParams,
};
use crate::db::{Database, EdgeKind, Symbol};
use crate::limits::MAX_MCP_SEARCH_RESULTS;
use crate::mcp::server::CtxServer;

/// Helper to create an internal error.
fn internal_error(msg: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::new(ErrorCode::INTERNAL_ERROR, msg.into(), None)
}

type GraphResult = crate::error::Result<Vec<(Symbol, Option<u32>, Option<String>, i32)>>;

/// Traverse resolved call edges in reverse, honoring the MCP depth parameter.
fn collect_callers(db: &Database, start_id: &str, max_depth: i32) -> GraphResult {
    let max_depth = bounded_mcp_depth(Some(max_depth));
    let mut queue = VecDeque::from([(start_id.to_string(), 0)]);
    let mut visited = HashSet::from([start_id.to_string()]);
    let mut callers = Vec::new();

    while let Some((target_id, depth)) = queue.pop_front() {
        if callers.len() >= MAX_MCP_SEARCH_RESULTS as usize {
            break;
        }
        if depth >= max_depth {
            continue;
        }
        let next_depth = depth + 1;
        for edge in db.get_incoming_edges_limited(&target_id, MAX_MCP_SEARCH_RESULTS)? {
            if callers.len() >= MAX_MCP_SEARCH_RESULTS as usize {
                break;
            }
            if edge.kind != EdgeKind::Calls
                || edge.target_id.as_deref() != Some(target_id.as_str())
                || !visited.insert(edge.source_id.clone())
            {
                continue;
            }

            if let Some(caller) = db.get_symbol(&edge.source_id)? {
                queue.push_back((caller.id.clone(), next_depth));
                callers.push((caller, edge.line, edge.context, next_depth));
            }
        }
    }

    Ok(callers)
}

/// Traverse resolved call edges forward, honoring the MCP depth parameter.
fn collect_callees(db: &Database, start_id: &str, max_depth: i32) -> GraphResult {
    let max_depth = bounded_mcp_depth(Some(max_depth));
    let mut queue = VecDeque::from([(start_id.to_string(), 0)]);
    let mut visited = HashSet::from([start_id.to_string()]);
    let mut callees = Vec::new();

    while let Some((source_id, depth)) = queue.pop_front() {
        if callees.len() >= MAX_MCP_SEARCH_RESULTS as usize {
            break;
        }
        if depth >= max_depth {
            continue;
        }
        let next_depth = depth + 1;
        for edge in db.get_outgoing_edges_limited(&source_id, MAX_MCP_SEARCH_RESULTS)? {
            if callees.len() >= MAX_MCP_SEARCH_RESULTS as usize {
                break;
            }
            let Some(target_id) = edge.target_id.as_deref() else {
                continue;
            };
            if edge.kind != EdgeKind::Calls || !visited.insert(target_id.to_string()) {
                continue;
            }

            if let Some(callee) = db.get_symbol(target_id)? {
                queue.push_back((callee.id.clone(), next_depth));
                callees.push((callee, edge.line, edge.context, next_depth));
            }
        }
    }

    Ok(callees)
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
    let depth = bounded_mcp_depth(params.depth);

    // Find the function first
    let symbols = server
        .with_db(|db| {
            db.find_symbols_filtered(
                &params.function,
                100,
                params.file.as_deref(),
                Some("function"),
            )
        })
        .map_err(|e| internal_error(e.to_string()))?;

    if symbols.is_empty() {
        return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Function '{}' not found",
            params.function
        ))]));
    }

    let sym = &symbols[0];
    // Traverse incoming resolved call edges to the requested depth.
    let callers = server
        .with_db(|db| collect_callers(db, &sym.id, depth))
        .map_err(|e| internal_error(e.to_string()))?;

    if callers.is_empty() {
        return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "No callers found for '{}'",
            sym.name
        ))]));
    }

    let mut output = format!(
        "Functions that call '{}' ({} within depth {}):\n\n",
        sym.name,
        callers.len(),
        depth
    );

    for (caller, line, context, distance) in callers {
        output.push_str(&format!(
            "- {} ({}:{}, depth {})\n",
            caller.name,
            caller.file_path,
            line.unwrap_or(caller.line_start),
            distance
        ));
        if let Some(ctx) = context {
            output.push_str(&format!("  Call: {}\n", ctx));
        }
    }

    Ok(CallToolResult::success(vec![ContentBlock::text(
        bounded_mcp_output(output),
    )]))
}

/// Execute the get_callees tool.
pub async fn get_callees(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let params: CallGraphParams = parse_params(args)?;
    let depth = bounded_mcp_depth(params.depth);

    // Find the function first
    let symbols = server
        .with_db(|db| {
            db.find_symbols_filtered(
                &params.function,
                100,
                params.file.as_deref(),
                Some("function"),
            )
        })
        .map_err(|e| internal_error(e.to_string()))?;

    if symbols.is_empty() {
        return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Function '{}' not found",
            params.function
        ))]));
    }

    let sym = &symbols[0];
    // Traverse outgoing resolved call edges to the requested depth.
    let callees = server
        .with_db(|db| collect_callees(db, &sym.id, depth))
        .map_err(|e| internal_error(e.to_string()))?;

    if callees.is_empty() {
        return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "No function calls found in '{}'",
            sym.name
        ))]));
    }

    let mut output = format!(
        "Functions called by '{}' ({} within depth {}):\n\n",
        sym.name,
        callees.len(),
        depth
    );

    for (callee, line, context, distance) in callees {
        output.push_str(&format!(
            "- {} [calls] ({}:{}, depth {})\n",
            callee.name,
            callee.file_path,
            line.unwrap_or(callee.line_start),
            distance
        ));
        if let Some(ctx) = context {
            output.push_str(&format!("  Call: {}\n", ctx));
        }
    }

    Ok(CallToolResult::success(vec![ContentBlock::text(
        bounded_mcp_output(output),
    )]))
}

/// Execute the smart_context tool.
pub async fn smart_context(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    use crate::embeddings::local::LocalProvider;
    use crate::embeddings::ollama::OllamaProvider;
    use crate::embeddings::openai::OpenAIProvider;
    use crate::embeddings::{Embedding, EmbeddingProvider, Provider};
    use crate::limits::MAX_MCP_RESPONSE_BYTES;
    use crate::output::read_file_content_with_limit;
    use crate::smart::{smart_context_with_embedding_filtered, SmartConfig};
    use crate::tokens::Encoding;
    use crate::walker::{secure_file_path, FilePatternFilter};

    let params: SmartContextParams = parse_params(args)?;

    // Check if embeddings exist
    let embedding_count = server
        .with_db(|db| db.count_embeddings())
        .map_err(|e| internal_error(e.to_string()))?;

    if embedding_count == 0 {
        return Err(internal_error(
            "No embeddings found. Run 'ctx embed' first to generate embeddings.",
        ));
    }

    // Check if analytics is available
    let has_analytics = server.with_analytics(|_| ()).is_some();
    if !has_analytics {
        return Err(internal_error(
            "Analytics not available. Run 'ctx index' first.",
        ));
    }

    // Configure smart context
    let config = SmartConfig {
        max_tokens: params.max_tokens.unwrap_or(8000),
        depth: bounded_mcp_depth(params.depth),
        top: bounded_mcp_top(params.top),
        encoding: Encoding::default(),
    };

    // Resolve provider: explicit `provider` string wins, else the deprecated
    // `use_openai` bool, else the trusted `.ctx/config.toml` default, else
    // local. Network providers embed asynchronously so they don't block the
    // async runtime. Project model/host settings are likewise honored only
    // after the server was launched with project trust.
    // (Named `project_config` so it doesn't shadow the `SmartConfig` above.)
    let project_config = crate::config::CtxConfig::load(server.root());
    let project_provider = if server.trust_project() {
        project_config.embedding.provider
    } else {
        None
    };
    let provider = match params.provider.as_deref() {
        Some("openai") => Provider::Openai,
        Some("ollama") => Provider::Ollama,
        Some("local") => Provider::Local,
        None => Provider::resolve(None, params.use_openai.unwrap_or(false), project_provider),
        Some(other) => {
            return Err(internal_error(format!(
                "Unknown provider '{}'. Expected: local, openai, or ollama.",
                other
            )))
        }
    };

    let task_embedding: Embedding = match provider {
        Provider::Openai => {
            let provider = OpenAIProvider::from_env().map_err(|e| {
                internal_error(format!(
                    "Failed to initialize OpenAI provider: {}. Set OPENAI_API_KEY environment variable.",
                    e
                ))
            })?;
            provider
                .embed_async(&params.task)
                .await
                .map_err(|e| internal_error(format!("Failed to generate embedding: {}", e)))?
        }
        Provider::Ollama => {
            let provider = OllamaProvider::from_config_async(
                project_config
                    .embedding
                    .model
                    .as_deref()
                    .filter(|_| server.trust_project()),
                project_config
                    .embedding
                    .host
                    .as_deref()
                    .filter(|_| server.trust_project()),
            )
            .await
            .map_err(|e| internal_error(format!("Failed to initialize Ollama provider: {}", e)))?;
            provider
                .embed_async(&params.task)
                .await
                .map_err(|e| internal_error(format!("Failed to generate embedding: {}", e)))?
        }
        Provider::Local => {
            // Local fastembed is CPU-bound; sync embed is fine.
            let provider = LocalProvider::new().map_err(|e| {
                internal_error(format!("Failed to initialize embedding model: {}", e))
            })?;
            provider
                .embed(&params.task)
                .map_err(|e| internal_error(format!("Failed to generate embedding: {}", e)))?
        }
    };

    // Run smart context selection with pre-computed embedding
    let result = {
        let db = server.db.lock().unwrap();
        let analytics = server
            .analytics
            .as_ref()
            .ok_or_else(|| internal_error("Analytics not available"))?
            .lock()
            .unwrap();

        let filter = FilePatternFilter::all(server.root());
        smart_context_with_embedding_filtered(
            &db,
            &analytics,
            &params.task,
            &task_embedding,
            config,
            &filter,
        )
    }
    .map_err(|e| internal_error(format!("Smart context selection failed: {}", e)))?;

    if result.selected_files.is_empty() {
        return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "No relevant files found for task: \"{}\"",
            params.task
        ))]));
    }

    // Format output
    let mut output = format!("Smart context for: \"{}\"\n\n", params.task);
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
        let Ok(path) = secure_file_path(root, std::path::Path::new(&file.path)) else {
            continue;
        };
        let remaining = MAX_MCP_RESPONSE_BYTES.saturating_sub(output.len()) as u64;
        if remaining == 0 {
            break;
        }
        if let Ok(content) = read_file_content_with_limit(&path, remaining) {
            output.push_str(&format!("// === {} ===\n\n", file.path));
            output.push_str(&content);
            output.push_str("\n\n");
        } else {
            // The next file did not fit in the remaining response budget (or
            // exceeded the per-file cap). Stop before attempting more reads.
            break;
        }
    }

    Ok(CallToolResult::success(vec![ContentBlock::text(
        bounded_mcp_output(output),
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Edge, FileRecord, SymbolKind, Visibility};

    fn test_symbol(id: &str, name: &str, line: u32) -> Symbol {
        Symbol {
            id: id.to_string(),
            file_path: "test.rs".to_string(),
            name: name.to_string(),
            qualified_name: None,
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: None,
            brief: None,
            docstring: None,
            line_start: line,
            line_end: line,
            col_start: 0,
            col_end: 0,
            parent_id: None,
            source: None,
        }
    }

    fn test_edge(source_id: &str, target_id: &str) -> Edge {
        Edge {
            source_id: source_id.to_string(),
            target_id: Some(target_id.to_string()),
            target_name: target_id.to_string(),
            kind: EdgeKind::Calls,
            line: Some(1),
            col: Some(0),
            context: None,
        }
    }

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

    #[test]
    fn graph_helpers_honor_depth_and_deduplicate_cycles() {
        let db = Database::open_in_memory().unwrap();
        db.upsert_file(
            &FileRecord {
                path: "test.rs".to_string(),
                content_hash: "test".to_string(),
                size_bytes: 0,
                language: Some("rust".to_string()),
                last_indexed: 0,
            },
            None,
        )
        .unwrap();
        for (id, name, line) in [("a", "a", 1), ("b", "b", 2), ("c", "c", 3)] {
            db.insert_symbol(&test_symbol(id, name, line)).unwrap();
        }
        db.insert_edge(&test_edge("a", "b")).unwrap();
        db.insert_edge(&test_edge("a", "b")).unwrap();
        db.insert_edge(&test_edge("b", "c")).unwrap();
        db.insert_edge(&test_edge("c", "a")).unwrap();

        assert_eq!(collect_callees(&db, "a", 1).unwrap().len(), 1);
        assert_eq!(
            collect_callees(&db, "a", i32::MAX)
                .unwrap()
                .iter()
                .map(|(symbol, _, _, _)| symbol.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c"]
        );
        assert_eq!(collect_callers(&db, "b", 1).unwrap().len(), 1);
        assert_eq!(collect_callers(&db, "b", 2).unwrap().len(), 2);
    }
}
