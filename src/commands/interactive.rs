//! Interactive command implementations.
//!
//! Handles the interactive shell and MCP server.

use std::env;

use ctx::error::Result;
use crate::shell;

/// Run the interactive shell.
pub fn run_shell(
    history: Option<std::path::PathBuf>,
    no_history: bool,
    vi: bool,
) -> Result<()> {
    let root = env::current_dir()?;

    let mut config = shell::ShellConfig::default();
    config.db_path = root;
    config.no_history = no_history;
    config.vi_mode = vi;

    if let Some(h) = history {
        config.history_file = h;
    }

    shell::run_shell(config)
}

/// Run the MCP server.
#[cfg(feature = "mcp")]
pub fn run_serve(mcp: bool) -> Result<()> {
    use ctx::error::CtxError;
    use crate::mcp;

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
