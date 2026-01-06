//! MCP tool implementations for ctx.

pub mod analysis;
pub mod files;
pub mod search;

use rmcp::model::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Helper to create a JSON schema object from a type.
fn schema_for<T: JsonSchema>() -> Arc<serde_json::Map<String, serde_json::Value>> {
    let schema = schemars::schema_for!(T);
    let value = serde_json::to_value(schema).unwrap_or_default();
    if let serde_json::Value::Object(obj) = value {
        Arc::new(obj)
    } else {
        Arc::new(serde_json::Map::new())
    }
}

/// Get all available tools.
pub fn get_all_tools() -> Vec<Tool> {
    vec![
        // Search tools
        search::search_symbols_tool(),
        search::get_definition_tool(),
        search::find_references_tool(),
        // File tools
        files::get_file_tool(),
        files::get_file_tree_tool(),
        // Analysis tools
        analysis::get_callers_tool(),
        analysis::get_callees_tool(),
        analysis::smart_context_tool(),
    ]
}

// Common parameter types used across tools

/// Parameters for searching symbols.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// The search query (symbol name or pattern).
    pub query: String,
    /// Maximum number of results to return (default: 20).
    #[serde(default = "default_limit")]
    pub limit: Option<i32>,
    /// Filter by symbol kind (function, struct, enum, trait, etc.).
    pub kind: Option<String>,
    /// Filter by file path pattern (glob syntax).
    pub file: Option<String>,
}

/// Parameters for getting a symbol definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DefinitionParams {
    /// The symbol name to get the definition for.
    pub symbol: String,
    /// Filter by file path pattern (glob syntax).
    pub file: Option<String>,
    /// Filter by symbol kind.
    pub kind: Option<String>,
}

/// Parameters for finding references.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReferencesParams {
    /// The symbol name to find references to.
    pub symbol: String,
    /// Filter by file path pattern (glob syntax).
    pub file: Option<String>,
}

/// Parameters for getting a file's contents.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetFileParams {
    /// The file path relative to the project root.
    pub path: String,
}

/// Parameters for listing files.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileTreeParams {
    /// Optional directory path to list (defaults to project root).
    pub path: Option<String>,
    /// File pattern to match (glob syntax, e.g., "*.rs").
    pub pattern: Option<String>,
    /// Maximum depth to traverse (default: unlimited).
    pub depth: Option<u32>,
}

/// Parameters for getting callers/callees.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CallGraphParams {
    /// The function name to analyze.
    pub function: String,
    /// Filter by file path pattern (glob syntax).
    pub file: Option<String>,
    /// Maximum depth to traverse (default: 3).
    #[serde(default = "default_depth")]
    pub depth: Option<i32>,
}

/// Parameters for smart context selection.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SmartContextParams {
    /// Natural language description of the task.
    pub task: String,
    /// Maximum tokens in output (default: 8000).
    #[serde(default = "default_max_tokens")]
    pub max_tokens: Option<usize>,
    /// Call graph expansion depth (default: 2).
    #[serde(default = "default_depth")]
    pub depth: Option<i32>,
    /// Number of initial semantic matches (default: 10).
    #[serde(default = "default_top")]
    pub top: Option<usize>,
}

fn default_limit() -> Option<i32> {
    Some(20)
}

fn default_depth() -> Option<i32> {
    Some(3)
}

fn default_max_tokens() -> Option<usize> {
    Some(8000)
}

fn default_top() -> Option<usize> {
    Some(10)
}
