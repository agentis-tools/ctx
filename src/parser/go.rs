//! Go-specific code parsing using tree-sitter.

use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::db::{Edge, EdgeKind, ModuleInfo, ParseResult, Symbol, SymbolKind, Visibility};
use crate::parser::{
    extract_brief, extract_call_edges, find_symbol_kind, is_def_capture, CallCapturePatterns,
    SymbolKindMapping,
};

/// Symbol kind mappings for Go capture names.
const GO_SYMBOL_MAPPINGS: &[SymbolKindMapping] = &[
    SymbolKindMapping::new("func", SymbolKind::Function),
    SymbolKindMapping::new("method", SymbolKind::Method),
    SymbolKindMapping::new("struct", SymbolKind::Struct),
    SymbolKindMapping::new("interface", SymbolKind::Interface),
    SymbolKindMapping::new("type", SymbolKind::Type),
    SymbolKindMapping::new("const", SymbolKind::Const),
    SymbolKindMapping::new("var", SymbolKind::Variable),
];

/// Go-specific parser.
pub struct GoParser {
    parser: Parser,
    symbols_query: Query,
    calls_query: Query,
}

impl GoParser {
    /// Create a new Go parser.
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
        parser
            .set_language(&language)
            .expect("Failed to set Go language");

        // Query for extracting symbols (functions, structs, interfaces, etc.)
        let symbols_query = Query::new(
            &language,
            r#"
            ; Top-level functions
            (function_declaration
                name: (identifier) @func.name
            ) @func.def

            ; Methods (functions with receivers)
            (method_declaration
                receiver: (parameter_list
                    (parameter_declaration
                        type: (_) @method.receiver_type
                    )
                )
                name: (field_identifier) @method.name
            ) @method.def

            ; Struct types
            (type_declaration
                (type_spec
                    name: (type_identifier) @struct.name
                    type: (struct_type)
                )
            ) @struct.def

            ; Interface types
            (type_declaration
                (type_spec
                    name: (type_identifier) @interface.name
                    type: (interface_type)
                )
            ) @interface.def

            ; Constants (single)
            (const_declaration
                (const_spec
                    name: (identifier) @const.name
                )
            ) @const.def

            ; Variables (top-level)
            (var_declaration
                (var_spec
                    name: (identifier) @var.name
                )
            ) @var.def

            ; Import statements
            (import_declaration
                (import_spec
                    path: (interpreted_string_literal) @import.path
                )
            ) @import.def

            ; Import blocks
            (import_declaration
                (import_spec_list
                    (import_spec
                        path: (interpreted_string_literal) @import.path
                    ) @import.def
                )
            )
            "#,
        )
        .expect("Invalid Go symbols query");

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
                function: (selector_expression
                    field: (field_identifier) @call.name
                )
            ) @call.expr

            ; Package-qualified calls (e.g., fmt.Println)
            (call_expression
                function: (selector_expression
                    operand: (identifier) @call.pkg
                    field: (field_identifier) @call.name
                )
            ) @call.expr
            "#,
        )
        .expect("Invalid Go calls query");

        Self {
            parser,
            symbols_query,
            calls_query,
        }
    }

    /// Parse Go source code.
    pub fn parse(&mut self, file_path: &str, source: &str) -> Option<ParseResult> {
        let tree = self.parser.parse(source, None)?;
        let root = tree.root_node();

        let mut symbols = Vec::new();
        let mut edges = Vec::new();

        // Extract package name as module
        let module = self.extract_package_name(&root, source, file_path);

        // Extract symbols
        self.extract_symbols(&root, source, file_path, &mut symbols, &mut edges);

        // Extract call edges using standard patterns
        extract_call_edges(
            &self.calls_query,
            &root,
            source,
            &symbols,
            &mut edges,
            &CallCapturePatterns::STANDARD,
        );

        Some(ParseResult {
            file_path: file_path.to_string(),
            language: "go".to_string(),
            symbols,
            edges,
            module,
        })
    }

    /// Extract the package name from the source.
    fn extract_package_name(
        &self,
        root: &Node,
        source: &str,
        file_path: &str,
    ) -> Option<ModuleInfo> {
        // Look for package clause
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "package_clause" {
                // Find the package identifier
                let mut pkg_cursor = child.walk();
                for pkg_child in child.children(&mut pkg_cursor) {
                    if pkg_child.kind() == "package_identifier" {
                        let name = pkg_child.utf8_text(source.as_bytes()).ok()?.to_string();
                        return Some(ModuleInfo {
                            file_path: file_path.to_string(),
                            module_name: Some(name),
                            exports: Vec::new(),
                            imports: Vec::new(),
                        });
                    }
                }
            }
        }
        None
    }

    /// Extract symbols from the AST.
    fn extract_symbols(
        &self,
        root: &Node,
        source: &str,
        file_path: &str,
        symbols: &mut Vec<Symbol>,
        edges: &mut Vec<Edge>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.symbols_query, *root, source.as_bytes());

        // Track which definitions we've processed (to avoid duplicates)
        let mut processed_defs = std::collections::HashSet::new();

        while let Some(m) = matches.next() {
            // Find the definition capture
            let def_capture = m.captures.iter().find(|c| {
                let name = &self.symbols_query.capture_names()[c.index as usize];
                is_def_capture(name)
            });

            let Some(def_capture) = def_capture else {
                continue;
            };

            let def_node = def_capture.node;

            // Skip if already processed
            let def_key = (def_node.start_byte(), def_node.end_byte());
            if processed_defs.contains(&def_key) {
                continue;
            }
            processed_defs.insert(def_key);

            let def_name = &self.symbols_query.capture_names()[def_capture.index as usize];

            // Import definitions produce import edges rather than symbols.
            if def_name.starts_with("import.") {
                push_go_import_edges(&self.symbols_query, m, source, file_path, edges);
                continue;
            }

            // Everything else is a symbol definition.
            if let Some(symbol) =
                build_go_symbol(&self.symbols_query, m, def_node, source, file_path)
            {
                symbols.push(symbol);
            }
        }
    }
}

/// Push import edges for an `import.*` definition match.
fn push_go_import_edges(
    query: &Query,
    m: &tree_sitter::QueryMatch,
    source: &str,
    file_path: &str,
    edges: &mut Vec<Edge>,
) {
    for capture in m.captures {
        let capture_name = query.capture_names()[capture.index as usize];
        if capture_name == "import.path" {
            let import_path = capture
                .node
                .utf8_text(source.as_bytes())
                .unwrap_or("")
                .trim_matches('"')
                .to_string();

            if !import_path.is_empty() {
                edges.push(Edge {
                    source_id: file_path.to_string(),
                    target_id: Some(import_path.clone()),
                    target_name: import_path,
                    kind: EdgeKind::Imports,
                    line: Some(capture.node.start_position().row as u32 + 1),
                    col: Some(capture.node.start_position().column as u32),
                    context: None,
                });
            }
        }
    }
}

/// Build a [`Symbol`] from a Go definition match, or `None` if it should be skipped.
fn build_go_symbol(
    query: &Query,
    m: &tree_sitter::QueryMatch,
    def_node: Node,
    source: &str,
    file_path: &str,
) -> Option<Symbol> {
    // Find name capture first to determine symbol kind
    let name_capture = m.captures.iter().find(|c| {
        let name = &query.capture_names()[c.index as usize];
        name.ends_with(".name")
    })?;

    let name_capture_name = &query.capture_names()[name_capture.index as usize];

    // Determine symbol kind from the name capture
    let kind = find_symbol_kind(name_capture_name, GO_SYMBOL_MAPPINGS)?;

    let name = name_capture
        .node
        .utf8_text(source.as_bytes())
        .unwrap_or("")
        .to_string();

    if name.is_empty() {
        return None;
    }

    // Build signature
    let signature = build_go_signature(query, m, source, kind);

    // Extract docstring (Go uses // comments above declarations)
    let docstring = extract_go_docstring(&def_node, source);
    let brief = docstring.as_ref().and_then(|d| extract_brief(d));

    // Determine visibility (Go uses capitalization)
    let visibility = if name.chars().next().is_some_and(|c| c.is_uppercase()) {
        Visibility::Public
    } else {
        Visibility::Private
    };

    // Get position info
    let start = def_node.start_position();
    let end = def_node.end_position();

    // Create symbol ID
    let id = format!("{}::{}", file_path, name);

    // Handle method receivers
    let parent_id = if kind == SymbolKind::Method {
        // Extract receiver type for parent reference
        m.captures.iter().find_map(|c| {
            let capture_name = query.capture_names()[c.index as usize];
            if capture_name == "method.receiver_type" {
                let receiver_text = c.node.utf8_text(source.as_bytes()).ok()?;
                // Clean up pointer receivers (*Type -> Type)
                let clean_type = receiver_text.trim_start_matches('*');
                Some(format!("{}::{}", file_path, clean_type))
            } else {
                None
            }
        })
    } else {
        None
    };

    Some(Symbol {
        id,
        file_path: file_path.to_string(),
        name,
        qualified_name: None,
        kind,
        visibility,
        signature,
        brief,
        docstring,
        line_start: start.row as u32 + 1,
        line_end: end.row as u32 + 1,
        col_start: start.column as u32,
        col_end: end.column as u32,
        parent_id,
        source: None,
    })
}

/// Build a signature string for a symbol.
fn build_go_signature(
    query: &Query,
    m: &tree_sitter::QueryMatch,
    source: &str,
    kind: SymbolKind,
) -> Option<String> {
    match kind {
        SymbolKind::Function | SymbolKind::Method => {
            // Find params and return type
            let mut params = None;
            let mut return_type = None;
            let mut name = "";
            let mut receiver = None;

            for capture in m.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).ok()?;

                if capture_name.ends_with(".name") {
                    name = text;
                } else if capture_name.ends_with(".params") {
                    params = Some(text);
                } else if capture_name.ends_with(".return") {
                    return_type = Some(text);
                } else if capture_name == "method.receiver_type" {
                    receiver = Some(text);
                }
            }

            let mut sig = String::from("func ");
            if let Some(recv) = receiver {
                sig.push_str(&format!("({}) ", recv));
            }
            sig.push_str(name);
            if let Some(p) = params {
                sig.push_str(p);
            } else {
                sig.push_str("()");
            }
            if let Some(ret) = return_type {
                sig.push(' ');
                sig.push_str(ret);
            }
            Some(sig)
        }
        SymbolKind::Struct => {
            let name = m.captures.iter().find_map(|c| {
                let capture_name = query.capture_names()[c.index as usize];
                if capture_name == "struct.name" {
                    c.node.utf8_text(source.as_bytes()).ok()
                } else {
                    None
                }
            })?;
            Some(format!("type {} struct", name))
        }
        SymbolKind::Interface => {
            let name = m.captures.iter().find_map(|c| {
                let capture_name = query.capture_names()[c.index as usize];
                if capture_name == "interface.name" {
                    c.node.utf8_text(source.as_bytes()).ok()
                } else {
                    None
                }
            })?;
            Some(format!("type {} interface", name))
        }
        SymbolKind::Type => {
            let name = m.captures.iter().find_map(|c| {
                let capture_name = query.capture_names()[c.index as usize];
                if capture_name == "type.name" {
                    c.node.utf8_text(source.as_bytes()).ok()
                } else {
                    None
                }
            })?;
            Some(format!("type {}", name))
        }
        SymbolKind::Const => {
            let name = m.captures.iter().find_map(|c| {
                let capture_name = query.capture_names()[c.index as usize];
                if capture_name == "const.name" {
                    c.node.utf8_text(source.as_bytes()).ok()
                } else {
                    None
                }
            })?;
            Some(format!("const {}", name))
        }
        SymbolKind::Variable => {
            let name = m.captures.iter().find_map(|c| {
                let capture_name = query.capture_names()[c.index as usize];
                if capture_name == "var.name" {
                    c.node.utf8_text(source.as_bytes()).ok()
                } else {
                    None
                }
            })?;
            Some(format!("var {}", name))
        }
        _ => None,
    }
}

/// Extract docstring from comments above a node.
fn extract_go_docstring(node: &Node, source: &str) -> Option<String> {
    // Look for comments immediately preceding the node
    let mut prev = node.prev_sibling();
    let mut doc_lines = Vec::new();

    while let Some(sibling) = prev {
        if sibling.kind() == "comment" {
            let comment_text = sibling.utf8_text(source.as_bytes()).ok()?;
            // Strip // prefix and trim
            let clean = comment_text
                .strip_prefix("//")
                .unwrap_or(comment_text)
                .trim();
            doc_lines.push(clean.to_string());
            prev = sibling.prev_sibling();
        } else {
            break;
        }
    }

    if doc_lines.is_empty() {
        return None;
    }

    // Reverse since we collected bottom-up
    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}

impl Default for GoParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_go_function() {
        let mut parser = GoParser::new();
        let source = r#"
package main

// Hello prints a greeting message.
func Hello(name string) string {
    return "Hello, " + name
}
"#;

        let result = parser.parse("test.go", source).unwrap();
        assert_eq!(result.language, "go");

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "Hello")
            .expect("Should find Hello function");

        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.visibility, Visibility::Public);
        assert!(func.signature.as_ref().unwrap().contains("func Hello"));
        assert!(func.brief.as_ref().unwrap().contains("prints a greeting"));
    }

    #[test]
    fn test_parse_go_struct() {
        let mut parser = GoParser::new();
        let source = r#"
package main

// User represents a user in the system.
type User struct {
    Name  string
    Email string
    Age   int
}
"#;

        let result = parser.parse("test.go", source).unwrap();

        let user = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Should find User struct");

        assert_eq!(user.kind, SymbolKind::Struct);
        assert_eq!(user.visibility, Visibility::Public);
        assert!(user
            .signature
            .as_ref()
            .unwrap()
            .contains("type User struct"));
    }

    #[test]
    fn test_parse_go_interface() {
        let mut parser = GoParser::new();
        let source = r#"
package main

// Reader is an interface for reading data.
type Reader interface {
    Read(p []byte) (n int, err error)
}
"#;

        let result = parser.parse("test.go", source).unwrap();

        let reader = result
            .symbols
            .iter()
            .find(|s| s.name == "Reader")
            .expect("Should find Reader interface");

        assert_eq!(reader.kind, SymbolKind::Interface);
        assert_eq!(reader.visibility, Visibility::Public);
        assert!(reader
            .signature
            .as_ref()
            .unwrap()
            .contains("type Reader interface"));
    }

    #[test]
    fn test_parse_go_method_receiver() {
        let mut parser = GoParser::new();
        let source = r#"
package main

type User struct {
    Name string
}

// Greet returns a greeting for the user.
func (u *User) Greet() string {
    return "Hello, " + u.Name
}

// private method
func (u User) getName() string {
    return u.Name
}
"#;

        let result = parser.parse("test.go", source).unwrap();

        let greet = result
            .symbols
            .iter()
            .find(|s| s.name == "Greet")
            .expect("Should find Greet method");

        assert_eq!(greet.kind, SymbolKind::Method);
        assert_eq!(greet.visibility, Visibility::Public);
        assert!(greet.parent_id.as_ref().unwrap().contains("User"));
        assert!(greet.signature.as_ref().unwrap().contains("(*User)"));

        let get_name = result
            .symbols
            .iter()
            .find(|s| s.name == "getName")
            .expect("Should find getName method");

        assert_eq!(get_name.visibility, Visibility::Private);
    }

    #[test]
    fn test_parse_go_imports() {
        let mut parser = GoParser::new();
        let source = r#"
package main

import (
    "fmt"
    "strings"
)

func main() {
    fmt.Println("Hello")
}
"#;

        let result = parser.parse("test.go", source).unwrap();

        // Check for import edges
        let import_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();

        assert!(import_edges.len() >= 2, "Should have at least 2 imports");
        assert!(import_edges.iter().any(|e| e.target_name == "fmt"));
        assert!(import_edges.iter().any(|e| e.target_name == "strings"));
    }

    #[test]
    fn test_parse_go_calls() {
        let mut parser = GoParser::new();
        let source = r#"
package main

func helper() string {
    return "help"
}

func main() {
    result := helper()
    println(result)
}
"#;

        let result = parser.parse("test.go", source).unwrap();

        // Check for call edges from main to helper
        let call_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        assert!(!call_edges.is_empty(), "Should have call edges");
        assert!(
            call_edges.iter().any(|e| e.target_name.contains("helper")),
            "Should have call to helper"
        );
    }

    #[test]
    fn test_parse_go_package() {
        let mut parser = GoParser::new();
        let source = r#"
package mypackage

func Test() {}
"#;

        let result = parser.parse("test.go", source).unwrap();
        assert!(result.module.is_some());
        assert_eq!(
            result.module.as_ref().unwrap().module_name,
            Some("mypackage".to_string())
        );
    }

    #[test]
    fn test_parse_go_const_and_var() {
        let mut parser = GoParser::new();
        let source = r#"
package main

const MaxSize = 100

var globalVar = "hello"
"#;

        let result = parser.parse("test.go", source).unwrap();

        let max_size = result
            .symbols
            .iter()
            .find(|s| s.name == "MaxSize")
            .expect("Should find MaxSize const");

        assert_eq!(max_size.kind, SymbolKind::Const);
        assert_eq!(max_size.visibility, Visibility::Public);

        let global_var = result
            .symbols
            .iter()
            .find(|s| s.name == "globalVar")
            .expect("Should find globalVar");

        assert_eq!(global_var.kind, SymbolKind::Variable);
        assert_eq!(global_var.visibility, Visibility::Private);
    }
}
