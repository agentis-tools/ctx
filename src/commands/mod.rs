//! Command implementations for ctx CLI.
//!
//! This module contains the implementation of all CLI commands,
//! extracted from main.rs for better organization.

pub mod analysis;
pub mod check;
pub mod context;
pub mod diff_cmd;
pub mod embed;
pub mod graph;
pub mod index;
pub mod interactive;
pub mod query;
pub mod search;
pub mod smart_cmd;
pub mod symbol;

pub use analysis::{run_audit, run_complexity, run_duplicates};
pub use check::run_check;
pub use context::run_context;
pub use diff_cmd::{run_diff, run_review};
pub use embed::{run_embed, run_embed_watch, run_semantic};
pub use graph::run_graph;
pub use index::{run_index, IndexConfig};
#[cfg(feature = "mcp")]
pub use interactive::run_serve;
pub use interactive::run_shell;
pub use query::run_query;
pub use search::run_search;
pub use smart_cmd::run_smart;
pub use symbol::{run_explain, run_source};

/// Format a token count as a human-readable string (e.g. 241_502 → "241.5k").
pub fn format_token_count(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
