//! C and C++ parsing using tree-sitter.

use std::collections::HashSet;

use tree_sitter::{Node, Parser, Tree};

use crate::db::{
    Edge, EdgeKind, ImportInfo, ModuleInfo, ParseResult, Symbol, SymbolKind, Visibility,
};
use crate::parser::{
    extract_brief, extract_module_name, parse_block_doc_comment, truncate_context, Language,
};

#[derive(Clone)]
struct Scope {
    name: String,
    qualified: String,
    id: String,
    kind: ScopeKind,
    visibility: Visibility,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Namespace,
    Class,
    Struct,
}

/// Shared C/C++ parser. Headers are parsed with both grammars and the tree
/// with fewer error/missing nodes wins; C wins ties.
pub struct CCppParser {
    c: Parser,
    cpp: Parser,
}

impl CCppParser {
    pub fn new() -> Self {
        let mut c = Parser::new();
        let c_language: tree_sitter::Language = tree_sitter_c::LANGUAGE.into();
        c.set_language(&c_language)
            .expect("Failed to set C language");

        let mut cpp = Parser::new();
        let cpp_language: tree_sitter::Language = tree_sitter_cpp::LANGUAGE.into();
        cpp.set_language(&cpp_language)
            .expect("Failed to set C++ language");
        Self { c, cpp }
    }

    pub fn parse(
        &mut self,
        file_path: &str,
        source: &str,
        requested: Language,
    ) -> Option<ParseResult> {
        let is_header = std::path::Path::new(file_path)
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("h");
        let (tree, language) = if is_header {
            let c_tree = self.c.parse(source, None)?;
            let cpp_tree = self.cpp.parse(source, None)?;
            if syntax_error_count(c_tree.root_node()) <= syntax_error_count(cpp_tree.root_node()) {
                (c_tree, Language::C)
            } else {
                (cpp_tree, Language::Cpp)
            }
        } else if requested == Language::Cpp {
            (self.cpp.parse(source, None)?, Language::Cpp)
        } else {
            (self.c.parse(source, None)?, Language::C)
        };

        Some(extract(file_path, source, &tree, language))
    }
}

pub(super) fn classify_header(source: &str) -> Language {
    let mut c = Parser::new();
    let c_language: tree_sitter::Language = tree_sitter_c::LANGUAGE.into();
    c.set_language(&c_language)
        .expect("Failed to set C language");
    let mut cpp = Parser::new();
    let cpp_language: tree_sitter::Language = tree_sitter_cpp::LANGUAGE.into();
    cpp.set_language(&cpp_language)
        .expect("Failed to set C++ language");
    match (c.parse(source, None), cpp.parse(source, None)) {
        (Some(c_tree), Some(cpp_tree))
            if syntax_error_count(c_tree.root_node())
                > syntax_error_count(cpp_tree.root_node()) =>
        {
            Language::Cpp
        }
        _ => Language::C,
    }
}

fn syntax_error_count(root: Node<'_>) -> usize {
    let mut count = 0;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            count += 1;
        }
        for index in 0..node.child_count() {
            if let Some(child) = node.child(index) {
                stack.push(child);
            }
        }
    }
    count
}

fn extract(file_path: &str, source: &str, tree: &Tree, language: Language) -> ParseResult {
    let mut symbols = Vec::new();
    let mut edges = Vec::new();
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    let mut scopes = Vec::new();
    let mut processed = HashSet::new();

    walk(
        tree.root_node(),
        source,
        file_path,
        language,
        &mut scopes,
        &mut symbols,
        &mut edges,
        &mut imports,
        &mut exports,
        &mut processed,
    );

    // Resolve qualified out-of-class definitions to a syntactic class or
    // namespace declared in this file.
    for index in 0..symbols.len() {
        let Some(qualified) = symbols[index].qualified_name.clone() else {
            continue;
        };
        let Some((parent_name, _)) = qualified.rsplit_once("::") else {
            continue;
        };
        if let Some((parent_id, parent_kind)) = symbols
            .iter()
            .find(|candidate| {
                candidate.qualified_name.as_deref() == Some(parent_name)
                    && matches!(
                        candidate.kind,
                        SymbolKind::Class | SymbolKind::Struct | SymbolKind::Module
                    )
            })
            .map(|parent| (parent.id.clone(), parent.kind))
        {
            symbols[index].parent_id = Some(parent_id);
            if matches!(parent_kind, SymbolKind::Class | SymbolKind::Struct) {
                symbols[index].kind = SymbolKind::Method;
            }
        }
    }

    ParseResult {
        file_path: file_path.to_string(),
        language: language.as_str().to_string(),
        symbols,
        edges,
        module: Some(ModuleInfo {
            file_path: file_path.to_string(),
            module_name: extract_module_name(file_path, &[]),
            exports,
            imports,
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    node: Node<'_>,
    source: &str,
    file_path: &str,
    language: Language,
    scopes: &mut Vec<Scope>,
    symbols: &mut Vec<Symbol>,
    edges: &mut Vec<Edge>,
    imports: &mut Vec<ImportInfo>,
    exports: &mut Vec<String>,
    processed: &mut HashSet<(usize, usize, String)>,
) {
    match node.kind() {
        "preproc_include" => {
            if let Some(path) = node.child_by_field_name("path") {
                let raw = text(path, source).trim();
                let from = if raw.starts_with('"') && raw.ends_with('"') {
                    raw[1..raw.len() - 1].to_string()
                } else if raw.starts_with('<') && raw.ends_with('>') {
                    raw.to_string()
                } else {
                    return;
                };
                if !from.is_empty() {
                    imports.push(ImportInfo {
                        from,
                        names: Vec::new(),
                        alias: None,
                    });
                }
            }
            return;
        }
        "preproc_def" | "preproc_function_def" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                push_symbol(
                    node,
                    clean_name(text(name_node, source)),
                    SymbolKind::Macro,
                    source,
                    file_path,
                    language,
                    scopes,
                    symbols,
                    exports,
                    processed,
                    None,
                );
            }
            return;
        }
        "namespace_definition" if language == Language::Cpp => {
            let name = node
                .child_by_field_name("name")
                .map(|name| clean_name(text(name, source)))
                .filter(|name| !name.is_empty());
            let anonymous = name.is_none();
            let scope_name = name.unwrap_or_else(|| "<anonymous>".to_string());
            let qualified = qualify(scopes, &scope_name);
            let id = Symbol::make_id(
                file_path,
                &scope_name,
                scopes.last().map(|scope| scope.name.as_str()),
            );
            if !anonymous {
                push_symbol(
                    node,
                    scope_name.clone(),
                    SymbolKind::Module,
                    source,
                    file_path,
                    language,
                    scopes,
                    symbols,
                    exports,
                    processed,
                    Some(qualified.clone()),
                );
            }
            scopes.push(Scope {
                name: scope_name,
                qualified,
                id,
                kind: ScopeKind::Namespace,
                visibility: if anonymous {
                    Visibility::Private
                } else {
                    current_visibility(scopes, node, source)
                },
            });
            if let Some(body) = node.child_by_field_name("body") {
                walk_children(
                    body, source, file_path, language, scopes, symbols, edges, imports, exports,
                    processed,
                );
            }
            scopes.pop();
            return;
        }
        "class_specifier" | "struct_specifier" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                walk_children(
                    node, source, file_path, language, scopes, symbols, edges, imports, exports,
                    processed,
                );
                return;
            };
            let name = clean_name(text(name_node, source));
            if name.is_empty() {
                return;
            }
            let kind = if node.kind() == "class_specifier" {
                SymbolKind::Class
            } else {
                SymbolKind::Struct
            };
            let qualified = qualify(scopes, &name);
            let id = Symbol::make_id(
                file_path,
                &name,
                scopes.last().map(|scope| scope.name.as_str()),
            );
            push_symbol(
                outer_template(node),
                name.clone(),
                kind,
                source,
                file_path,
                language,
                scopes,
                symbols,
                exports,
                processed,
                Some(qualified.clone()),
            );
            scopes.push(Scope {
                name,
                qualified,
                id,
                kind: if kind == SymbolKind::Class {
                    ScopeKind::Class
                } else {
                    ScopeKind::Struct
                },
                visibility: if kind == SymbolKind::Class {
                    Visibility::Private
                } else {
                    Visibility::Public
                },
            });
            if let Some(body) = node.child_by_field_name("body") {
                walk_class_body(
                    body, source, file_path, language, scopes, symbols, edges, imports, exports,
                    processed,
                );
            }
            scopes.pop();
            return;
        }
        "union_specifier" | "enum_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let kind = if node.kind() == "enum_specifier" {
                    SymbolKind::Enum
                } else {
                    SymbolKind::Type
                };
                push_symbol(
                    node,
                    clean_name(text(name_node, source)),
                    kind,
                    source,
                    file_path,
                    language,
                    scopes,
                    symbols,
                    exports,
                    processed,
                    None,
                );
            }
            // Named nested declarations inside the body are still useful.
        }
        "type_definition" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                let name = declarator_name(declarator, source);
                let named_compound = node.child_by_field_name("type").and_then(|ty| {
                    matches!(
                        ty.kind(),
                        "struct_specifier" | "union_specifier" | "enum_specifier"
                    )
                    .then(|| ty.child_by_field_name("name"))
                    .flatten()
                });
                let same_as_compound = named_compound
                    .map(|compound| clean_name(text(compound, source)) == name)
                    .unwrap_or(false);
                if !same_as_compound {
                    push_symbol(
                        node,
                        name,
                        SymbolKind::Type,
                        source,
                        file_path,
                        language,
                        scopes,
                        symbols,
                        exports,
                        processed,
                        None,
                    );
                }
            }
        }
        "alias_declaration" if language == Language::Cpp => {
            if let Some(name) = node.child_by_field_name("name") {
                push_symbol(
                    node,
                    clean_name(text(name, source)),
                    SymbolKind::Type,
                    source,
                    file_path,
                    language,
                    scopes,
                    symbols,
                    exports,
                    processed,
                    None,
                );
            }
        }
        "function_definition" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                push_function(
                    outer_template(node),
                    declarator,
                    source,
                    file_path,
                    language,
                    scopes,
                    symbols,
                    exports,
                    processed,
                );
            }
        }
        "declaration" | "field_declaration" => {
            let functions = descendants_of_kind(node, "function_declarator");
            if functions.is_empty() {
                if node.kind() == "declaration" && !inside_function(node) && !inside_class(scopes) {
                    for declarator in declaration_declarators(node) {
                        let kind = if has_token(node, source, "const") {
                            SymbolKind::Const
                        } else {
                            SymbolKind::Variable
                        };
                        push_symbol(
                            node,
                            declarator_name(declarator, source),
                            kind,
                            source,
                            file_path,
                            language,
                            scopes,
                            symbols,
                            exports,
                            processed,
                            None,
                        );
                    }
                } else if language == Language::Cpp && inside_function(node) {
                    if let Some(type_node) = node.child_by_field_name("type") {
                        let has_constructor_arguments =
                            descendants_of_kind(node, "init_declarator")
                                .iter()
                                .any(|declarator| {
                                    declarator
                                        .child_by_field_name("value")
                                        .is_some_and(|value| value.kind() == "argument_list")
                                });
                        if has_constructor_arguments {
                            push_named_call(
                                node,
                                type_call_name(type_node, source),
                                source,
                                symbols,
                                edges,
                            );
                        }
                    }
                }
            } else {
                for function in functions {
                    push_function(
                        node, function, source, file_path, language, scopes, symbols, exports,
                        processed,
                    );
                }
            }
        }
        "call_expression" => push_call(node, source, symbols, edges),
        "new_expression" if language == Language::Cpp => {
            if let Some(type_node) = node.child_by_field_name("type") {
                push_named_call(
                    node,
                    type_call_name(type_node, source),
                    source,
                    symbols,
                    edges,
                );
            }
        }
        _ => {}
    }

    walk_children(
        node, source, file_path, language, scopes, symbols, edges, imports, exports, processed,
    );
}

#[allow(clippy::too_many_arguments)]
fn walk_children(
    node: Node<'_>,
    source: &str,
    file_path: &str,
    language: Language,
    scopes: &mut Vec<Scope>,
    symbols: &mut Vec<Symbol>,
    edges: &mut Vec<Edge>,
    imports: &mut Vec<ImportInfo>,
    exports: &mut Vec<String>,
    processed: &mut HashSet<(usize, usize, String)>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(
            child, source, file_path, language, scopes, symbols, edges, imports, exports, processed,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_class_body(
    node: Node<'_>,
    source: &str,
    file_path: &str,
    language: Language,
    scopes: &mut Vec<Scope>,
    symbols: &mut Vec<Symbol>,
    edges: &mut Vec<Edge>,
    imports: &mut Vec<ImportInfo>,
    exports: &mut Vec<String>,
    processed: &mut HashSet<(usize, usize, String)>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "access_specifier" {
            if let Some(scope) = scopes.last_mut() {
                scope.visibility = match text(child, source).trim_end_matches(':').trim() {
                    "public" => Visibility::Public,
                    "private" | "protected" => Visibility::Private,
                    _ => scope.visibility,
                };
            }
        } else {
            walk(
                child, source, file_path, language, scopes, symbols, edges, imports, exports,
                processed,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_function(
    node: Node<'_>,
    declarator: Node<'_>,
    source: &str,
    file_path: &str,
    language: Language,
    scopes: &[Scope],
    symbols: &mut Vec<Symbol>,
    exports: &mut Vec<String>,
    processed: &mut HashSet<(usize, usize, String)>,
) {
    let raw_name = declarator_name(declarator, source);
    if raw_name.is_empty() {
        return;
    }
    let qualified = if raw_name.contains("::") {
        Some(raw_name.clone())
    } else {
        Some(qualify(scopes, &raw_name))
    };
    let name = raw_name
        .rsplit("::")
        .next()
        .unwrap_or(&raw_name)
        .to_string();
    let kind = if inside_class(scopes) || raw_name.contains("::") {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };
    push_symbol(
        node, name, kind, source, file_path, language, scopes, symbols, exports, processed,
        qualified,
    );
}

#[allow(clippy::too_many_arguments)]
fn push_symbol(
    node: Node<'_>,
    name: String,
    kind: SymbolKind,
    source: &str,
    file_path: &str,
    _language: Language,
    scopes: &[Scope],
    symbols: &mut Vec<Symbol>,
    exports: &mut Vec<String>,
    processed: &mut HashSet<(usize, usize, String)>,
    qualified_override: Option<String>,
) {
    if name.is_empty()
        || !processed.insert((
            node.start_byte(),
            node.end_byte(),
            format!("{}:{name}", kind.as_str()),
        ))
    {
        return;
    }
    let visibility = current_visibility(scopes, node, source);
    let qualified = qualified_override.unwrap_or_else(|| qualify(scopes, &name));
    let parent_id = scopes.last().map(|scope| scope.id.clone());
    let docstring = doc_comment(node, source);
    let signature = Some(signature(node, source));
    if visibility == Visibility::Public
        && !scopes
            .iter()
            .any(|scope| matches!(scope.kind, ScopeKind::Class | ScopeKind::Struct))
    {
        exports.push(name.clone());
    }
    symbols.push(Symbol {
        id: Symbol::make_id(
            file_path,
            &name,
            scopes.last().map(|scope| scope.name.as_str()),
        ),
        file_path: file_path.to_string(),
        name,
        qualified_name: Some(qualified),
        kind,
        visibility,
        signature,
        brief: docstring.as_deref().and_then(extract_brief),
        docstring,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        col_start: node.start_position().column as u32,
        col_end: node.end_position().column as u32,
        parent_id,
        source: Some(text(node, source).to_string()),
    });
}

fn push_call(node: Node<'_>, source: &str, symbols: &[Symbol], edges: &mut Vec<Edge>) {
    let Some(function) = node.child_by_field_name("function") else {
        return;
    };
    let name = call_name(function, source);
    push_named_call(node, name, source, symbols, edges);
}

fn push_named_call(
    node: Node<'_>,
    name: String,
    source: &str,
    symbols: &[Symbol],
    edges: &mut Vec<Edge>,
) {
    if name.is_empty() {
        return;
    }
    let line = node.start_position().row as u32 + 1;
    let source_symbol = symbols
        .iter()
        .filter(|symbol| {
            matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method)
                && line >= symbol.line_start
                && line <= symbol.line_end
        })
        .min_by_key(|symbol| symbol.line_end - symbol.line_start);
    let Some(source_symbol) = source_symbol else {
        return;
    };
    let target_id = symbols
        .iter()
        .find(|symbol| {
            symbol.name == name
                && symbol
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| text(node, source).contains(qualified))
        })
        .map(|symbol| symbol.id.clone());
    edges.push(Edge {
        source_id: source_symbol.id.clone(),
        target_id,
        target_name: name,
        kind: EdgeKind::Calls,
        line: Some(line),
        col: Some(node.start_position().column as u32),
        context: Some(truncate_context(text(node, source), 80)),
    });
}

fn type_call_name(node: Node<'_>, source: &str) -> String {
    let raw = clean_name(text(node, source));
    raw.rsplit("::")
        .next()
        .unwrap_or(&raw)
        .split('<')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn call_name(node: Node<'_>, source: &str) -> String {
    match node.kind() {
        "field_expression" => node
            .child_by_field_name("field")
            .map(|field| clean_name(text(field, source)))
            .unwrap_or_default(),
        "qualified_identifier" => node
            .child_by_field_name("name")
            .map(|name| clean_name(text(name, source)))
            .unwrap_or_default(),
        _ => clean_name(text(node, source))
            .rsplit("::")
            .next()
            .unwrap_or_default()
            .to_string(),
    }
}

fn declarator_name(node: Node<'_>, source: &str) -> String {
    match node.kind() {
        "identifier" | "field_identifier" | "type_identifier" | "destructor_name"
        | "operator_name" | "operator_cast" => clean_name(text(node, source)),
        "qualified_identifier" => clean_name(text(node, source)),
        _ => node
            .child_by_field_name("declarator")
            .map(|child| declarator_name(child, source))
            .or_else(|| {
                let mut cursor = node.walk();
                let result = node.named_children(&mut cursor).find_map(|child| {
                    let name = declarator_name(child, source);
                    (!name.is_empty()).then_some(name)
                });
                result
            })
            .unwrap_or_default(),
    }
}

fn declaration_declarators(node: Node<'_>) -> Vec<Node<'_>> {
    let mut result = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "init_declarator" | "identifier" | "pointer_declarator" | "array_declarator"
        ) {
            result.push(child);
        }
    }
    result
}

fn descendants_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Vec<Node<'tree>> {
    let mut found = Vec::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.kind() == kind {
            found.push(current);
            continue;
        }
        for index in 0..current.named_child_count() {
            if let Some(child) = current.named_child(index) {
                stack.push(child);
            }
        }
    }
    found
}

fn current_visibility(scopes: &[Scope], node: Node<'_>, source: &str) -> Visibility {
    if scopes
        .iter()
        .any(|scope| scope.kind == ScopeKind::Namespace && scope.name == "<anonymous>")
    {
        return Visibility::Private;
    }
    if let Some(scope) = scopes.last() {
        if matches!(scope.kind, ScopeKind::Class | ScopeKind::Struct) {
            return scope.visibility;
        }
    }
    if has_token(node, source, "static") {
        Visibility::Private
    } else {
        Visibility::Public
    }
}

fn has_token(node: Node<'_>, source: &str, token: &str) -> bool {
    if source.is_empty() {
        return false;
    }
    text(node, source)
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|part| part == token)
}

fn inside_class(scopes: &[Scope]) -> bool {
    scopes
        .iter()
        .any(|scope| matches!(scope.kind, ScopeKind::Class | ScopeKind::Struct))
}

fn inside_function(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == "function_definition" {
            return true;
        }
        node = parent;
    }
    false
}

fn qualify(scopes: &[Scope], name: &str) -> String {
    scopes
        .last()
        .map(|scope| format!("{}::{name}", scope.qualified))
        .unwrap_or_else(|| name.to_string())
}

fn clean_name(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn signature(node: Node<'_>, source: &str) -> String {
    if let Some(body) = node.child_by_field_name("body") {
        return source[node.start_byte()..body.start_byte()]
            .trim()
            .to_string();
    }
    text(node, source).trim().to_string()
}

fn doc_comment(node: Node<'_>, source: &str) -> Option<String> {
    let mut comments = Vec::new();
    let mut previous = node.prev_named_sibling();
    let mut expected_line = node.start_position().row;
    while let Some(comment) = previous {
        if comment.kind() != "comment" || comment.end_position().row + 1 < expected_line {
            break;
        }
        let raw = text(comment, source).trim();
        let parsed = if raw.starts_with("///") || raw.starts_with("//!") {
            Some(raw[3..].trim().to_string())
        } else if raw.starts_with("/**") || raw.starts_with("/*!") {
            Some(parse_block_doc_comment(raw))
        } else {
            None
        };
        let Some(parsed) = parsed else {
            break;
        };
        comments.push(parsed);
        expected_line = comment.start_position().row;
        previous = comment.prev_named_sibling();
    }
    comments.reverse();
    (!comments.is_empty()).then(|| comments.join("\n"))
}

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn outer_template(node: Node<'_>) -> Node<'_> {
    node.parent()
        .filter(|parent| parent.kind() == "template_declaration")
        .unwrap_or(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_selection_prefers_c_on_ties_and_cpp_for_cpp_syntax() {
        let mut parser = CCppParser::new();
        assert_eq!(
            parser
                .parse("plain.h", "int value;\n", Language::C)
                .unwrap()
                .language,
            "c"
        );
        assert_eq!(parser.parse("widget.h", "template <typename T> class Widget { public: virtual ~Widget() = default; };\n", Language::C).unwrap().language, "cpp");
    }

    #[test]
    fn extracts_c_symbols_calls_includes_docs_and_visibility() {
        let source = r#"
#include "local.h"
#include <vendor/api.h>
/// Adds two values.
int add(int a, int b);
static int hidden(int value) { return add(value, 1); }
typedef struct Point { int x; } Point;
union Value { int number; };
enum Mode { FAST };
const int LIMIT = 10;
int count = 0;
#define SCALE(v) ((v) * 2)
"#;
        let result = CCppParser::new()
            .parse("sample.c", source, Language::C)
            .unwrap();
        let symbol = |name: &str| {
            result
                .symbols
                .iter()
                .find(|symbol| symbol.name == name)
                .unwrap()
        };
        assert_eq!(symbol("add").kind, SymbolKind::Function);
        assert_eq!(symbol("add").docstring.as_deref(), Some("Adds two values."));
        assert_eq!(symbol("hidden").visibility, Visibility::Private);
        assert_eq!(symbol("Point").kind, SymbolKind::Struct);
        assert_eq!(symbol("Value").kind, SymbolKind::Type);
        assert_eq!(symbol("Mode").kind, SymbolKind::Enum);
        assert_eq!(symbol("LIMIT").kind, SymbolKind::Const);
        assert_eq!(symbol("SCALE").kind, SymbolKind::Macro);
        assert!(result.edges.iter().any(|edge| edge.target_name == "add"));
        let imports = &result.module.unwrap().imports;
        assert!(imports.iter().any(|import| import.from == "local.h"));
        assert!(imports.iter().any(|import| import.from == "<vendor/api.h>"));
    }

    #[test]
    fn extracts_cpp_namespaces_classes_methods_and_access() {
        let source = r#"
namespace api {
class Widget {
public:
    Widget();
    ~Widget();
    int run() const { return helper(); }
protected:
    void reset();
private:
    int helper();
};
struct Item { void use(); };
using Count = unsigned long;
}
api::Widget::Widget() {}
int api::Widget::helper() { return 1; }
api::Widget* make_widget() { return new api::Widget(); }
"#;
        let result = CCppParser::new()
            .parse("sample.cpp", source, Language::Cpp)
            .unwrap();
        let symbol = |name: &str| {
            result
                .symbols
                .iter()
                .find(|symbol| symbol.name == name)
                .unwrap()
        };
        assert_eq!(symbol("api").kind, SymbolKind::Module);
        assert_eq!(symbol("Widget").kind, SymbolKind::Class);
        assert_eq!(symbol("run").kind, SymbolKind::Method);
        assert_eq!(
            symbol("run").qualified_name.as_deref(),
            Some("api::Widget::run")
        );
        assert_eq!(symbol("reset").visibility, Visibility::Private);
        assert_eq!(symbol("helper").visibility, Visibility::Private);
        assert_eq!(symbol("use").visibility, Visibility::Public);
        assert_eq!(symbol("Count").kind, SymbolKind::Type);
        assert!(result
            .edges
            .iter()
            .any(|edge| edge.source_id.contains("make_widget") && edge.target_name == "Widget"));
    }
}
