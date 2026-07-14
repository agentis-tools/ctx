//! Rust-specific code parsing using tree-sitter.

use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::db::{Edge, EdgeKind, ModuleInfo, ParseResult, Symbol, SymbolKind, Visibility};
use crate::parser::{
    extract_brief, extract_call_edges, find_symbol_kind, is_def_capture, CallCapturePatterns,
    SymbolKindMapping,
};

/// Symbol kind mappings for Rust capture names.
const RUST_SYMBOL_MAPPINGS: &[SymbolKindMapping] = &[
    SymbolKindMapping::new("func", SymbolKind::Function),
    SymbolKindMapping::new("method", SymbolKind::Method),
    SymbolKindMapping::new("struct", SymbolKind::Struct),
    SymbolKindMapping::new("enum", SymbolKind::Enum),
    SymbolKindMapping::new("trait", SymbolKind::Trait),
    SymbolKindMapping::new("type", SymbolKind::Type),
    SymbolKindMapping::new("const", SymbolKind::Const),
    SymbolKindMapping::new("static", SymbolKind::Static),
    SymbolKindMapping::new("macro", SymbolKind::Macro),
    SymbolKindMapping::new("mod", SymbolKind::Module),
];

/// Rust-specific parser.
pub struct RustParser {
    parser: Parser,
    symbols_query: Query,
    calls_query: Query,
    impl_query: Query,
}

impl RustParser {
    /// Create a new Rust parser.
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser
            .set_language(&language)
            .expect("Failed to set Rust language");

        // Query for extracting trait implementations
        let impl_query = Query::new(
            &language,
            r#"
            ; Trait implementations: impl Trait for Type
            (impl_item
                trait: (type_identifier) @trait.name
                type: (type_identifier) @type.name
            ) @impl.def
            "#,
        )
        .expect("Invalid impl query");

        // Query for extracting symbols (functions, structs, enums, etc.)
        // Note: We only match top-level functions (not inside impl blocks) and
        // methods inside impl blocks separately to avoid duplicate symbols.
        let symbols_query = Query::new(
            &language,
            r#"
            ; Top-level functions only (not inside impl blocks)
            ; Using a negated pattern to exclude functions in impl blocks
            (source_file
                (function_item
                    name: (identifier) @func.name
                    parameters: (parameters) @func.params
                    return_type: (_)? @func.return
                ) @func.def
            )

            ; Functions inside mod blocks (but not impl blocks)
            (mod_item
                body: (declaration_list
                    (function_item
                        name: (identifier) @func.name
                        parameters: (parameters) @func.params
                        return_type: (_)? @func.return
                    ) @func.def
                )
            )

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
            impl_query,
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

        // Extract trait implementation edges
        self.extract_impl_edges(&root, file_path, source, &symbols, &mut edges);

        let module = ModuleInfo {
            file_path: file_path.to_string(),
            module_name: super::extract_module_name(file_path, &["mod", "lib", "main"]),
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
        let mut matches = cursor.matches(&self.symbols_query, *root, source.as_bytes());

        while let Some(m) = matches.next() {
            let mut name: Option<&str> = None;
            let mut kind: Option<SymbolKind> = None;
            let mut def_node: Option<Node> = None;
            let mut signature_parts: Vec<&str> = Vec::new();
            let mut parent_type: Option<&str> = None;

            for capture in m.captures {
                let capture_name = self.symbols_query.capture_names()[capture.index as usize];
                let capture_str = capture_name;
                let node = capture.node;
                let text = node.utf8_text(source.as_bytes()).unwrap_or("");

                // Try to match standard symbol patterns
                if let Some(k) = find_symbol_kind(capture_str, RUST_SYMBOL_MAPPINGS) {
                    name = Some(text);
                    kind = Some(k);
                } else {
                    // Handle special cases
                    match capture_str {
                        // Signature parts
                        "func.params" | "method.params" => signature_parts.push(text),
                        "func.return" | "method.return" => signature_parts.push(text),
                        // Parent type for methods
                        "impl.type" => parent_type = Some(text),
                        // Use statements
                        "use.path" => {
                            let import = parse_use_path(text);
                            imports.push(import);
                        }
                        // Generic .def captures
                        _ if is_def_capture(capture_str) => {
                            def_node = Some(node);
                        }
                        _ => {}
                    }
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
        extract_call_edges(
            &self.calls_query,
            root,
            source,
            symbols,
            edges,
            &CallCapturePatterns::RUST,
        );
    }

    /// Extract trait implementation edges from the AST.
    fn extract_impl_edges(
        &self,
        root: &Node,
        _file_path: &str,
        source: &str,
        symbols: &[Symbol],
        edges: &mut Vec<Edge>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.impl_query, *root, source.as_bytes());

        while let Some(m) = matches.next() {
            let mut trait_name: Option<&str> = None;
            let mut type_name: Option<&str> = None;
            let mut def_node: Option<Node> = None;

            for capture in m.captures {
                let capture_name = self.impl_query.capture_names()[capture.index as usize];
                let node = capture.node;
                let text = node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "trait.name" => {
                        trait_name = Some(text);
                    }
                    "type.name" => {
                        type_name = Some(text);
                    }
                    "impl.def" => {
                        def_node = Some(node);
                    }
                    _ => {}
                }
            }

            if let (Some(trait_name), Some(type_name), Some(node)) =
                (trait_name, type_name, def_node)
            {
                let line = node.start_position().row as u32 + 1;
                let col = node.start_position().column as u32;

                // Find the type symbol - skip if not found (FK constraint requires valid source_id)
                let source_id = match symbols
                    .iter()
                    .find(|s| {
                        s.name == type_name
                            && matches!(s.kind, SymbolKind::Struct | SymbolKind::Enum)
                    })
                    .map(|s| s.id.clone())
                {
                    Some(id) => id,
                    None => continue, // Skip this edge if we can't find a valid source symbol
                };

                // Find the trait symbol
                let target_id = symbols
                    .iter()
                    .find(|s| s.name == trait_name && s.kind == SymbolKind::Trait)
                    .map(|s| s.id.clone());

                edges.push(Edge {
                    source_id,
                    target_id,
                    target_name: trait_name.to_string(),
                    kind: EdgeKind::Implements,
                    line: Some(line),
                    col: Some(col),
                    context: Some(format!("impl {} for {}", trait_name, type_name)),
                });
            }
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
                    doc_lines.push(super::parse_block_doc_comment(text));
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
        assert!(new_method
            .qualified_name
            .as_ref()
            .unwrap()
            .contains("Foo::new"));
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

    #[test]
    fn test_extract_impl_edges() {
        let mut parser = RustParser::new();
        let source = r#"
trait Animal {
    fn speak(&self);
}

struct Dog {
    name: String,
}

struct Cat {
    name: String,
}

impl Animal for Dog {
    fn speak(&self) {
        println!("Woof!");
    }
}

impl Animal for Cat {
    fn speak(&self) {
        println!("Meow!");
    }
}
"#;

        let result = parser.parse("test.rs", source).unwrap();

        let impl_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Implements)
            .collect();

        // Dog implements Animal, Cat implements Animal
        assert_eq!(impl_edges.len(), 2);

        assert!(impl_edges
            .iter()
            .any(|e| { e.source_id.contains("Dog") && e.target_name == "Animal" }));

        assert!(impl_edges
            .iter()
            .any(|e| { e.source_id.contains("Cat") && e.target_name == "Animal" }));
    }

    #[test]
    fn test_imports_stored_in_module() {
        // NOTE: Import edges are now stored in module.imports rather than as edges
        // because edges require a source_id that references an existing symbol (FK constraint)
        let mut parser = RustParser::new();
        let source = r#"
use std::collections::HashMap;
use std::io::{Read, Write};
"#;

        let result = parser.parse("test.rs", source).unwrap();

        // Imports should be in module info, not as edges
        let module = result.module.unwrap();
        assert!(
            !module.imports.is_empty(),
            "Expected imports in module info"
        );

        // Check we captured the imports
        let all_imports: Vec<_> = module.imports.iter().flat_map(|i| i.names.iter()).collect();
        assert!(all_imports
            .iter()
            .any(|n| n.contains("HashMap") || n.contains("*")));
    }
}
