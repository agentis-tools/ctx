//! Unified error type for ctx operations.
//!
//! This module provides a single error type that consolidates all error handling
//! across the codebase, replacing module-specific error types like `EmbeddingError`,
//! `SmartError`, `DiffError`, and `AuditError`.

use thiserror::Error;

/// Unified error type for all ctx operations.
#[derive(Error, Debug)]
pub enum CtxError {
    // ========== Database Errors ==========
    /// SQLite database error.
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// DuckDB analytics error.
    #[cfg(feature = "duckdb")]
    #[error("Analytics error: {0}")]
    Analytics(#[from] duckdb::Error),

    // ========== IO Errors ==========
    /// File system or IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Readline/shell input error.
    #[error("Readline error: {0}")]
    Readline(#[from] rustyline::error::ReadlineError),

    // ========== Serialization Errors ==========
    /// JSON serialization/deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    // ========== Network Errors ==========
    /// HTTP request error.
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    // ========== Embedding Errors ==========
    /// Generic embedding operation error.
    #[error("Embedding error: {0}")]
    Embedding(String),

    /// API rate limit exceeded.
    #[error("Rate limited: retry after {0} seconds")]
    RateLimited(u64),

    /// Invalid or missing API key.
    #[error("Invalid API key")]
    InvalidApiKey,

    /// Embedding model not found.
    #[error("Model not found: {0}")]
    ModelNotFound(String),

    /// Vector dimension mismatch.
    #[error("Dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    // ========== Git/Diff Errors ==========
    /// Git command or operation error.
    #[error("Git error: {0}")]
    Git(String),

    /// Not inside a git repository.
    #[error("Not a git repository")]
    NotGitRepo,

    /// Invalid git revision or reference.
    #[error("Invalid revision: {0}")]
    InvalidRevision(String),

    /// No changes found in diff.
    #[error("No changes found")]
    NoChanges,

    /// The revision has changes, but none match the requested path scope.
    ///
    /// Distinct from [`CtxError::NoChanges`]: an empty scope is a legitimate
    /// answer to a narrower question, not a failure to answer it.
    #[error("No changes found in scope")]
    NoChangesInScope,

    // ========== Business Logic Errors ==========
    /// No relevant files found for smart context.
    #[error("No relevant files found")]
    NoMatches,

    /// Index database not found.
    #[error("Index not found: {0}")]
    #[allow(dead_code)] // Part of public API for future use
    IndexNotFound(String),

    /// The on-disk index was created with a different schema version.
    #[error("Index schema version mismatch (found v{found}, expected v{expected}). Run 'ctx index --force' to rebuild the index.")]
    SchemaVersionMismatch { found: i64, expected: i64 },

    /// Parse error during code analysis.
    #[error("Parse error: {0}")]
    #[allow(dead_code)] // Part of public API for future use
    ParseError(String),

    /// Token counting error.
    #[error("Token counting error: {0}")]
    #[allow(dead_code)] // Part of public API for future use
    TokenCount(String),

    /// File read error with path context.
    #[error("Failed to read file '{path}': {message}")]
    #[allow(dead_code)] // Part of public API for future use
    FileRead { path: String, message: String },

    // ========== Generic Errors ==========
    /// Generic error for cases not covered above.
    #[error("{0}")]
    Other(String),
}

/// Result type alias using CtxError.
pub type Result<T> = std::result::Result<T, CtxError>;

impl CtxError {
    /// Create an embedding error from a string message.
    pub fn embedding(msg: impl Into<String>) -> Self {
        CtxError::Embedding(msg.into())
    }

    /// Create a git error from a string message.
    pub fn git(msg: impl Into<String>) -> Self {
        CtxError::Git(msg.into())
    }

    /// Create a parse error from a string message.
    #[allow(dead_code)] // Part of public API for future use
    pub fn parse(msg: impl Into<String>) -> Self {
        CtxError::ParseError(msg.into())
    }

    /// Create a token count error from a string message.
    #[allow(dead_code)] // Part of public API for future use
    pub fn token_count(msg: impl Into<String>) -> Self {
        CtxError::TokenCount(msg.into())
    }

    /// Create a file read error with path context.
    #[allow(dead_code)] // Part of public API for future use
    pub fn file_read(path: impl Into<String>, msg: impl Into<String>) -> Self {
        CtxError::FileRead {
            path: path.into(),
            message: msg.into(),
        }
    }

    /// Create an "other" error from a string message.
    #[allow(dead_code)] // Part of public API for future use
    pub fn other(msg: impl Into<String>) -> Self {
        CtxError::Other(msg.into())
    }
}

// ========== Convenience Conversions ==========

impl From<String> for CtxError {
    fn from(s: String) -> Self {
        CtxError::Other(s)
    }
}

impl From<&str> for CtxError {
    fn from(s: &str) -> Self {
        CtxError::Other(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = CtxError::Database(rusqlite::Error::InvalidQuery);
        assert!(err.to_string().contains("Database error"));

        let err = CtxError::DimensionMismatch {
            expected: 1536,
            actual: 384,
        };
        assert_eq!(
            err.to_string(),
            "Dimension mismatch: expected 1536, got 384"
        );

        let err = CtxError::RateLimited(60);
        assert_eq!(err.to_string(), "Rate limited: retry after 60 seconds");
    }

    #[test]
    fn test_error_constructors() {
        let err = CtxError::embedding("test error");
        assert!(matches!(err, CtxError::Embedding(_)));

        let err = CtxError::git("not a repo");
        assert!(matches!(err, CtxError::Git(_)));

        let err = CtxError::file_read("/path/to/file", "permission denied");
        assert!(matches!(err, CtxError::FileRead { .. }));
    }

    #[test]
    fn test_from_string() {
        let err: CtxError = "some error".into();
        assert!(matches!(err, CtxError::Other(_)));

        let err: CtxError = String::from("another error").into();
        assert!(matches!(err, CtxError::Other(_)));
    }
}
