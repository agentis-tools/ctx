//! MCP (Model Context Protocol) server implementation for ctx.
//!
//! This module exposes ctx's code intelligence capabilities via the MCP protocol,
//! allowing AI assistants like Claude to query codebases through standardized tools.
//!
//! # Available Tools
//!
//! - `search_symbols`: Search for symbols by name pattern
//! - `get_definition`: Get the source code for a symbol
//! - `find_references`: Find all references to a symbol
//! - `get_callers`: Get functions that call a given function
//! - `get_callees`: Get functions called by a given function
//! - `get_file`: Read a file's contents
//! - `get_file_tree`: List files in the project
//! - `smart_context`: Intelligently select files for a task
//!
//! # Usage
//!
//! Start the MCP server with:
//! ```bash
//! ctx serve --mcp
//! ```
//!
//! Configure Claude Desktop by adding to `claude_desktop_config.json`:
//! ```json
//! {
//!   "mcpServers": {
//!     "ctx": {
//!       "command": "ctx",
//!       "args": ["serve", "--mcp"],
//!       "cwd": "/path/to/your/project"
//!     }
//!   }
//! }
//! ```

pub mod server;
pub mod tools;

pub use server::{run_mcp_server, CtxServer};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_module_exports() {
        // Basic sanity check that the module compiles and exports work
        let _ = std::any::TypeId::of::<CtxServer>();
    }
}
