//! Convert LSP responses into ctx's extraction contract
//! ([`crate::db::Symbol`] / [`crate::db::Edge`]).
//!
//! Provisional symbol ids use [`Symbol::make_id`] (no line suffix) exactly
//! like the tree-sitter parsers, so `store_symbols` rewrites them into the
//! canonical `path::[parent::]name@line` form through the same id map.

use lsp_types::{
    CallHierarchyOutgoingCall, DocumentSymbol, DocumentSymbolResponse, SymbolInformation,
    SymbolKind as LspSymbolKind,
};

use crate::db::{Edge, EdgeKind, Symbol, SymbolKind, Visibility};

/// A converted symbol plus the LSP selection position (0-based) needed for
/// follow-up requests such as `prepareCallHierarchy`.
#[derive(Debug, Clone)]
pub struct ExtractedSymbol {
    pub symbol: Symbol,
    /// 0-based line of the symbol's selection range (its name).
    pub sel_line: u32,
    /// 0-based character of the symbol's selection range.
    pub sel_col: u32,
}

/// Map an LSP symbol kind to a ctx [`SymbolKind`].
///
/// Variable-like kinds map to [`SymbolKind::Variable`]; the callers skip
/// those when they are nested inside a function/method body (locals).
/// Unknown kinds return `None` and are dropped.
fn map_kind(kind: LspSymbolKind) -> Option<SymbolKind> {
    Some(match kind {
        LspSymbolKind::FILE
        | LspSymbolKind::MODULE
        | LspSymbolKind::NAMESPACE
        | LspSymbolKind::PACKAGE => SymbolKind::Module,
        LspSymbolKind::CLASS => SymbolKind::Class,
        LspSymbolKind::METHOD | LspSymbolKind::CONSTRUCTOR | LspSymbolKind::OPERATOR => {
            SymbolKind::Method
        }
        LspSymbolKind::PROPERTY | LspSymbolKind::FIELD | LspSymbolKind::EVENT => SymbolKind::Field,
        LspSymbolKind::ENUM => SymbolKind::Enum,
        LspSymbolKind::ENUM_MEMBER => SymbolKind::Variant,
        LspSymbolKind::INTERFACE => SymbolKind::Interface,
        LspSymbolKind::FUNCTION => SymbolKind::Function,
        LspSymbolKind::CONSTANT => SymbolKind::Const,
        LspSymbolKind::STRUCT => SymbolKind::Struct,
        LspSymbolKind::TYPE_PARAMETER => SymbolKind::Type,
        LspSymbolKind::VARIABLE
        | LspSymbolKind::STRING
        | LspSymbolKind::NUMBER
        | LspSymbolKind::BOOLEAN
        | LspSymbolKind::ARRAY
        | LspSymbolKind::OBJECT
        | LspSymbolKind::KEY
        | LspSymbolKind::NULL => SymbolKind::Variable,
        _ => return None,
    })
}

/// Variable-like kinds are locals when nested inside a function/method and
/// must be skipped.
fn is_variable_like(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Variable)
}

fn is_function_like(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Function | SymbolKind::Method)
}

/// Convert a `textDocument/documentSymbol` response into symbols.
pub fn symbols_from_response(
    rel_path: &str,
    text: &str,
    response: DocumentSymbolResponse,
) -> Vec<ExtractedSymbol> {
    match response {
        DocumentSymbolResponse::Nested(roots) => {
            let lines: Vec<&str> = text.lines().collect();
            let mut out = Vec::new();
            for root in &roots {
                convert_nested(rel_path, &lines, root, &[], &mut out);
            }
            out
        }
        DocumentSymbolResponse::Flat(infos) => convert_flat(rel_path, text, &infos),
    }
}

/// Recursively convert a hierarchical [`DocumentSymbol`] subtree.
///
/// `ancestors` is the chain of (name, ctx-kind) pairs above this node.
fn convert_nested(
    rel_path: &str,
    lines: &[&str],
    node: &DocumentSymbol,
    ancestors: &[(String, SymbolKind)],
    out: &mut Vec<ExtractedSymbol>,
) {
    let Some(kind) = map_kind(node.kind) else {
        return;
    };

    // Skip locals: variable-like symbols nested inside a function/method
    // body (and their subtrees) are not indexed.
    let inside_function = ancestors
        .last()
        .map(|(_, k)| is_function_like(*k))
        .unwrap_or(false);
    if is_variable_like(kind) && inside_function {
        return;
    }

    let name = node.name.trim().to_string();
    if name.is_empty() {
        return;
    }

    let parent_name = ancestors.last().map(|(n, _)| n.as_str());
    let id = Symbol::make_id(rel_path, &name, parent_name);
    let parent_id = parent_name.map(|p| Symbol::make_id(rel_path, p, None));

    // Qualified name: parent chain + own name joined with "." (None at the
    // top level, mirroring the tree-sitter extractors).
    let qualified_name = if ancestors.is_empty() {
        None
    } else {
        let mut parts: Vec<&str> = ancestors.iter().map(|(n, _)| n.as_str()).collect();
        parts.push(&name);
        Some(parts.join("."))
    };

    // LSP positions are 0-based; ctx lines are 1-based (columns stay 0-based).
    let line_start = node.range.start.line + 1;
    let line_end = node.range.end.line + 1;

    let signature = node
        .detail
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string);

    let source = slice_lines(lines, node.range.start.line, node.range.end.line);

    let visibility = if name.starts_with('_') {
        Visibility::Private
    } else {
        Visibility::Public
    };

    out.push(ExtractedSymbol {
        symbol: Symbol {
            id,
            file_path: rel_path.to_string(),
            name: name.clone(),
            qualified_name,
            kind,
            visibility,
            signature,
            brief: None,
            docstring: None,
            line_start,
            line_end,
            col_start: node.range.start.character,
            col_end: node.range.end.character,
            parent_id,
            source,
        },
        sel_line: node.selection_range.start.line,
        sel_col: node.selection_range.start.character,
    });

    if let Some(children) = &node.children {
        let mut chain = ancestors.to_vec();
        chain.push((name, kind));
        for child in children {
            convert_nested(rel_path, lines, child, &chain, out);
        }
    }
}

/// Convert a flat [`SymbolInformation`] list (servers without hierarchical
/// document symbols). Containers are only known by name (`container_name`),
/// so nesting is approximated through the parent name alone.
fn convert_flat(rel_path: &str, text: &str, infos: &[SymbolInformation]) -> Vec<ExtractedSymbol> {
    let lines: Vec<&str> = text.lines().collect();

    // Names of function-like symbols, to apply the local-variable skip rule.
    let function_names: Vec<&str> = infos
        .iter()
        .filter(|i| map_kind(i.kind).map(is_function_like).unwrap_or(false))
        .map(|i| i.name.as_str())
        .collect();

    let mut out = Vec::new();
    for info in infos {
        let Some(kind) = map_kind(info.kind) else {
            continue;
        };

        let container = info
            .container_name
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty());

        if is_variable_like(kind) {
            if let Some(container) = container {
                if function_names.contains(&container) {
                    continue; // local inside a function/method
                }
            }
        }

        let name = info.name.trim().to_string();
        if name.is_empty() {
            continue;
        }

        let id = Symbol::make_id(rel_path, &name, container);
        let parent_id = container.map(|p| Symbol::make_id(rel_path, p, None));
        let qualified_name = container.map(|c| format!("{c}.{name}"));

        let range = info.location.range;
        let visibility = if name.starts_with('_') {
            Visibility::Private
        } else {
            Visibility::Public
        };

        out.push(ExtractedSymbol {
            symbol: Symbol {
                id,
                file_path: rel_path.to_string(),
                name,
                qualified_name,
                kind,
                visibility,
                signature: None,
                brief: None,
                docstring: None,
                line_start: range.start.line + 1,
                line_end: range.end.line + 1,
                col_start: range.start.character,
                col_end: range.end.character,
                parent_id,
                source: slice_lines(&lines, range.start.line, range.end.line),
            },
            sel_line: range.start.line,
            sel_col: range.start.character,
        });
    }
    out
}

/// Inclusive 0-based line slice of the file text.
fn slice_lines(lines: &[&str], start: u32, end: u32) -> Option<String> {
    let start = start as usize;
    let end = (end as usize).min(lines.len().saturating_sub(1));
    if start >= lines.len() || start > end {
        return None;
    }
    Some(lines[start..=end].join("\n"))
}

/// Convert `callHierarchy/outgoingCalls` results for one source symbol into
/// `Calls` edges.
///
/// Same-file targets are wired to the target's provisional id (rewritten by
/// `store_symbols` through the shared id map). Cross-file targets stay
/// unresolved (`target_id: None`) and carry the call-site line/col so Stage B
/// can resolve them with `textDocument/definition`. They must never be routed
/// through `store_edges`' path rewriting.
pub fn edges_from_outgoing_calls(
    source: &ExtractedSymbol,
    file_uri: &str,
    file_symbols: &[ExtractedSymbol],
    calls: &[CallHierarchyOutgoingCall],
) -> Vec<Edge> {
    let mut edges = Vec::new();

    for call in calls {
        let target_name = call.to.name.trim().to_string();
        if target_name.is_empty() {
            continue;
        }

        // Same-file target: match the extracted symbol whose selection range
        // starts where the call-hierarchy item's does.
        let same_file = call.to.uri.as_str() == file_uri;
        let target_id = if same_file {
            file_symbols
                .iter()
                .find(|s| {
                    s.sel_line == call.to.selection_range.start.line
                        && s.sel_col == call.to.selection_range.start.character
                })
                .or_else(|| {
                    file_symbols.iter().find(|s| {
                        s.symbol.name == target_name
                            && s.symbol.line_start == call.to.range.start.line + 1
                    })
                })
                .map(|s| s.symbol.id.clone())
        } else {
            None
        };

        for range in &call.from_ranges {
            edges.push(Edge {
                source_id: source.symbol.id.clone(),
                target_id: target_id.clone(),
                target_name: target_name.clone(),
                kind: EdgeKind::Calls,
                line: Some(range.start.line + 1),
                col: Some(range.start.character),
                context: Some(target_name.clone()),
            });
        }

        // Some servers return no from_ranges; still record one edge so the
        // relationship isn't lost (without a call-site location it cannot be
        // Stage-B resolved, but same-file targets are already resolved).
        if call.from_ranges.is_empty() {
            edges.push(Edge {
                source_id: source.symbol.id.clone(),
                target_id: target_id.clone(),
                target_name: target_name.clone(),
                kind: EdgeKind::Calls,
                line: None,
                col: None,
                context: Some(target_name.clone()),
            });
        }
    }

    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const KOTLIN_FIXTURE: &str = "class Greeter {\n    val name = \"x\"\n    fun greet(who: String): String {\n        val message = \"hi\"\n        return message + who\n    }\n}\nfun _hidden() {}\nfun top() {}\n";

    /// A hierarchical documentSymbol response for `KOTLIN_FIXTURE`.
    fn nested_response() -> DocumentSymbolResponse {
        serde_json::from_value(json!([
            {
                "name": "Greeter",
                "kind": 5, // Class
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 6, "character": 1 } },
                "selectionRange": { "start": { "line": 0, "character": 6 }, "end": { "line": 0, "character": 13 } },
                "children": [
                    {
                        "name": "name",
                        "kind": 8, // Field
                        "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 18 } },
                        "selectionRange": { "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 12 } }
                    },
                    {
                        "name": "greet",
                        "detail": "fun greet(who: String): String",
                        "kind": 6, // Method
                        "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 5, "character": 5 } },
                        "selectionRange": { "start": { "line": 2, "character": 8 }, "end": { "line": 2, "character": 13 } },
                        "children": [
                            {
                                "name": "message",
                                "kind": 13, // Variable -> local, skipped
                                "range": { "start": { "line": 3, "character": 8 }, "end": { "line": 3, "character": 28 } },
                                "selectionRange": { "start": { "line": 3, "character": 12 }, "end": { "line": 3, "character": 19 } }
                            }
                        ]
                    }
                ]
            },
            {
                "name": "_hidden",
                "kind": 12, // Function
                "range": { "start": { "line": 7, "character": 0 }, "end": { "line": 7, "character": 16 } },
                "selectionRange": { "start": { "line": 7, "character": 4 }, "end": { "line": 7, "character": 11 } }
            },
            {
                "name": "top",
                "kind": 12, // Function
                "range": { "start": { "line": 8, "character": 0 }, "end": { "line": 8, "character": 12 } },
                "selectionRange": { "start": { "line": 8, "character": 4 }, "end": { "line": 8, "character": 7 } }
            }
        ]))
        .map(DocumentSymbolResponse::Nested)
        .unwrap()
    }

    #[test]
    fn nested_symbols_convert_with_hierarchy_and_line_conversion() {
        let extracted = symbols_from_response("src/main.kt", KOTLIN_FIXTURE, nested_response());
        let names: Vec<&str> = extracted.iter().map(|e| e.symbol.name.as_str()).collect();
        assert_eq!(names, vec!["Greeter", "name", "greet", "_hidden", "top"]);

        let class = &extracted[0].symbol;
        assert_eq!(class.kind, SymbolKind::Class);
        assert_eq!(class.line_start, 1, "0-based LSP line 0 -> ctx line 1");
        assert_eq!(class.line_end, 7);
        assert_eq!(class.id, "src/main.kt::Greeter");
        assert!(class.parent_id.is_none());
        assert!(class.qualified_name.is_none(), "top level has no chain");
        assert_eq!(class.visibility, Visibility::Public);
        assert!(class
            .source
            .as_deref()
            .unwrap()
            .starts_with("class Greeter"));

        let field = &extracted[1].symbol;
        assert_eq!(field.kind, SymbolKind::Field);
        assert_eq!(field.parent_id.as_deref(), Some("src/main.kt::Greeter"));
        assert_eq!(field.qualified_name.as_deref(), Some("Greeter.name"));

        let method = &extracted[2].symbol;
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.id, "src/main.kt::Greeter::greet");
        assert_eq!(method.qualified_name.as_deref(), Some("Greeter.greet"));
        assert_eq!(
            method.signature.as_deref(),
            Some("fun greet(who: String): String")
        );
        assert_eq!(method.line_start, 3);
        assert_eq!(method.line_end, 6);
        assert_eq!(extracted[2].sel_line, 2);
        assert_eq!(extracted[2].sel_col, 8);

        // The local `message` variable inside the method was skipped.
        assert!(!names.contains(&"message"));

        // Leading underscore -> private.
        assert_eq!(extracted[3].symbol.visibility, Visibility::Private);
    }

    #[test]
    fn kind_mapping_table() {
        let cases: Vec<(i32, Option<SymbolKind>)> = vec![
            (1, Some(SymbolKind::Module)), // File
            (2, Some(SymbolKind::Module)), // Module
            (3, Some(SymbolKind::Module)), // Namespace
            (4, Some(SymbolKind::Module)), // Package
            (5, Some(SymbolKind::Class)),  // Class
            (6, Some(SymbolKind::Method)), // Method
            (7, Some(SymbolKind::Field)),  // Property
            (8, Some(SymbolKind::Field)),  // Field
            (9, Some(SymbolKind::Method)), // Constructor
            (10, Some(SymbolKind::Enum)),  // Enum
            (11, Some(SymbolKind::Interface)),
            (12, Some(SymbolKind::Function)),
            (13, Some(SymbolKind::Variable)),
            (14, Some(SymbolKind::Const)),    // Constant
            (15, Some(SymbolKind::Variable)), // String
            (19, Some(SymbolKind::Variable)), // Object
            (22, Some(SymbolKind::Variant)),  // EnumMember
            (23, Some(SymbolKind::Struct)),   // Struct
            (24, Some(SymbolKind::Field)),    // Event
            (25, Some(SymbolKind::Method)),   // Operator
            (26, Some(SymbolKind::Type)),     // TypeParameter
        ];
        for (raw, expected) in cases {
            let kind: LspSymbolKind = serde_json::from_value(json!(raw)).unwrap();
            assert_eq!(map_kind(kind), expected, "LSP kind {raw}");
        }
    }

    #[test]
    fn flat_symbol_information_fallback() {
        let infos: Vec<SymbolInformation> = serde_json::from_value(json!([
            {
                "name": "Greeter",
                "kind": 5,
                "location": {
                    "uri": "file:///w/src/main.kt",
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 6, "character": 1 } }
                }
            },
            {
                "name": "greet",
                "kind": 6,
                "containerName": "Greeter",
                "location": {
                    "uri": "file:///w/src/main.kt",
                    "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 5, "character": 5 } }
                }
            },
            {
                "name": "message",
                "kind": 13,
                "containerName": "greet",
                "location": {
                    "uri": "file:///w/src/main.kt",
                    "range": { "start": { "line": 3, "character": 8 }, "end": { "line": 3, "character": 28 } }
                }
            }
        ]))
        .unwrap();

        let extracted = symbols_from_response(
            "src/main.kt",
            KOTLIN_FIXTURE,
            DocumentSymbolResponse::Flat(infos),
        );
        let names: Vec<&str> = extracted.iter().map(|e| e.symbol.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Greeter", "greet"],
            "local skipped in flat mode"
        );
        assert_eq!(extracted[1].symbol.id, "src/main.kt::Greeter::greet");
        assert_eq!(
            extracted[1].symbol.qualified_name.as_deref(),
            Some("Greeter.greet")
        );
        assert_eq!(extracted[1].symbol.line_start, 3);
    }

    #[test]
    fn outgoing_calls_become_edges() {
        let extracted = symbols_from_response("src/main.kt", KOTLIN_FIXTURE, nested_response());
        let greet = extracted
            .iter()
            .find(|e| e.symbol.name == "greet")
            .unwrap()
            .clone();

        let calls: Vec<CallHierarchyOutgoingCall> = serde_json::from_value(json!([
            {
                // Same-file target: `top`, matched by selection position.
                "to": {
                    "name": "top",
                    "kind": 12,
                    "uri": "file:///w/src/main.kt",
                    "range": { "start": { "line": 8, "character": 0 }, "end": { "line": 8, "character": 12 } },
                    "selectionRange": { "start": { "line": 8, "character": 4 }, "end": { "line": 8, "character": 7 } }
                },
                "fromRanges": [
                    { "start": { "line": 4, "character": 15 }, "end": { "line": 4, "character": 18 } }
                ]
            },
            {
                // Cross-file target: stays unresolved for Stage B.
                "to": {
                    "name": "helper",
                    "kind": 12,
                    "uri": "file:///w/src/util.kt",
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 2, "character": 1 } },
                    "selectionRange": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 10 } }
                },
                "fromRanges": [
                    { "start": { "line": 3, "character": 22 }, "end": { "line": 3, "character": 28 } }
                ]
            }
        ]))
        .unwrap();

        let edges = edges_from_outgoing_calls(&greet, "file:///w/src/main.kt", &extracted, &calls);
        assert_eq!(edges.len(), 2);

        let same_file = &edges[0];
        assert_eq!(same_file.source_id, "src/main.kt::Greeter::greet");
        assert_eq!(same_file.target_id.as_deref(), Some("src/main.kt::top"));
        assert_eq!(same_file.target_name, "top");
        assert_eq!(same_file.kind, EdgeKind::Calls);
        assert_eq!(same_file.line, Some(5), "0-based from_range line 4 -> 5");
        assert_eq!(same_file.col, Some(15));

        let cross_file = &edges[1];
        assert!(
            cross_file.target_id.is_none(),
            "cross-file stays unresolved"
        );
        assert_eq!(cross_file.target_name, "helper");
        assert_eq!(cross_file.line, Some(4));
    }
}
