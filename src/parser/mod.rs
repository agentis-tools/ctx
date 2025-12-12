//! Code parsing module using tree-sitter.
//!
//! This module extracts symbols and relationships from source code files
//! using tree-sitter grammars.

mod rust;

use std::path::Path;

use crate::db::ParseResult;

/// Supported programming languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Unknown,
}

impl Language {
    /// Detect language from file extension.
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some("rs") => Language::Rust,
            Some("ts") | Some("tsx") => Language::TypeScript,
            Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => Language::JavaScript,
            Some("py") | Some("pyi") => Language::Python,
            Some("go") => Language::Go,
            _ => Language::Unknown,
        }
    }

    /// Get the language name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
            Language::Unknown => "unknown",
        }
    }
}

/// Code parser that extracts symbols and relationships.
pub struct CodeParser {
    rust_parser: rust::RustParser,
}

impl CodeParser {
    /// Create a new code parser.
    pub fn new() -> Self {
        Self {
            rust_parser: rust::RustParser::new(),
        }
    }

    /// Parse a source file and extract symbols/edges.
    pub fn parse(&mut self, path: &Path, source: &str) -> Option<ParseResult> {
        let language = Language::from_path(path);
        let file_path = path.to_string_lossy().to_string();

        match language {
            Language::Rust => self.rust_parser.parse(&file_path, source),
            // TODO: Add other language parsers
            _ => {
                // Return a minimal result for unsupported languages
                Some(ParseResult {
                    file_path,
                    language: language.as_str().to_string(),
                    symbols: Vec::new(),
                    edges: Vec::new(),
                    module: None,
                })
            }
        }
    }

    /// Check if a language is supported for full parsing.
    pub fn is_supported(&self, path: &Path) -> bool {
        matches!(Language::from_path(path), Language::Rust)
    }
}

impl Default for CodeParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract a brief description from a docstring.
pub fn extract_brief(docstring: &str) -> Option<String> {
    let trimmed = docstring.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Take the first line or sentence
    let first_line = trimmed.lines().next()?;
    let brief = first_line.trim();

    // If it ends with a period, take it as-is
    if brief.ends_with('.') {
        return Some(brief.to_string());
    }

    // Otherwise, try to find the first sentence
    if let Some(idx) = brief.find(". ") {
        return Some(brief[..=idx].to_string());
    }

    Some(brief.to_string())
}

/// Get a snippet of context around a line.
pub fn get_context_snippet(source: &str, line: usize, col: usize) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    if line == 0 || line > lines.len() {
        return None;
    }

    let target_line = lines[line - 1];

    // Get a reasonable snippet (up to 80 chars)
    let start = col.saturating_sub(20);
    let end = (col + 60).min(target_line.len());

    let snippet = &target_line[start..end];
    Some(snippet.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_detection() {
        assert_eq!(Language::from_path(Path::new("main.rs")), Language::Rust);
        assert_eq!(Language::from_path(Path::new("app.ts")), Language::TypeScript);
        assert_eq!(Language::from_path(Path::new("script.js")), Language::JavaScript);
        assert_eq!(Language::from_path(Path::new("main.py")), Language::Python);
        assert_eq!(Language::from_path(Path::new("main.go")), Language::Go);
        assert_eq!(Language::from_path(Path::new("data.json")), Language::Unknown);
    }

    #[test]
    fn test_extract_brief() {
        // Multi-line extracts first line
        assert_eq!(
            extract_brief("This is a brief.\nMore details here."),
            Some("This is a brief.".to_string())
        );
        assert_eq!(
            extract_brief("Single line"),
            Some("Single line".to_string())
        );
        assert_eq!(extract_brief(""), None);
    }
}
