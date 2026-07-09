//! ctx - Code intelligence library for AI-assisted development.
//!
//! This library powers the `ctx` CLI and can be embedded in your own tools.
//! It provides:
//!
//! - **Code Indexing**: Parse and index source code with symbol extraction
//! - **Semantic Search**: Find relevant code using embeddings
//! - **Call Graph Analysis**: Understand code relationships and impact
//! - **Smart Context Selection**: Intelligently select files for LLM context
//! - **Diff-Aware Context**: Generate context focused on code changes
//! - **Token Management**: Count and budget tokens for LLM context windows
//!
//! # Installation
//!
//! The package is published on crates.io as **`agentis-ctx`**, but the
//! library target is named `ctx`, so code imports use `ctx::`:
//!
//! ```toml
//! [dependencies]
//! agentis-ctx = "0.2"
//! ```
//!
//! Most of the commonly used types are re-exported through [`prelude`]:
//!
//! ```
//! use ctx::prelude::*;
//! ```
//!
//! # Quick Start: index a codebase and search it
//!
//! Indexing creates `.ctx/codebase.sqlite` under the project root. Subsequent
//! runs are incremental — only changed files are reparsed.
//!
//! ```no_run
//! use ctx::prelude::*;
//! use std::path::Path;
//!
//! fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
//!     let root = Path::new("./my-project");
//!
//!     // Build (or incrementally update) the index
//!     let mut indexer = Indexer::with_config(root, false, WalkerConfig::default())?;
//!     let result = indexer.index()?;
//!     println!(
//!         "Indexed {} files: {} symbols, {} edges",
//!         result.files_indexed, result.symbols_extracted, result.edges_extracted
//!     );
//!
//!     // Reopen the database later without reindexing
//!     let db = open_database(root)?;
//!
//!     // Keyword search over symbols (FTS5)
//!     for symbol in db.find_symbols("authenticate", 10)? {
//!         println!("{} ({}:{})", symbol.name, symbol.file_path, symbol.line_start);
//!     }
//!     Ok(())
//! }
//! ```
//!
//! # Smart context selection
//!
//! Select the files most relevant to a task description, within a token
//! budget — the same engine behind `ctx smart`:
//!
//! ```no_run
//! use ctx::prelude::*;
//! use std::path::Path;
//!
//! fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
//!     let root = Path::new("./my-project");
//!     let db = open_database(root)?;
//!     let analytics = Analytics::open(root)?;
//!
//!     // Local embedding model (downloaded on first use, ~90 MB).
//!     // OpenAIProvider is available as an alternative.
//!     let provider = LocalProvider::new()?;
//!
//!     let context = smart_context(
//!         &db,
//!         &analytics,
//!         &provider,
//!         "add rate limiting to the API",
//!         SmartConfig::default(),
//!     )?;
//!
//!     for file in &context.selected_files {
//!         println!("{} (relevance {:.2})", file.path, file.relevance_score);
//!     }
//!     println!("~{} tokens selected", context.total_tokens);
//!     Ok(())
//! }
//! ```
//!
//! # Token counting
//!
//! ```
//! use ctx::tokens::{count_tokens, count_tokens_with_encoding, Encoding};
//!
//! let n = count_tokens("fn main() { println!(\"hello\"); }").unwrap();
//! assert!(n > 0);
//!
//! let n = count_tokens_with_encoding("hello world", Encoding::O200kBase).unwrap();
//! assert!(n > 0);
//! ```
//!
//! # Module overview
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`index`] | Build and update the code intelligence index |
//! | [`db`] | SQLite storage: symbols, edges, files, FTS and vector search |
//! | [`parser`] | Tree-sitter based symbol/relationship extraction |
//! | [`embeddings`] | Embedding providers (local fastembed or OpenAI) |
//! | [`analytics`] | Call graph queries: callers, dependencies, impact (DuckDB) |
//! | [`smart`] | Task-driven file selection for LLM context |
//! | [`diff`] | Context generation focused on git changes |
//! | [`walker`] | File discovery with glob patterns and ignore rules |
//! | [`tokens`] | Token counting and budgeting (tiktoken) |
//! | [`formatter`], [`output`], [`tree`] | Rendering context as XML/Markdown/JSON/plain |
//! | [`audit`] | Code quality scoring for CI gates |
//!
//! # Feature Flags
//!
//! - `duckdb` *(enabled by default)* - DuckDB-backed analytics (call graphs,
//!   impact analysis, complexity). When disabled, [`analytics`] falls back to
//!   a stub that compiles everywhere (use this on Windows MSVC): add the
//!   dependency with `default-features = false`.
//! - `mcp` - Model Context Protocol server support for editor/agent
//!   integrations (see the `mcp` module).

// Core modules
pub mod analytics;
pub mod db;
pub mod embeddings;
pub mod error;
pub mod exit;
pub mod index;
pub mod json;
pub mod parser;
pub mod tokens;
pub mod walker;

// Context generation
pub mod diff;
pub mod smart;

// Output formatting
pub mod formatter;
pub mod output;
pub mod tree;

// Utilities
pub mod audit;
pub mod gitutil;
pub mod utils;

// Test helpers (public so the bin crate's tests can use them; not part of the
// supported API surface)
#[doc(hidden)]
pub mod testutil;

// Internal modules (not part of public API)
pub(crate) mod default_ignores;

// MCP server support (feature-gated)
#[cfg(feature = "mcp")]
pub mod mcp;

// Re-export commonly used types at the crate root
pub use error::{CtxError, Result};

/// Prelude module for convenient imports.
///
/// ```ignore
/// use ctx::prelude::*;
/// ```
pub mod prelude {
    pub use crate::analytics::Analytics;
    pub use crate::db::{Database, Edge, EdgeKind, FileRecord, ParseResult, Symbol, SymbolKind};
    pub use crate::diff::{diff_context, DiffConfig, DiffContext};
    pub use crate::embeddings::{Embedding, EmbeddingProvider, LocalProvider};
    pub use crate::error::{CtxError, Result};
    pub use crate::index::{open_database, IndexResult, Indexer};
    pub use crate::parser::{CodeParser, Language};
    pub use crate::smart::{smart_context, FileSelection, SmartConfig, SmartContext};
    pub use crate::tokens::{count_tokens, Encoding, HasTokenCount, TokenCount};
    pub use crate::walker::{discover_files, FileEntry, WalkerConfig};
}
