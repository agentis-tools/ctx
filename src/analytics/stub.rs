//! Stub analytics implementation used when the `duckdb` feature is disabled.
//!
//! This avoids pulling in the DuckDB C++ dependency on platforms where it
//! cannot compile (e.g. Windows MSVC). The public method set remains available
//! so callers can compile against either build, while runtime callers receive
//! an actionable error instead of mistaking unavailable analytics for no data.

use std::path::Path;

use super::{CallGraphNode, ComplexityResult, FileStats, ImpactNode, LocatedImpactNode};
use crate::error::{CtxError, Result};

const UNAVAILABLE: &str = "DuckDB analytics are unavailable in this build; reinstall ctx with default features or build with --features duckdb";

fn unavailable<T>() -> Result<T> {
    Err(CtxError::Other(UNAVAILABLE.to_string()))
}

/// Stub analytics engine retained only to keep the no-DuckDB API compilable.
pub struct Analytics;

impl Analytics {
    /// Reject analytics operations with an actionable build-feature error.
    pub fn open(_root: &Path) -> Result<Self> {
        unavailable()
    }

    /// Call graph is unavailable without DuckDB.
    pub fn call_graph(&self, _start_name: &str, _max_depth: i32) -> Result<Vec<CallGraphNode>> {
        unavailable()
    }

    /// Impact analysis is unavailable without DuckDB.
    pub fn impact_analysis(&self, _target_name: &str, _max_depth: i32) -> Result<Vec<ImpactNode>> {
        unavailable()
    }

    /// Located impact analysis is unavailable without DuckDB.
    pub fn impact_analysis_located(
        &self,
        _target_name: &str,
        _max_depth: i32,
    ) -> Result<Vec<LocatedImpactNode>> {
        unavailable()
    }

    /// File statistics are unavailable without DuckDB.
    pub fn file_statistics(&self) -> Result<Vec<FileStats>> {
        unavailable()
    }

    /// Symbol summaries are unavailable without DuckDB.
    #[allow(dead_code)]
    pub fn symbol_summary(&self) -> Result<Vec<(String, i64, i64)>> {
        unavailable()
    }

    /// Path queries are unavailable without DuckDB.
    #[allow(dead_code)]
    pub fn has_path(&self, _from_name: &str, _to_name: &str, _max_depth: i32) -> Result<bool> {
        unavailable()
    }

    /// Connectivity queries are unavailable without DuckDB.
    pub fn most_connected(&self, _limit: i32) -> Result<Vec<(String, String, i64, i64)>> {
        unavailable()
    }

    /// Recursive-function queries are unavailable without DuckDB.
    #[allow(dead_code)]
    pub fn find_recursive_functions(&self) -> Result<Vec<(String, String)>> {
        unavailable()
    }

    /// File-dependency queries are unavailable without DuckDB.
    pub fn file_dependencies(&self) -> Result<Vec<(String, String, i64)>> {
        unavailable()
    }

    /// Complexity analysis is unavailable without DuckDB.
    pub fn complexity_analysis(&self, _threshold: i64) -> Result<Vec<ComplexityResult>> {
        unavailable()
    }

    /// Full call graphs are unavailable without DuckDB.
    pub fn full_call_graph(
        &self,
        _max_depth: i32,
    ) -> Result<Vec<(String, String, String, String)>> {
        unavailable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytics_unavailable_is_actionable() {
        let err = Analytics::open(Path::new("."))
            .err()
            .expect("stub analytics must be unavailable");
        assert!(err.to_string().contains("DuckDB analytics are unavailable"));
        assert!(err.to_string().contains("default features"));
    }
}
