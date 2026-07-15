//! Code parsing module using tree-sitter.
//!
//! This module extracts symbols and relationships from source code files
//! using tree-sitter grammars.

mod c_cpp;
mod go;
mod python;
mod rust;
mod solidity;
mod typescript;
mod zig;

use std::path::Path;

use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};

use crate::db::{Edge, EdgeKind, ParseResult, Symbol, SymbolKind};

/// Supported programming languages.
///
/// Marked `#[non_exhaustive]`: new languages are appended here over time, so
/// downstream matches need a wildcard arm. Existing variants keep their
/// declaration order to hold their discriminants stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    Python,
    Go,
    Solidity,
    Yaml,
    Unknown,
    C,
    Cpp,
    Zig,
}

impl Language {
    /// Recover a language from its persisted index value.
    pub fn from_name(name: &str) -> Self {
        match name {
            "c" => Language::C,
            "cpp" => Language::Cpp,
            "rust" => Language::Rust,
            "typescript" => Language::TypeScript,
            "tsx" => Language::Tsx,
            "javascript" => Language::JavaScript,
            "jsx" => Language::Jsx,
            "python" => Language::Python,
            "go" => Language::Go,
            "zig" => Language::Zig,
            "solidity" => Language::Solidity,
            "yaml" => Language::Yaml,
            _ => Language::Unknown,
        }
    }

    /// Detect a language using source where the `.h` extension is ambiguous.
    pub fn from_path_and_source(path: &Path, source: &str) -> Self {
        if path.extension().and_then(|extension| extension.to_str()) == Some("h") {
            c_cpp::classify_header(source)
        } else {
            Self::from_path(path)
        }
    }

    /// Detect language from file extension.
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some("c") | Some("h") => Language::C,
            Some("cc") | Some("cpp") | Some("cxx") | Some("hh") | Some("hpp") | Some("hxx")
            | Some("ipp") | Some("tpp") => Language::Cpp,
            Some("rs") => Language::Rust,
            Some("ts") => Language::TypeScript,
            Some("tsx") => Language::Tsx,
            Some("js") | Some("mjs") | Some("cjs") => Language::JavaScript,
            Some("jsx") => Language::Jsx,
            Some("py") | Some("pyi") => Language::Python,
            Some("go") => Language::Go,
            Some("zig") => Language::Zig,
            Some("sol") => Language::Solidity,
            Some("yaml") | Some("yml") => Language::Yaml,
            _ => Language::Unknown,
        }
    }

    /// Get the language name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::JavaScript => "javascript",
            Language::Jsx => "jsx",
            Language::Python => "python",
            Language::Go => "go",
            Language::Zig => "zig",
            Language::Solidity => "solidity",
            Language::Yaml => "yaml",
            Language::Unknown => "unknown",
        }
    }
}

/// Code parser that extracts symbols and relationships.
pub struct CodeParser {
    c_cpp_parser: c_cpp::CCppParser,
    go_parser: go::GoParser,
    python_parser: python::PythonParser,
    rust_parser: rust::RustParser,
    solidity_parser: solidity::SolidityParser,
    typescript_parser: typescript::TypeScriptParser,
    zig_parser: zig::ZigParser,
}

impl CodeParser {
    /// Create a new code parser.
    pub fn new() -> Self {
        Self {
            c_cpp_parser: c_cpp::CCppParser::new(),
            go_parser: go::GoParser::new(),
            python_parser: python::PythonParser::new(),
            rust_parser: rust::RustParser::new(),
            solidity_parser: solidity::SolidityParser::new(),
            typescript_parser: typescript::TypeScriptParser::new(),
            zig_parser: zig::ZigParser::new(),
        }
    }

    /// Parse a source file and extract symbols/edges.
    pub fn parse(&mut self, path: &Path, source: &str) -> Option<ParseResult> {
        let language = Language::from_path(path);
        let file_path = path.to_string_lossy().to_string();

        match language {
            Language::C | Language::Cpp => self.c_cpp_parser.parse(&file_path, source, language),
            Language::Rust => self.rust_parser.parse(&file_path, source),
            Language::Solidity => self.solidity_parser.parse(&file_path, source),
            Language::TypeScript => {
                self.typescript_parser
                    .parse(&file_path, source, typescript::JsVariant::TypeScript)
            }
            Language::Tsx => {
                self.typescript_parser
                    .parse(&file_path, source, typescript::JsVariant::Tsx)
            }
            Language::JavaScript => {
                self.typescript_parser
                    .parse(&file_path, source, typescript::JsVariant::JavaScript)
            }
            Language::Jsx => {
                self.typescript_parser
                    .parse(&file_path, source, typescript::JsVariant::Jsx)
            }
            Language::Python => self.python_parser.parse(&file_path, source),
            Language::Go => self.go_parser.parse(&file_path, source),
            Language::Zig => self.zig_parser.parse(&file_path, source),
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
    /// Note: YAML is detected but not parsed (no tree-sitter grammar available).
    pub fn is_supported(&self, path: &Path) -> bool {
        Self::is_supported_static(path)
    }

    /// Static version of is_supported (for parallel indexing).
    pub fn is_supported_static(path: &Path) -> bool {
        matches!(
            Language::from_path(path),
            Language::Rust
                | Language::C
                | Language::Cpp
                | Language::Solidity
                | Language::TypeScript
                | Language::Tsx
                | Language::JavaScript
                | Language::Jsx
                | Language::Python
                | Language::Go
                | Language::Zig // Language::Yaml excluded - detected but not parsed
        )
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

/// Truncate a string to a maximum length, adding "..." if truncated.
pub fn truncate_context(s: &str, max_len: usize) -> String {
    let s = s.trim();
    if s.len() <= max_len {
        s.to_string()
    } else {
        // Find a valid UTF-8 char boundary for truncation
        let target = max_len.saturating_sub(3);
        let mut end = target;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Get a snippet of context around a line.
#[allow(dead_code)]
pub fn get_context_snippet(source: &str, line: usize, col: usize) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    if line == 0 || line > lines.len() {
        return None;
    }

    let target_line = lines[line - 1];

    // Get a reasonable snippet (up to 80 chars)
    // Find valid UTF-8 char boundaries
    let mut start = col.saturating_sub(20);
    while start > 0 && !target_line.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (col + 60).min(target_line.len());
    while end < target_line.len() && !target_line.is_char_boundary(end) {
        end += 1;
    }

    let snippet = &target_line[start..end];
    Some(snippet.trim().to_string())
}

/// Maps a capture name prefix (e.g., "func", "class") to a SymbolKind.
///
/// This is used to reduce the complexity of extract_symbols functions.
/// Capture names follow the pattern "prefix.name" or "prefix.def".
pub struct SymbolKindMapping {
    /// The prefix to match (e.g., "func", "class", "method")
    pub prefix: &'static str,
    /// The SymbolKind to assign when this prefix is matched
    pub kind: SymbolKind,
}

impl SymbolKindMapping {
    /// Create a new mapping from prefix to kind.
    pub const fn new(prefix: &'static str, kind: SymbolKind) -> Self {
        Self { prefix, kind }
    }
}

/// Find the SymbolKind for a capture name based on a list of mappings.
///
/// Returns Some(kind) if the capture name matches "prefix.name" for any mapping,
/// or None if no match is found.
pub fn find_symbol_kind(capture_name: &str, mappings: &[SymbolKindMapping]) -> Option<SymbolKind> {
    if !capture_name.ends_with(".name") {
        return None;
    }
    let prefix = capture_name.trim_end_matches(".name");
    mappings.iter().find(|m| m.prefix == prefix).map(|m| m.kind)
}

/// Check if a capture name is a definition capture (ends with ".def").
pub fn is_def_capture(capture_name: &str) -> bool {
    capture_name.ends_with(".def")
}

/// Capture name patterns for call extraction.
/// Each tuple contains (name_patterns, expr_patterns) that the query captures should match.
pub struct CallCapturePatterns {
    /// Patterns that match the function/method name being called
    pub name_patterns: &'static [&'static str],
    /// Patterns that match the call expression node
    pub expr_patterns: &'static [&'static str],
}

impl CallCapturePatterns {
    /// Standard patterns for Python (call.name, method_call.name)
    pub const STANDARD: CallCapturePatterns = CallCapturePatterns {
        name_patterns: &["call.name", "method_call.name"],
        expr_patterns: &["call.expr", "method_call.expr"],
    };

    /// Rust patterns include scoped calls (e.g., module::function())
    pub const RUST: CallCapturePatterns = CallCapturePatterns {
        name_patterns: &["call.name", "method_call.name", "scoped_call.name"],
        expr_patterns: &["call.expr", "method_call.expr", "scoped_call.expr"],
    };

    /// TypeScript/JavaScript patterns include new expressions (e.g., new Class())
    pub const TYPESCRIPT: CallCapturePatterns = CallCapturePatterns {
        name_patterns: &["call.name", "method_call.name", "new.name"],
        expr_patterns: &["call.expr", "method_call.expr", "new.expr"],
    };
}

/// Extract call edges from an AST using a tree-sitter query.
///
/// This is a shared helper for all language parsers. The query should capture:
/// - Call/method names with patterns like "call.name", "method_call.name"
/// - Call expressions with patterns like "call.expr", "method_call.expr"
pub fn extract_call_edges(
    query: &Query,
    root: &Node,
    source: &str,
    symbols: &[Symbol],
    edges: &mut Vec<Edge>,
    patterns: &CallCapturePatterns,
) {
    // Build a map of function ranges to their symbol IDs
    let func_ranges: Vec<_> = symbols
        .iter()
        .filter(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
        .map(|s| (s.line_start, s.line_end, s.id.clone()))
        .collect();

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, *root, source.as_bytes());

    while let Some(m) = matches.next() {
        let mut call_name: Option<&str> = None;
        let mut call_node: Option<Node> = None;

        for capture in m.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let node = capture.node;
            let text = node.utf8_text(source.as_bytes()).unwrap_or("");

            if patterns.name_patterns.contains(&capture_name) {
                call_name = Some(text);
            } else if patterns.expr_patterns.contains(&capture_name) {
                call_node = Some(node);
            }
        }

        if let (Some(name), Some(node)) = (call_name, call_node) {
            let line = node.start_position().row as u32 + 1;
            let col = node.start_position().column as u32;

            // Find which function this call is in
            let source_id = func_ranges
                .iter()
                .find(|(start, end, _)| line >= *start && line <= *end)
                .map(|(_, _, id)| id.clone());

            if let Some(source_id) = source_id {
                let context = node
                    .utf8_text(source.as_bytes())
                    .ok()
                    .map(|s| truncate_context(s, 80));

                // Try to resolve the target only if the context contains the qualified name
                // (e.g., "TypeScriptParser::new()").
                //
                // We don't resolve based on "unique name in file" because:
                // 1. Calls like "Vec::new()" or "Parser::new()" would match local "new" functions
                // 2. Cross-file resolution in post-indexing phase is more accurate
                //
                // Common names that are almost always external are left unresolved here.
                let target_id = if let Some(ctx) = &context {
                    symbols
                        .iter()
                        .find(|s| {
                            s.name == name
                                && s.qualified_name
                                    .as_ref()
                                    .map(|qn| ctx.contains(qn))
                                    .unwrap_or(false)
                        })
                        .map(|s| s.id.clone())
                } else {
                    None
                };

                edges.push(Edge {
                    source_id,
                    target_id,
                    target_name: name.to_string(),
                    kind: EdgeKind::Calls,
                    line: Some(line),
                    col: Some(col),
                    context,
                });
            }
        }
    }
}

/// Extract module name from a file path.
///
/// This is a shared helper for all language parsers. The `index_names` parameter
/// specifies which file stems should use the parent directory name instead
/// (e.g., "index" for TypeScript, "__init__" for Python, "mod" for Rust).
pub fn extract_module_name(file_path: &str, index_names: &[&str]) -> Option<String> {
    let path = std::path::Path::new(file_path);
    let stem = path.file_stem()?.to_str()?;

    if index_names.contains(&stem) {
        // Use parent directory name for index/entry files
        path.parent()?.file_name()?.to_str().map(String::from)
    } else {
        Some(stem.to_string())
    }
}

/// Parse a block doc comment (/** ... */ or /*! ... */) into clean content.
///
/// This is a shared helper for extracting doc comments in JSDoc, NatSpec, and Rust doc styles.
/// It strips the comment delimiters and leading asterisks from each line.
pub fn parse_block_doc_comment(text: &str) -> String {
    // Strip the opening delimiter (/** or /*!)
    let content = text
        .trim_start_matches("/**")
        .trim_start_matches("/*!")
        .trim_end_matches("*/");

    // Process each line: strip leading whitespace and asterisks
    content
        .lines()
        .map(|l| l.trim().trim_start_matches('*').trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_detection() {
        assert_eq!(Language::from_path(Path::new("main.c")), Language::C);
        assert_eq!(Language::from_path(Path::new("main.h")), Language::C);
        for extension in ["cc", "cpp", "cxx", "hh", "hpp", "hxx", "ipp", "tpp"] {
            assert_eq!(
                Language::from_path(Path::new(&format!("main.{extension}"))),
                Language::Cpp
            );
        }
        assert_eq!(Language::from_path(Path::new("main.rs")), Language::Rust);
        assert_eq!(
            Language::from_path(Path::new("app.ts")),
            Language::TypeScript
        );
        assert_eq!(Language::from_path(Path::new("App.tsx")), Language::Tsx);
        assert_eq!(
            Language::from_path(Path::new("script.js")),
            Language::JavaScript
        );
        assert_eq!(Language::from_path(Path::new("Button.jsx")), Language::Jsx);
        assert_eq!(Language::from_path(Path::new("main.py")), Language::Python);
        assert_eq!(Language::from_path(Path::new("main.go")), Language::Go);
        assert_eq!(Language::from_path(Path::new("main.zig")), Language::Zig);
        assert_eq!(
            Language::from_path(Path::new("Token.sol")),
            Language::Solidity
        );
        assert_eq!(
            Language::from_path(Path::new("config.yaml")),
            Language::Yaml
        );
        assert_eq!(Language::from_path(Path::new("ci.yml")), Language::Yaml);
        assert_eq!(
            Language::from_path(Path::new("data.json")),
            Language::Unknown
        );
    }

    #[test]
    fn test_zig_is_supported_for_index_and_watch_dispatch() {
        assert!(CodeParser::is_supported_static(Path::new("src/main.zig")));
        assert!(!CodeParser::is_supported_static(Path::new("build.zig.zon")));
    }

    #[test]
    fn test_c_cpp_are_supported_for_index_and_watch_dispatch() {
        for path in [
            "main.c", "main.h", "main.cc", "main.cpp", "main.cxx", "main.hh", "main.hpp",
            "main.hxx", "main.ipp", "main.tpp",
        ] {
            assert!(CodeParser::is_supported_static(Path::new(path)), "{path}");
        }
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

    #[test]
    fn test_truncate_context_ascii() {
        // Short string - no truncation
        assert_eq!(truncate_context("hello", 10), "hello");

        // Exact length - no truncation
        assert_eq!(truncate_context("hello", 5), "hello");

        // Needs truncation
        assert_eq!(truncate_context("hello world", 8), "hello...");

        // With leading/trailing whitespace
        assert_eq!(truncate_context("  hello world  ", 8), "hello...");
    }

    #[test]
    fn test_truncate_context_unicode() {
        // Box drawing characters (3 bytes each: ─ is \xe2\x94\x80)
        let box_line = "┌────────────────────────────────────────────────────────┐";

        // Should not panic on Unicode
        let result = truncate_context(box_line, 20);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 20);

        // Emoji (4 bytes each)
        let emoji_str = "Hello 🎉🎊🎁 World";
        let result = truncate_context(emoji_str, 12);
        assert!(result.ends_with("..."));

        // Mixed content
        let mixed = "console.log(\"├──────┤\")";
        let result = truncate_context(mixed, 15);
        assert!(result.ends_with("..."));

        // Chinese characters (3 bytes each)
        let chinese = "你好世界这是一个测试";
        let result = truncate_context(chinese, 10);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_context_edge_cases() {
        // Very short max_len
        assert_eq!(truncate_context("hello", 3), "...");
        assert_eq!(truncate_context("hi", 3), "hi");

        // Empty string
        assert_eq!(truncate_context("", 10), "");

        // Only whitespace
        assert_eq!(truncate_context("   ", 10), "");
    }

    #[test]
    fn test_get_context_snippet_unicode() {
        // Source with Unicode box drawing
        let source = "line1\nconsole.log(\"┌────────────────────┐\")\nline3";

        // Should not panic when getting snippet from line with Unicode
        let result = get_context_snippet(source, 2, 15);
        assert!(result.is_some());
    }
}
