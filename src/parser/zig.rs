//! Zig-specific code parsing using tree-sitter.

use tree_sitter::{Node, Parser, Query};

use crate::db::{ImportInfo, ModuleInfo, ParseResult, Symbol, SymbolKind, Visibility};
use crate::parser::{extract_brief, extract_call_edges, extract_module_name, CallCapturePatterns};

const CONTAINER_KINDS: &[&str] = &[
    "struct_declaration",
    "enum_declaration",
    "union_declaration",
    "opaque_declaration",
    "error_set_declaration",
];

const ZIG_CALL_PATTERNS: CallCapturePatterns = CallCapturePatterns {
    name_patterns: &["call.name", "method_call.name", "builtin_call.name"],
    expr_patterns: &["call.expr", "method_call.expr", "builtin_call.expr"],
};

#[derive(Clone)]
struct ContainerContext {
    name: String,
    qualified_name: String,
    id: String,
}

/// Zig parser for declarations, calls, imports, and documentation comments.
pub struct ZigParser {
    parser: Parser,
    calls_query: Query,
}

impl ZigParser {
    pub fn new() -> Self {
        let language: tree_sitter::Language = tree_sitter_zig::LANGUAGE.into();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .expect("Failed to set Zig language");

        let calls_query = Query::new(
            &language,
            r#"
            (call_expression
                function: (identifier) @call.name
            ) @call.expr

            (call_expression
                function: (field_expression
                    member: (identifier) @method_call.name
                )
            ) @method_call.expr

            (builtin_function
                (builtin_identifier) @builtin_call.name
            ) @builtin_call.expr
            "#,
        )
        .expect("Invalid Zig calls query");

        Self {
            parser,
            calls_query,
        }
    }

    pub fn parse(&mut self, file_path: &str, source: &str) -> Option<ParseResult> {
        let tree = self.parser.parse(source, None)?;
        let root = tree.root_node();
        let mut symbols = Vec::new();
        let mut imports = Vec::new();
        let mut exports = Vec::new();

        extract_members(
            root,
            None,
            file_path,
            source,
            &mut symbols,
            &mut imports,
            &mut exports,
        );

        let mut edges = Vec::new();
        extract_call_edges(
            &self.calls_query,
            &root,
            source,
            &symbols,
            &mut edges,
            &ZIG_CALL_PATTERNS,
        );

        Some(ParseResult {
            file_path: file_path.to_string(),
            language: "zig".to_string(),
            symbols,
            edges,
            module: Some(ModuleInfo {
                file_path: file_path.to_string(),
                module_name: extract_module_name(file_path, &[]),
                exports,
                imports,
            }),
        })
    }
}

fn extract_members(
    node: Node<'_>,
    parent: Option<&ContainerContext>,
    file_path: &str,
    source: &str,
    symbols: &mut Vec<Symbol>,
    imports: &mut Vec<ImportInfo>,
    exports: &mut Vec<String>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(symbol) = function_symbol(child, parent, file_path, source) {
                    record_symbol(symbol, symbols, exports);
                }
            }
            "test_declaration" => {
                if let Some(symbol) = test_symbol(child, parent, file_path, source) {
                    record_symbol(symbol, symbols, exports);
                }
            }
            "variable_declaration" => {
                let Some(name_node) = direct_child_of_kind(child, "identifier") else {
                    continue;
                };
                let name = node_text(name_node, source).to_string();
                if name.is_empty() {
                    continue;
                }

                if let Some(import) = import_info(child, &name, source) {
                    imports.push(import);
                }

                if let Some(container_node) = find_descendant_kind(child, CONTAINER_KINDS) {
                    let symbol = variable_symbol(
                        child,
                        &name,
                        container_symbol_kind(container_node.kind()),
                        parent,
                        file_path,
                        source,
                    );
                    let context = ContainerContext {
                        name: name.clone(),
                        qualified_name: symbol
                            .qualified_name
                            .clone()
                            .unwrap_or_else(|| name.clone()),
                        id: symbol.id.clone(),
                    };
                    record_symbol(symbol, symbols, exports);
                    extract_members(
                        container_node,
                        Some(&context),
                        file_path,
                        source,
                        symbols,
                        imports,
                        exports,
                    );
                } else {
                    let declaration = node_text(child, source);
                    let kind = if declaration_keyword(declaration) == "var" {
                        SymbolKind::Variable
                    } else {
                        SymbolKind::Const
                    };
                    let symbol = variable_symbol(child, &name, kind, parent, file_path, source);
                    record_symbol(symbol, symbols, exports);
                }
            }
            _ => {}
        }
    }
}

fn record_symbol(symbol: Symbol, symbols: &mut Vec<Symbol>, exports: &mut Vec<String>) {
    if symbol.visibility == Visibility::Public {
        exports.push(symbol.name.clone());
    }
    symbols.push(symbol);
}

fn function_symbol(
    node: Node<'_>,
    parent: Option<&ContainerContext>,
    file_path: &str,
    source: &str,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);
    if name.is_empty() {
        return None;
    }
    let kind = if parent.is_some() {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };
    Some(build_symbol(
        node,
        name,
        kind,
        parent,
        file_path,
        source,
        Some(declaration_signature(node, source)),
    ))
}

fn test_symbol(
    node: Node<'_>,
    parent: Option<&ContainerContext>,
    file_path: &str,
    source: &str,
) -> Option<Symbol> {
    let name_node = direct_child_of_kinds(node, &["string", "identifier"])?;
    let raw_name = node_text(name_node, source);
    let name = raw_name.trim_matches('"');
    if name.is_empty() {
        return None;
    }
    Some(build_symbol(
        node,
        name,
        SymbolKind::Function,
        parent,
        file_path,
        source,
        Some(declaration_signature(node, source)),
    ))
}

fn variable_symbol(
    node: Node<'_>,
    name: &str,
    kind: SymbolKind,
    parent: Option<&ContainerContext>,
    file_path: &str,
    source: &str,
) -> Symbol {
    build_symbol(
        node,
        name,
        kind,
        parent,
        file_path,
        source,
        Some(variable_signature(node, source)),
    )
}

fn build_symbol(
    node: Node<'_>,
    name: &str,
    kind: SymbolKind,
    parent: Option<&ContainerContext>,
    file_path: &str,
    source: &str,
    signature: Option<String>,
) -> Symbol {
    let qualified_name = parent
        .map(|p| format!("{}.{}", p.qualified_name, name))
        .or_else(|| Some(name.to_string()));
    let docstring = extract_zig_docstring(node, source);
    let start = node.start_position();
    let end = node.end_position();
    Symbol {
        id: Symbol::make_id(file_path, name, parent.map(|p| p.name.as_str())),
        file_path: file_path.to_string(),
        name: name.to_string(),
        qualified_name,
        kind,
        visibility: if is_public(node, source) {
            Visibility::Public
        } else {
            Visibility::Private
        },
        signature,
        brief: docstring.as_deref().and_then(extract_brief),
        docstring,
        line_start: start.row as u32 + 1,
        line_end: end.row as u32 + 1,
        col_start: start.column as u32,
        col_end: end.column as u32,
        parent_id: parent.map(|p| p.id.clone()),
        source: Some(node_text(node, source).to_string()),
    }
}

fn container_symbol_kind(kind: &str) -> SymbolKind {
    match kind {
        "struct_declaration" => SymbolKind::Struct,
        "enum_declaration" => SymbolKind::Enum,
        _ => SymbolKind::Type,
    }
}

fn import_info(node: Node<'_>, alias: &str, source: &str) -> Option<ImportInfo> {
    let builtin = find_descendant_kind(node, &["builtin_function"])?;
    let builtin_name = direct_child_of_kind(builtin, "builtin_identifier")?;
    if node_text(builtin_name, source) != "@import" {
        return None;
    }
    let string = find_descendant_kind(builtin, &["string"])?;
    let from = node_text(string, source).trim_matches('"').to_string();
    if from.is_empty() {
        return None;
    }
    Some(ImportInfo {
        from,
        names: Vec::new(),
        alias: Some(alias.to_string()),
    })
}

fn declaration_keyword(source: &str) -> &str {
    source
        .split(|c: char| c.is_whitespace() || c == ';')
        .find(|word| matches!(*word, "const" | "var"))
        .unwrap_or("const")
}

fn is_public(node: Node<'_>, source: &str) -> bool {
    node_text(node, source)
        .split_whitespace()
        .next()
        .is_some_and(|word| word == "pub")
}

fn declaration_signature(node: Node<'_>, source: &str) -> String {
    if let Some(body) = node.child_by_field_name("body") {
        source[node.start_byte()..body.start_byte()]
            .trim()
            .to_string()
    } else {
        node_text(node, source)
            .trim()
            .trim_end_matches(';')
            .to_string()
    }
}

fn variable_signature(node: Node<'_>, source: &str) -> String {
    let text = node_text(node, source).trim().trim_end_matches(';');
    text.split_once('=')
        .map_or(text, |(header, _)| header.trim())
        .to_string()
}

fn extract_zig_docstring(node: Node<'_>, source: &str) -> Option<String> {
    let mut comments = Vec::new();
    let mut expected_row = node.start_position().row;
    let mut sibling = node.prev_named_sibling();
    while let Some(previous) = sibling {
        if previous.kind() != "comment" || previous.end_position().row + 1 != expected_row {
            break;
        }
        let text = node_text(previous, source).trim();
        let Some(doc) = text.strip_prefix("///") else {
            break;
        };
        comments.push(doc.trim().to_string());
        expected_row = previous.start_position().row;
        sibling = previous.prev_named_sibling();
    }
    comments.reverse();
    (!comments.is_empty()).then(|| comments.join("\n"))
}

fn direct_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    direct_child_of_kinds(node, &[kind])
}

fn direct_child_of_kinds<'tree>(node: Node<'tree>, kinds: &[&str]) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .find(|child| kinds.contains(&child.kind()));
    found
}

fn find_descendant_kind<'tree>(node: Node<'tree>, kinds: &[&str]) -> Option<Node<'tree>> {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current != node && kinds.contains(&current.kind()) {
            return Some(current);
        }
        let mut cursor = current.walk();
        let children: Vec<_> = current.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    None
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_zig_symbols_calls_imports_and_docs() {
        let source = r#"
const std = @import("std");
const util = @import("util.zig");

/// Public entry point.
/// Handles a request.
pub fn run(value: usize) !void {
    helper(value);
    std.debug.print("{}", .{value});
    @panic("boom");
}

fn helper(value: usize) void { _ = value; }

pub const Server = struct {
    field: usize,

    /// Starts the server.
    pub fn start(self: *Server) void {
        self.stop();
    }

    fn stop(self: *Server) void { _ = self; }
};

const Mode = enum { fast, slow };
const Choice = union(enum) { one: u8, two: u16 };
const Hidden = opaque {};
const Failure = error { Broken };
pub const limit: usize = 10;
var counter: usize = 0;

test "named behavior" { helper(1); }
test { helper(2); }
"#;

        let result = ZigParser::new().parse("src/main.zig", source).unwrap();
        assert_eq!(result.language, "zig");
        let symbol = |name: &str| result.symbols.iter().find(|s| s.name == name).unwrap();

        assert_eq!(symbol("run").kind, SymbolKind::Function);
        assert_eq!(symbol("run").visibility, Visibility::Public);
        assert_eq!(
            symbol("run").docstring.as_deref(),
            Some("Public entry point.\nHandles a request.")
        );
        assert_eq!(symbol("Server").kind, SymbolKind::Struct);
        assert_eq!(symbol("start").kind, SymbolKind::Method);
        assert_eq!(
            symbol("start").parent_id.as_deref(),
            Some("src/main.zig::Server")
        );
        assert_eq!(
            symbol("start").qualified_name.as_deref(),
            Some("Server.start")
        );
        assert_eq!(symbol("Mode").kind, SymbolKind::Enum);
        assert_eq!(symbol("Choice").kind, SymbolKind::Type);
        assert_eq!(symbol("Hidden").kind, SymbolKind::Type);
        assert_eq!(symbol("Failure").kind, SymbolKind::Type);
        assert_eq!(symbol("limit").kind, SymbolKind::Const);
        assert_eq!(symbol("counter").kind, SymbolKind::Variable);
        assert_eq!(symbol("named behavior").kind, SymbolKind::Function);
        assert_eq!(
            result
                .symbols
                .iter()
                .filter(|s| s.name == "named behavior")
                .count(),
            1
        );
        assert_eq!(result.symbols.len(), 14);

        let module = result.module.unwrap();
        assert_eq!(module.module_name.as_deref(), Some("main"));
        assert!(module.exports.contains(&"run".to_string()));
        assert!(module.exports.contains(&"Server".to_string()));
        assert!(module
            .imports
            .iter()
            .any(|i| i.from == "std" && i.alias.as_deref() == Some("std")));
        assert!(module
            .imports
            .iter()
            .any(|i| i.from == "util.zig" && i.alias.as_deref() == Some("util")));

        let calls: Vec<&str> = result
            .edges
            .iter()
            .map(|e| e.target_name.as_str())
            .collect();
        assert!(calls.contains(&"helper"));
        assert!(calls.contains(&"print"));
        assert!(calls.contains(&"@panic"));
        assert!(calls.contains(&"stop"));
    }
}
