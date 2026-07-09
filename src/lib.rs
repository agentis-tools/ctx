//! ctx - Code intelligence library for AI-assisted development.
//!
//! This library provides tools for understanding codebases and generating
//! context for Large Language Models (LLMs). It includes:
//!
//! - **Code Indexing**: Parse and index source code with symbol extraction
//! - **Semantic Search**: Find relevant code using embeddings
//! - **Call Graph Analysis**: Understand code relationships and impact
//! - **Smart Context Selection**: Intelligently select files for LLM context
//! - **Diff-Aware Context**: Generate context focused on code changes
//! - **Token Management**: Count and budget tokens for LLM context windows
//!
//! # Quick Start
//!
//! ```ignore
//! use ctx::{index::Indexer, db::Database, smart::{smart_context, SmartConfig}};
//! use ctx::embeddings::LocalProvider;
//!
//! // Index a codebase
//! let mut indexer = Indexer::new("./my-project", false)?;
//! indexer.index()?;
//!
//! // Open the database
//! let db = index::open_database("./my-project")?;
//!
//! // Generate smart context for a task
//! let provider = LocalProvider::new()?;
//! let config = SmartConfig::default();
//! let context = smart_context(&db, &analytics, &provider, "add caching", config)?;
//! ```
//!
//! # Feature Flags
//!
//! - `mcp` - Enable Model Context Protocol server support

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

// Architecture rules (ctx check)
pub mod rules;

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
