//! Analytics module using DuckDB for complex graph queries.
//!
//! This module is gated behind the `duckdb` feature flag. When disabled,
//! a stub implementation is provided so the crate can still compile and
//! run on platforms where DuckDB's bundled C++ library cannot build
//! (e.g. Windows MSVC).

use serde::{Deserialize, Serialize};

// ============================================================================
// Data structures (always available)
// ============================================================================

/// A node in a call graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraphNode {
    pub name: String,
    pub file_path: String,
    pub kind: String,
    pub depth: i32,
}

/// A node in impact analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactNode {
    pub name: String,
    pub file_path: String,
    pub kind: String,
    pub distance: i32,
}

/// A node in impact analysis with its indexed source location.
///
/// This complements [`ImpactNode`] without changing that existing public type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocatedImpactNode {
    pub symbol_id: String,
    pub name: String,
    pub qualified_name: Option<String>,
    pub file_path: String,
    pub kind: String,
    pub line_start: i64,
    pub line_end: i64,
    pub distance: i32,
}

/// Complexity analysis result for a function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityResult {
    pub name: String,
    pub file_path: String,
    pub line: u32,
    pub fan_out: i64,
    pub fan_in: i64,
    pub complexity_score: i64,
    pub severity: String,
}

/// Module dependency information.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ModuleDep {
    pub source_module: String,
    pub target_module: String,
    pub imported_names: Vec<String>,
}

/// File statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStats {
    pub file_path: String,
    pub symbol_count: i64,
    pub functions: i64,
    pub structs: i64,
    pub enums: i64,
    pub public_symbols: i64,
}

// ============================================================================
// Platform-specific Analytics engine
// ============================================================================

#[cfg(feature = "duckdb")]
mod duckdb_impl;

#[cfg(not(feature = "duckdb"))]
mod stub;

#[cfg(feature = "duckdb")]
pub use duckdb_impl::Analytics;

#[cfg(feature = "duckdb")]
pub use duckdb_impl::{SqlColumn, SqlResult};

#[cfg(not(feature = "duckdb"))]
pub use stub::Analytics;
