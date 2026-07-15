//! MCP Server implementation for ctx.

use std::path::PathBuf;
use std::sync::Mutex;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ErrorCode, Implementation, InitializeRequestParams,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities,
    ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::io::stdio;
use rmcp::{ServerHandler, ServiceExt};

use crate::analytics::Analytics;
use crate::db::Database;
use crate::index;

use super::tools;

/// MCP server for ctx code intelligence.
///
/// Uses Mutex wrappers to make Database and Analytics thread-safe.
/// Note: The MCP server is single-threaded per request, so contention is minimal.
pub struct CtxServer {
    /// Path to the project root
    root: PathBuf,
    /// Database connection (wrapped for thread safety)
    pub(crate) db: Mutex<Database>,
    /// Analytics engine (optional, wrapped for thread safety)
    pub(crate) analytics: Option<Mutex<Analytics>>,
}

impl CtxServer {
    /// Create a new CtxServer for the given project root.
    pub fn new(root: PathBuf) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Open the database
        let db = index::open_database(&root)
            .map_err(|e| format!("Failed to open database: {}. Run 'ctx index' first.", e))?;

        // Try to open analytics (optional)
        let analytics = Analytics::open(&root).ok().map(Mutex::new);

        Ok(Self {
            root,
            db: Mutex::new(db),
            analytics,
        })
    }

    /// Get the project root path.
    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    /// Execute a function with the database.
    pub fn with_db<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Database) -> R,
    {
        let db = self.db.lock().unwrap();
        f(&db)
    }

    /// Execute a function with analytics if available.
    #[allow(dead_code)]
    pub fn with_analytics<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&Analytics) -> R,
    {
        self.analytics.as_ref().map(|a| {
            let analytics = a.lock().unwrap();
            f(&analytics)
        })
    }
}

impl ServerHandler for CtxServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_server_info(Implementation::new("ctx", env!("CARGO_PKG_VERSION")))
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, rmcp::ErrorData> {
        Ok(self.get_info())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult {
            tools: tools::get_all_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let name = request.name.as_ref();
        let args = request.arguments.as_ref();

        match name {
            // Search tools
            "search_symbols" => tools::search::search_symbols(self, args).await,
            "get_definition" => tools::search::get_definition(self, args).await,
            "find_references" => tools::search::find_references(self, args).await,

            // File tools
            "get_file" => tools::files::get_file(self, args).await,
            "get_file_tree" => tools::files::get_file_tree(self, args).await,

            // Analysis tools
            "get_callers" => tools::analysis::get_callers(self, args).await,
            "get_callees" => tools::analysis::get_callees(self, args).await,
            "smart_context" => tools::analysis::smart_context(self, args).await,

            _ => Err(rmcp::ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("Unknown tool: {}", name),
                None,
            )),
        }
    }
}

/// Run the MCP server over stdio.
pub async fn run_mcp_server(root: PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = CtxServer::new(root)?;

    // Create stdio transport
    let transport = stdio();

    // Serve the MCP protocol
    let service = server.serve(transport).await?;

    // Wait for the service to complete
    service.waiting().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    use crate::index::Indexer;

    /// Create a test database with some symbols for testing.
    fn setup_test_project() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().to_path_buf();

        // Create a simple Rust file
        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            src_dir.join("lib.rs"),
            r#"
/// A test function.
pub fn hello_world() -> String {
    "Hello, World!".to_string()
}

/// Another function that calls hello_world.
pub fn greet() -> String {
    hello_world()
}
"#,
        )
        .unwrap();

        // Index the project using Indexer
        let mut indexer =
            Indexer::with_config(&root, false, crate::walker::WalkerConfig::default()).unwrap();
        indexer.index().unwrap();

        (temp_dir, root)
    }

    #[test]
    fn test_mcp_server_startup() {
        let (_temp_dir, root) = setup_test_project();

        // Create server - this should succeed with an indexed database
        let server =
            CtxServer::new(root.clone()).expect("Server should start with indexed database");

        // Verify server has correct root
        assert_eq!(server.root(), &root);

        // Verify we can access the database
        let symbol_count = server.with_db(|db| db.find_symbols("hello", 10).unwrap().len());
        assert!(symbol_count > 0, "Should find indexed symbols");
    }

    #[test]
    fn test_mcp_server_without_index_fails() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().to_path_buf();

        // Create an empty project without indexing
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "fn test() {}").unwrap();

        // Server creation should fail without index
        let result = CtxServer::new(root);
        assert!(
            result.is_err(),
            "Server should fail without indexed database"
        );
    }

    #[test]
    fn test_server_get_info() {
        let (_temp_dir, root) = setup_test_project();
        let server = CtxServer::new(root).unwrap();

        let info = server.get_info();
        assert_eq!(info.server_info.name.as_str(), "ctx");
        // Version is populated from CARGO_PKG_VERSION
        assert!(!info.server_info.version.is_empty());
    }
}
