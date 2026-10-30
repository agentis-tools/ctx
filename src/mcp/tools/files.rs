//! File-related MCP tools.

use rmcp::model::{CallToolResult, ContentBlock, ErrorCode, Tool};
use serde_json::Value;

use super::{
    bounded_mcp_output, invalid_params, parse_params, schema_for, FileTreeParams, GetFileParams,
};
use crate::limits::MAX_MCP_RESPONSE_BYTES;
use crate::mcp::server::CtxServer;
use crate::output::read_file_content;
use crate::walker::{discover_files, secure_file_path, validate_project_path, WalkerConfig};

/// Helper to create an internal error.
fn internal_error(msg: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::new(ErrorCode::INTERNAL_ERROR, msg.into(), None)
}

/// Create the get_file tool definition.
pub fn get_file_tool() -> Tool {
    Tool::new(
        "get_file",
        "Read the contents of a file. Returns the full file content as text. \
         Use this when you need to examine the implementation details of a specific file.",
        schema_for::<GetFileParams>(),
    )
}

/// Create the get_file_tree tool definition.
pub fn get_file_tree_tool() -> Tool {
    Tool::new(
        "get_file_tree",
        "List files in the project directory. Can filter by path and pattern. \
         Use this to explore the project structure and find relevant files.",
        schema_for::<FileTreeParams>(),
    )
}

/// Execute the get_file tool.
pub async fn get_file(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let params: GetFileParams = parse_params(args)?;

    let root = server.root();
    // Security check: reject absolute/parent/symlink paths before opening the
    // file.  The database and MCP caller are both untrusted inputs at this
    // boundary, so lexical prefix checks alone are insufficient.
    let canonical = secure_file_path(root, std::path::Path::new(&params.path))
        .map_err(|e| invalid_params(format!("Invalid path: {}", e)))?;

    // Read the file
    let content = read_file_content(&canonical).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            invalid_params(format!("File not found: {}", params.path))
        } else {
            internal_error(e.to_string())
        }
    })?;

    // Format output with file path header
    let output = format!("// File: {}\n\n{}", params.path, content);

    Ok(CallToolResult::success(vec![ContentBlock::text(
        bounded_mcp_output(output),
    )]))
}

/// Execute the get_file_tree tool.
pub async fn get_file_tree(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let params: FileTreeParams = parse_params(args).unwrap_or(FileTreeParams {
        path: None,
        pattern: None,
        depth: None,
    });

    let root = server.root();
    let start_path = if let Some(ref p) = params.path {
        validate_project_path(root, std::path::Path::new(p), false)
            .map_err(|e| invalid_params(format!("Invalid path: {}", e)))?
    } else {
        root.clone()
    };

    // Build walker config
    let patterns = if let Some(ref pattern) = params.pattern {
        vec![pattern.clone()]
    } else {
        vec![".".to_string()]
    };

    let config = WalkerConfig {
        use_gitignore: true,
        use_default_ignores: true,
        custom_ignores: vec![],
        include_patterns: patterns,
    };

    // Discover files
    let entries =
        discover_files(&start_path, &config).map_err(|e| internal_error(e.to_string()))?;

    if entries.is_empty() {
        return Ok(CallToolResult::success(vec![ContentBlock::text(
            "No files found matching the criteria",
        )]));
    }

    // Apply depth filter if specified
    let entries: Vec<_> = if let Some(max_depth) = params.depth {
        let max_depth = max_depth.min(crate::limits::MAX_GRAPH_DEPTH as u32);
        entries
            .into_iter()
            .filter(|e| {
                let depth = e.relative_path.components().count() as u32;
                depth <= max_depth
            })
            .collect()
    } else {
        entries
    };

    // Format output as a tree-like structure
    let mut output = format!("Project files ({} found):\n\n", entries.len());

    // Group by directory for better organization
    let mut current_dir = String::new();
    for entry in &entries {
        if output.len() >= MAX_MCP_RESPONSE_BYTES {
            break;
        }
        let path_str = entry.relative_path.display().to_string();
        let parent = entry
            .relative_path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        if parent != current_dir {
            if !parent.is_empty() {
                output.push_str(&format!("\n{}:\n", parent));
            }
            current_dir = parent;
        }

        let file_name = entry
            .relative_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(path_str);

        output.push_str(&format!("  {}\n", file_name));
    }

    Ok(CallToolResult::success(vec![ContentBlock::text(
        bounded_mcp_output(output),
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use tempfile::TempDir;

    use crate::index::Indexer;

    fn setup_test_project() -> (TempDir, CtxServer) {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn hello() {}\n").unwrap();
        let mut indexer = Indexer::with_config(&root, false, WalkerConfig::default()).unwrap();
        indexer.index().unwrap();
        let server = CtxServer::new(root).unwrap();
        (temp_dir, server)
    }

    #[test]
    fn test_get_file_tool_definition() {
        let tool = get_file_tool();
        assert_eq!(tool.name.as_ref(), "get_file");
        assert!(tool.description.is_some());
    }

    #[test]
    fn test_get_file_tree_tool_definition() {
        let tool = get_file_tree_tool();
        assert_eq!(tool.name.as_ref(), "get_file_tree");
        assert!(tool.description.is_some());
    }

    #[tokio::test]
    async fn test_get_file_tree_rejects_paths_outside_project_root() {
        let (_temp_dir, server) = setup_test_project();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.rs"), "fn secret() {}\n").unwrap();

        for path in [
            outside.path().display().to_string(),
            PathBuf::from("../").display().to_string(),
        ] {
            let args = json!({"path": path});
            let result = get_file_tree(&server, args.as_object()).await;
            assert!(result.is_err(), "outside path unexpectedly accepted");
        }
    }
}
