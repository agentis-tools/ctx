//! Stub analytics implementation used when the `duckdb` feature is disabled.
//!
//! This avoids pulling in the DuckDB C++ dependency on platforms where it
//! cannot compile (e.g. Windows MSVC). All methods return empty results so
//! analytics-dependent commands degrade gracefully.

use std::path::Path;

use super::{CallGraphNode, ComplexityResult, FileStats, ImpactNode, LocatedImpactNode};
use crate::error::Result;

/// Stub analytics engine that returns empty results.
pub struct Analytics;

impl Analytics {
    /// Create a stub (always succeeds).
    pub fn open(_root: &Path) -> Result<Self> {
        Ok(Analytics)
    }

    /// Call graph: returns empty list.
    pub fn call_graph(&self, _start_name: &str, _max_depth: i32) -> Result<Vec<CallGraphNode>> {
        Ok(Vec::new())
    }

    /// Impact analysis: returns empty list.
    pub fn impact_analysis(&self, _target_name: &str, _max_depth: i32) -> Result<Vec<ImpactNode>> {
        Ok(Vec::new())
    }

    /// Located impact analysis: returns empty list.
    pub fn impact_analysis_located(
        &self,
        _target_name: &str,
        _max_depth: i32,
    ) -> Result<Vec<LocatedImpactNode>> {
        Ok(Vec::new())
    }

    /// File statistics: returns empty list.
    pub fn file_statistics(&self) -> Result<Vec<FileStats>> {
        Ok(Vec::new())
    }

    /// Symbol summary: returns empty list.
    #[allow(dead_code)]
    pub fn symbol_summary(&self) -> Result<Vec<(String, i64, i64)>> {
        Ok(Vec::new())
    }

    /// Path existence check: always returns false.
    #[allow(dead_code)]
    pub fn has_path(&self, _from_name: &str, _to_name: &str, _max_depth: i32) -> Result<bool> {
        Ok(false)
    }

    /// Most connected symbols: returns empty list.
    pub fn most_connected(&self, _limit: i32) -> Result<Vec<(String, String, i64, i64)>> {
        Ok(Vec::new())
    }

    /// Recursive functions: returns empty list.
    #[allow(dead_code)]
    pub fn find_recursive_functions(&self) -> Result<Vec<(String, String)>> {
        Ok(Vec::new())
    }

    /// File dependencies: returns empty list.
    pub fn file_dependencies(&self) -> Result<Vec<(String, String, i64)>> {
        Ok(Vec::new())
    }

    /// Complexity analysis: returns empty list.
    pub fn complexity_analysis(&self, _threshold: i64) -> Result<Vec<ComplexityResult>> {
        Ok(Vec::new())
    }

    /// Full call graph: returns empty list.
    pub fn full_call_graph(
        &self,
        _max_depth: i32,
    ) -> Result<Vec<(String, String, String, String)>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn located_impact_analysis_returns_empty_data() {
        let analytics = Analytics::open(Path::new(".")).unwrap();
        assert!(analytics
            .impact_analysis_located("anything", 5)
            .unwrap()
            .is_empty());
    }
}
