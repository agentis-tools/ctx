//! Shared resource limits for operations that inspect repository-controlled data.
//!
//! These limits are deliberately conservative enough for ordinary repositories
//! while preventing a single checkout or MCP request from turning an analysis
//! operation into an unbounded allocation or traversal.

/// Maximum number of files accepted by one discovery pass.
pub const MAX_DISCOVERED_FILES: usize = 100_000;

/// Maximum size of one file read by the repository walker.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Maximum aggregate size of files accepted by one discovery pass.
pub const MAX_DISCOVERED_BYTES: u64 = 256 * 1024 * 1024;

/// Maximum number of search results returned by CLI/database APIs.
pub const MAX_SEARCH_RESULTS: i32 = 10_000;

/// Maximum number of search results returned by MCP tools.
pub const MAX_MCP_SEARCH_RESULTS: i32 = 1_000;

/// Maximum graph traversal depth accepted at public boundaries.
pub const MAX_GRAPH_DEPTH: i32 = 64;

/// Maximum UTF-8 response size returned by an MCP tool.
pub const MAX_MCP_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// Clamp a caller-provided search limit to a safe, useful range.
///
/// A zero or negative limit historically had backend-specific behavior (SQLite
/// treats a negative LIMIT as unlimited). Treat it as one result instead so
/// every caller gets a bounded query.
pub fn clamp_search_limit(limit: i32) -> i32 {
    limit.clamp(1, MAX_SEARCH_RESULTS)
}

/// Clamp a graph depth while preserving zero as the explicit "no expansion"
/// value used by the query commands.
pub fn clamp_graph_depth(depth: i32) -> i32 {
    depth.clamp(0, MAX_GRAPH_DEPTH)
}

/// Cap an MCP response without splitting a UTF-8 code point.
pub fn cap_mcp_response(mut output: String) -> String {
    if output.len() <= MAX_MCP_RESPONSE_BYTES {
        return output;
    }

    const MARKER: &str = "\n\n[response truncated by ctx resource limit]";
    let mut end = MAX_MCP_RESPONSE_BYTES.saturating_sub(MARKER.len());
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    output.truncate(end);
    output.push_str(MARKER);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_limits_are_always_positive_and_bounded() {
        assert_eq!(clamp_search_limit(-10), 1);
        assert_eq!(clamp_search_limit(0), 1);
        assert_eq!(clamp_search_limit(10), 10);
        assert_eq!(clamp_search_limit(i32::MAX), MAX_SEARCH_RESULTS);
    }

    #[test]
    fn graph_depth_is_bounded_but_zero_is_preserved() {
        assert_eq!(clamp_graph_depth(-1), 0);
        assert_eq!(clamp_graph_depth(0), 0);
        assert_eq!(clamp_graph_depth(10), 10);
        assert_eq!(clamp_graph_depth(i32::MAX), MAX_GRAPH_DEPTH);
    }

    #[test]
    fn mcp_response_cap_preserves_utf8() {
        let output = cap_mcp_response("🙂".repeat(MAX_MCP_RESPONSE_BYTES));
        assert!(output.len() <= MAX_MCP_RESPONSE_BYTES);
        assert!(output.ends_with("[response truncated by ctx resource limit]"));
    }
}
