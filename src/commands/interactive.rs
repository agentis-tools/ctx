//! Interactive command implementations.
//!
//! Handles the interactive shell and MCP server.

use std::env;
use std::path::PathBuf;

use crate::shell;
use ctx::error::Result;

/// Run the interactive shell.
pub fn run_shell(history: Option<PathBuf>, no_history: bool, vi: bool) -> Result<()> {
    let root = env::current_dir()?;

    let mut config = shell::ShellConfig {
        db_path: root,
        no_history,
        vi_mode: vi,
        ..Default::default()
    };

    if let Some(h) = history {
        config.history_file = h;
    }

    shell::run_shell(config)
}

/// Run the MCP server.
#[cfg(feature = "mcp")]
pub fn run_serve(mcp: bool) -> Result<()> {
    use ctx::error::CtxError;
    use ctx::mcp;

    if !mcp {
        eprintln!("Error: Please specify --mcp flag to start the MCP server.");
        eprintln!("Usage: ctx serve --mcp");
        std::process::exit(1);
    }

    let root = env::current_dir()?;

    // Create a tokio runtime for the async MCP server
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async {
        mcp::run_mcp_server(root)
            .await
            .map_err(|e| CtxError::Other(e.to_string()))
    })
}
