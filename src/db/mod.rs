//! Database module for code intelligence storage.
//!
//! This module provides SQLite-based storage for:
//! - File tracking with content hashes
//! - Symbol information (functions, structs, enums, etc.)
//! - Relationships between symbols (calls, imports, types)
//! - Module-level information

pub mod models;
pub mod schema;

pub use models::*;
pub use schema::{
    CrossFileEdge, Database, EdgeSymbol, FileComplexity, MapSymbolRow, SymbolMetrics,
    SCHEMA_VERSION,
};
