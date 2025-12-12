//! Rust-specific code parsing using tree-sitter.

use tree_sitter::{Node, Parser, Query, QueryCursor};

use crate::db::{Edge, EdgeKind, ModuleInfo, ParseResult, Symbol, SymbolKind, Visibility};
use crate::parser::extract_brief;

/// Rust-specific parser.
pub struct RustParser {
    parser: Parser,
    symbols_query: Query,
    calls_query: Query,
}

impl RustParser {
    /// Create a new Rust parser.
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language = tree_sitter_rust::language();
        parser
            .set_language(&language)
            .expect("Failed to set Rust language");

        // Query for extracting symbols (functions, structs, enums, etc.)
        let symbols_query = Query::new(
            &language,
            r#"
            ; Functions
            (function_item
                name: (identifier) @func.name
                parameters: (parameters) @func.params
                return_type: (_)? @func.return
            ) @func.def

            ; Methods in impl blocks
            (impl_item
                type: (_) @impl.type
                body: (declaration_list
                    (function_item
                        name: (identifier) @method.name
                        parameters: (parameters) @method.params
                        return_type: (_)? @method.return
                    ) @method.def
                )
            ) @impl.def

            ; Structs
            (struct_item
                name: (type_identifier) @struct.name
            ) @struct.def

            ; Enums
            (enum_item
                name: (type_identifier) @enum.name
            ) @enum.def

            ; Traits
            (trait_item
                name: (type_identifier) @trait.name
            ) @trait.def

            ; Type aliases
            (type_item
                name: (type_identifier) @type.name
            ) @type.def

            ; Constants
            (const_item
                name: (identifier) @const.name
            ) @const.def

            ; Static items
            (static_item
                name: (identifier) @static.name
            ) @static.def

            ; Macro definitions
            (macro_definition
                name: (identifier) @macro.name
            ) @macro.def

            ; Module declarations
            (mod_item
                name: (identifier) @mod.name
            ) @mod.def

            ; Use statements (imports)
            (use_declaration
                argument: (_) @use.path
            ) @use.def
            "#,
        )
        .expect("Invalid symbols query");

        // Query for extracting function calls
        let calls_query = Query::new(
            &language,
            r#"
            ; Function calls
            (call_expression
                function: (identifier) @call.name
            ) @call.expr

            ; Method calls
            (call_expression
                function: (field_expression
                    field: (field_identifier) @method_call.name
                )
            ) @method_call.expr

            ; Scoped calls (e.g., Type::method())
            (call_expression
                function: (scoped_identifier
                    name: (identifier) @scoped_call.name
                )
            ) @scoped_call.expr
            "#,
        )
        .expect("Invalid calls query");

        Self {
            parser,
            symbols_query,
            calls_query,
        }
    }

    /// Parse a Rust source file.
    pub fn parse(&mut self, file_path: &str, source: &str) -> Option<ParseResult> {
        let tree = self.parser.parse(source, None)?;
        let root = tree.root_node();

        let mut symbols = Vec::new();
        let mut edges = Vec::new();
        let mut imports = Vec::new();
        let mut exports = Vec::new();

        // Extract symbols
        self.extract_symbols(
            &root,
            file_path,
            source,
            &mut symbols,
            &mut imports,
            &mut exports,
        );

        // Extract edges (calls, uses)
        self.extract_edges(&root, file_path, source, &symbols, &mut edges);

        let module = ModuleInfo {
            file_path: file_path.to_string(),
            module_name: self.extract_module_name(file_path),
            exports,
            imports,
        };

        Some(ParseResult {
            file_path: file_path.to_string(),
            language: "rust".to_string(),
            symbols,
            edges,
            module: Some(module),
        })
    }

    /// Extract symbols from the AST.
    fn extract_symbols(
        &self,
        root: &Node,
        file_path: &str,
        source: &str,
        symbols: &mut Vec<Symbol>,
        imports: &mut Vec<crate::db::ImportInfo>,
        exports: &mut Vec<String>,
    ) {
        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(&self.symbols_query, *root, source.as_bytes());

        for m in matches {
            let mut name: Option<&str> = None;
            let mut kind: Option<SymbolKind> = None;
            let mut def_node: Option<Node> = None;
            let mut signature_parts: Vec<&str> = Vec::new();
            let mut parent_type: Option<&str> = None;

            for capture in m.captures {
                let capture_name = self.symbols_query.capture_names()[capture.index as usize];
                let node = capture.node;
                let text = node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    // Functions
                    "func.name" => {
                        name = Some(text);
                        kind = Some(SymbolKind::Function);
                    }
                    "func.params" => signature_parts.push(text),
                    "func.return" => signature_parts.push(text),
                    "func.def" => def_node = Some(node),

                    // Methods
                    "method.name" => {
                        name = Some(text);
                        kind = Some(SymbolKind::Method);
                    }
                    "method.params" => signature_parts.push(text),
                    "method.return" => signature_parts.push(text),
                    "method.def" => def_node = Some(node),
                    "impl.type" => parent_type = Some(text),

                    // Structs
                    "struct.name" => {
                        name = Some(text);
                        kind = Some(SymbolKind::Struct);
                    }
                    "struct.def" => def_node = Some(node),

                    // Enums
                    "enum.name" => {
                        name = Some(text);
                        kind = Some(SymbolKind::Enum);
                    }
                    "enum.def" => def_node = Some(node),

                    // Traits
                    "trait.name" => {
                        name = Some(text);
                        kind = Some(SymbolKind::Trait);
                    }
                    "trait.def" => def_node = Some(node),

                    // Type aliases
                    "type.name" => {
                        name = Some(text);
                        kind = Some(SymbolKind::Type);
                    }
                    "type.def" => def_node = Some(node),

                    // Constants
                    "const.name" => {
                        name = Some(text);
                        kind = Some(SymbolKind::Const);
                    }
                    "const.def" => def_node = Some(node),

                    // Statics
                    "static.name" => {
                        name = Some(text);
                        kind = Some(SymbolKind::Static);
                    }
                    "static.def" => def_node = Some(node),

                    // Macros
                    "macro.name" => {
                        name = Some(text);
                        kind = Some(SymbolKind::Macro);
                    }
                    "macro.def" => def_node = Some(node),

                    // Modules
                    "mod.name" => {
                        name = Some(text);
                        kind = Some(SymbolKind::Module);
                    }
                    "mod.def" => def_node = Some(node),

                    // Use statements
                    "use.path" => {
                        let import = parse_use_path(text);
                        imports.push(import);
                    }

                    _ => {}
                }
            }

            // Create symbol if we have enough information
            if let (Some(name), Some(kind), Some(node)) = (name, kind, def_node) {
                let visibility = extract_visibility(&node, source);
                let docstring = extract_docstring(&node, source);
                let brief = docstring.as_ref().and_then(|d| extract_brief(d));

                let signature = build_signature(kind, name, &signature_parts, source, &node);

                let parent_id = parent_type.map(|p| Symbol::make_id(file_path, p, None));

                let symbol_source = node.utf8_text(source.as_bytes()).ok().map(String::from);

                let id = Symbol::make_id(file_path, name, parent_type);

                // Track exports (public symbols)
                if visibility == Visibility::Public {
                    exports.push(name.to_string());
                }

                symbols.push(Symbol {
                    id,
                    file_path: file_path.to_string(),
                    name: name.to_string(),
                    qualified_name: parent_type.map(|p| format!("{}::{}", p, name)),
                    kind,
                    visibility,
                    signature,
                    brief,
                    docstring,
                    line_start: node.start_position().row as u32 + 1,
                    line_end: node.end_position().row as u32 + 1,
                    col_start: node.start_position().column as u32,
                    col_end: node.end_position().column as u32,
                    parent_id,
                    source: symbol_source,
                });
            }
        }
    }

    /// Extract edges (function calls) from the AST.
    fn extract_edges(
        &self,
        root: &Node,
        _file_path: &str,
        source: &str,
        symbols: &[Symbol],
        edges: &mut Vec<Edge>,
    ) {
        // Build a map of function ranges to their symbol IDs
        let func_ranges: Vec<_> = symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
            .map(|s| {
                (
                    s.line_start,
                    s.line_end,
                    s.id.clone(),
                )
            })
            .collect();

        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(&self.calls_query, *root, source.as_bytes());

        for m in matches {
            let mut call_name: Option<&str> = None;
            let mut call_node: Option<Node> = None;

            for capture in m.captures {
                let capture_name = self.calls_query.capture_names()[capture.index as usize];
                let node = capture.node;
                let text = node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "call.name" | "method_call.name" | "scoped_call.name" => {
                        call_name = Some(text);
                    }
                    "call.expr" | "method_call.expr" | "scoped_call.expr" => {
                        call_node = Some(node);
                    }
                    _ => {}
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
                    // Try to resolve the target
                    let target_id = symbols
                        .iter()
                        .find(|s| s.name == name)
                        .map(|s| s.id.clone());

                    let context = node
                        .utf8_text(source.as_bytes())
                        .ok()
                        .map(|s| truncate_context(s, 80));

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

    /// Extract module name from file path.
    fn extract_module_name(&self, file_path: &str) -> Option<String> {
        let path = std::path::Path::new(file_path);
        let stem = path.file_stem()?.to_str()?;

        if stem == "mod" || stem == "lib" || stem == "main" {
            // Use parent directory name
            path.parent()?.file_name()?.to_str().map(String::from)
        } else {
            Some(stem.to_string())
        }
    }
}

/// Extract visibility from a node.
fn extract_visibility(node: &Node, source: &str) -> Visibility {
    // Look for visibility modifier as first child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = child.utf8_text(source.as_bytes()).unwrap_or("");
            return match text {
                "pub" => Visibility::Public,
                "pub(crate)" => Visibility::Crate,
                "pub(super)" => Visibility::Super,
                _ if text.starts_with("pub(in") => Visibility::InPath,
                _ => Visibility::Private,
            };
        }
    }
    Visibility::Private
}

/// Extract docstring from a node (looking for preceding comments).
fn extract_docstring(node: &Node, source: &str) -> Option<String> {
    let mut doc_lines = Vec::new();
    let mut prev = node.prev_sibling();

    // Collect consecutive doc comments before the node
    while let Some(sibling) = prev {
        match sibling.kind() {
            "line_comment" => {
                let text = sibling.utf8_text(source.as_bytes()).unwrap_or("");
                if text.starts_with("///") || text.starts_with("//!") {
                    let content = text
                        .trim_start_matches("///")
                        .trim_start_matches("//!")
                        .trim();
                    doc_lines.push(content.to_string());
                } else {
                    break;
                }
            }
            "block_comment" => {
                let text = sibling.utf8_text(source.as_bytes()).unwrap_or("");
                if text.starts_with("/**") || text.starts_with("/*!") {
                    let content = text
                        .trim_start_matches("/**")
                        .trim_start_matches("/*!")
                        .trim_end_matches("*/")
                        .trim();
                    doc_lines.push(content.to_string());
                }
                break;
            }
            _ => break,
        }
        prev = sibling.prev_sibling();
    }

    if doc_lines.is_empty() {
        return None;
    }

    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}

/// Build a signature string for a symbol.
fn build_signature(
    kind: SymbolKind,
    _name: &str,
    _parts: &[&str],
    source: &str,
    node: &Node,
) -> Option<String> {
    match kind {
        SymbolKind::Function | SymbolKind::Method => {
            // Get the function signature up to the body
            let text = node.utf8_text(source.as_bytes()).ok()?;
            // Find where the body starts (first '{')
            if let Some(idx) = text.find('{') {
                let sig = text[..idx].trim();
                Some(sig.to_string())
            } else {
                // No body, might be a trait method
                Some(text.lines().next()?.trim().to_string())
            }
        }
        SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait => {
            // Get the first line
            let text = node.utf8_text(source.as_bytes()).ok()?;
            let first_line = text.lines().next()?.trim();
            Some(first_line.trim_end_matches('{').trim().to_string())
        }
        _ => None,
    }
}

/// Parse a use path into ImportInfo.
fn parse_use_path(path: &str) -> crate::db::ImportInfo {
    // Handle various use patterns:
    // - use foo::bar;
    // - use foo::bar::{baz, qux};
    // - use foo::bar::*;
    // - use foo::bar as alias;

    let path = path.trim();

    // Check for alias
    if let Some(idx) = path.rfind(" as ") {
        let (path_part, alias) = path.split_at(idx);
        let alias = alias.trim_start_matches(" as ").trim();
        return crate::db::ImportInfo {
            from: path_part.trim().to_string(),
            names: vec![alias.to_string()],
            alias: Some(alias.to_string()),
        };
    }

    // Check for group imports
    if path.contains('{') {
        if let Some(idx) = path.find('{') {
            let base = path[..idx].trim_end_matches(':').trim();
            let names_part = path[idx..].trim_matches(|c| c == '{' || c == '}');
            let names: Vec<String> = names_part
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            return crate::db::ImportInfo {
                from: base.to_string(),
                names,
                alias: None,
            };
        }
    }

    // Check for glob import
    if path.ends_with("::*") {
        let base = path.trim_end_matches("::*");
        return crate::db::ImportInfo {
            from: base.to_string(),
            names: vec!["*".to_string()],
            alias: None,
        };
    }

    // Simple import
    let parts: Vec<&str> = path.rsplitn(2, "::").collect();
    if parts.len() == 2 {
        crate::db::ImportInfo {
            from: parts[1].to_string(),
            names: vec![parts[0].to_string()],
            alias: None,
        }
    } else {
        crate::db::ImportInfo {
            from: path.to_string(),
            names: Vec::new(),
            alias: None,
        }
    }
}

/// Truncate context to a maximum length.
fn truncate_context(s: &str, max_len: usize) -> String {
    let s = s.trim();
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

impl Default for RustParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_function() {
        let mut parser = RustParser::new();
        let source = r#"
/// This is a test function.
pub fn hello_world(name: &str) -> String {
    format!("Hello, {}!", name)
}
"#;

        let result = parser.parse("test.rs", source).unwrap();
        assert_eq!(result.symbols.len(), 1);

        let func = &result.symbols[0];
        assert_eq!(func.name, "hello_world");
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.visibility, Visibility::Public);
        assert!(func.signature.as_ref().unwrap().contains("fn hello_world"));
        assert_eq!(func.brief.as_ref().unwrap(), "This is a test function.");
    }

    #[test]
    fn test_parse_struct() {
        let mut parser = RustParser::new();
        let source = r#"
/// A point in 2D space.
pub struct Point {
    pub x: f64,
    pub y: f64,
}
"#;

        let result = parser.parse("test.rs", source).unwrap();
        assert_eq!(result.symbols.len(), 1);

        let s = &result.symbols[0];
        assert_eq!(s.name, "Point");
        assert_eq!(s.kind, SymbolKind::Struct);
        assert_eq!(s.visibility, Visibility::Public);
    }

    #[test]
    fn test_parse_impl_methods() {
        let mut parser = RustParser::new();
        let source = r#"
struct Foo;

impl Foo {
    pub fn new() -> Self {
        Foo
    }

    fn private_method(&self) {
    }
}
"#;

        let result = parser.parse("test.rs", source).unwrap();

        // Should have struct and 2 methods
        assert!(result.symbols.len() >= 2);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);

        let new_method = methods.iter().find(|m| m.name == "new").unwrap();
        assert_eq!(new_method.visibility, Visibility::Public);
        assert!(new_method.qualified_name.as_ref().unwrap().contains("Foo::new"));
    }

    #[test]
    fn test_extract_calls() {
        let mut parser = RustParser::new();
        let source = r#"
fn foo() {
    bar();
    baz();
}

fn bar() {}
fn baz() {}
"#;

        let result = parser.parse("test.rs", source).unwrap();

        let calls: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        assert_eq!(calls.len(), 2);
        assert!(calls.iter().any(|e| e.target_name == "bar"));
        assert!(calls.iter().any(|e| e.target_name == "baz"));
    }

    #[test]
    fn test_parse_use_path() {
        let import = parse_use_path("std::collections::HashMap");
        assert_eq!(import.from, "std::collections");
        assert_eq!(import.names, vec!["HashMap"]);

        let import = parse_use_path("std::io::{Read, Write}");
        assert_eq!(import.from, "std::io");
        assert_eq!(import.names, vec!["Read", "Write"]);

        let import = parse_use_path("std::prelude::*");
        assert_eq!(import.from, "std::prelude");
        assert_eq!(import.names, vec!["*"]);
    }
}
