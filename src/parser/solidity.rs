//! Solidity-specific code parsing using tree-sitter.

use tree_sitter::{Node, Parser, Query, QueryCursor};

use crate::db::{
    Edge, EdgeKind, ImportInfo, ModuleInfo, ParseResult, Symbol, SymbolKind, Visibility,
};
use crate::parser::{extract_brief, find_symbol_kind, is_def_capture, SymbolKindMapping};

/// Symbol kind mappings for Solidity capture names.
const SOLIDITY_SYMBOL_MAPPINGS: &[SymbolKindMapping] = &[
    SymbolKindMapping::new("contract", SymbolKind::Class),
    SymbolKindMapping::new("interface", SymbolKind::Interface),
    SymbolKindMapping::new("library", SymbolKind::Module),
    SymbolKindMapping::new("func", SymbolKind::Function),
    SymbolKindMapping::new("modifier", SymbolKind::Function),
    SymbolKindMapping::new("event", SymbolKind::Function),
    SymbolKindMapping::new("error", SymbolKind::Type),
    SymbolKindMapping::new("struct", SymbolKind::Struct),
    SymbolKindMapping::new("enum", SymbolKind::Enum),
    SymbolKindMapping::new("statevar", SymbolKind::Field),
];

/// Capture prefixes that represent container types (contract, interface, library).
const SOLIDITY_CONTAINER_PREFIXES: &[&str] = &["contract", "interface", "library"];

/// Solidity-specific parser.
pub struct SolidityParser {
    parser: Parser,
    symbols_query: Query,
    calls_query: Query,
}

impl SolidityParser {
    /// Create a new Solidity parser.
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language = tree_sitter_solidity::language();
        parser
            .set_language(language)
            .expect("Failed to set Solidity language");

        // Query for extracting symbols (contracts, functions, events, etc.)
        // Uses tree-sitter-solidity v1.2 grammar node names
        let symbols_query = Query::new(
            language,
            r#"
            ; Contracts
            (contract_declaration
                name: (identifier) @contract.name
            ) @contract.def

            ; Interfaces
            (interface_declaration
                name: (identifier) @interface.name
            ) @interface.def

            ; Libraries
            (library_declaration
                name: (identifier) @library.name
            ) @library.def

            ; Functions
            (function_definition
                name: (identifier) @func.name
            ) @func.def

            ; Constructor
            (constructor_definition) @constructor.def

            ; Modifiers
            (modifier_definition
                name: (identifier) @modifier.name
            ) @modifier.def

            ; Events
            (event_definition
                name: (identifier) @event.name
            ) @event.def

            ; Structs
            (struct_declaration
                name: (identifier) @struct.name
            ) @struct.def

            ; Enums
            (enum_declaration
                name: (identifier) @enum.name
            ) @enum.def

            ; State variables
            (state_variable_declaration
                name: (identifier) @statevar.name
            ) @statevar.def
            "#,
        )
        .expect("Invalid Solidity symbols query");

        // Query for extracting function calls
        // Use simple identifier matching for now
        let calls_query = Query::new(
            language,
            r#"
            ; Any identifier that might be a call
            (identifier) @call.name
            "#,
        )
        .expect("Invalid Solidity calls query");

        Self {
            parser,
            symbols_query,
            calls_query,
        }
    }

    /// Parse a Solidity source file.
    pub fn parse(&mut self, file_path: &str, source: &str) -> Option<ParseResult> {
        let tree = self.parser.parse(source, None)?;
        let root = tree.root_node();

        let mut symbols = Vec::new();
        let mut edges = Vec::new();
        let mut imports = Vec::new();

        // Extract symbols
        self.extract_symbols(&root, file_path, source, &mut symbols, &mut imports);

        // Extract edges (calls)
        self.extract_edges(&root, file_path, source, &symbols, &mut edges);

        let module = ModuleInfo {
            file_path: file_path.to_string(),
            module_name: self.extract_contract_name(source),
            exports: symbols
                .iter()
                .filter(|s| s.visibility == Visibility::Public)
                .map(|s| s.name.clone())
                .collect(),
            imports,
        };

        Some(ParseResult {
            file_path: file_path.to_string(),
            language: "solidity".to_string(),
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
        imports: &mut Vec<ImportInfo>,
    ) {
        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(&self.symbols_query, *root, source.as_bytes());

        // Track current contract/interface/library for parent relationships
        let mut current_parent: Option<String> = None;

        for m in matches {
            let mut name: Option<&str> = None;
            let mut kind: Option<SymbolKind> = None;
            let mut def_node: Option<Node> = None;
            let mut type_info: Option<&str> = None;

            for capture in m.captures {
                let capture_name = &self.symbols_query.capture_names()[capture.index as usize];
                let capture_str = capture_name.as_str();
                let node = capture.node;
                let text = node.utf8_text(source.as_bytes()).unwrap_or("");

                // Try to match standard symbol patterns
                if let Some(k) = find_symbol_kind(capture_str, SOLIDITY_SYMBOL_MAPPINGS) {
                    name = Some(text);
                    kind = Some(k);
                    // Track current parent for container types
                    let prefix = capture_str.trim_end_matches(".name");
                    if SOLIDITY_CONTAINER_PREFIXES.contains(&prefix) {
                        current_parent = Some(text.to_string());
                    }
                } else {
                    // Handle special cases
                    match capture_str {
                        // Constructor (no .name capture)
                        "constructor.def" => {
                            name = Some("constructor");
                            kind = Some(SymbolKind::Function);
                            def_node = Some(node);
                        }
                        // State variable type
                        "statevar.type" => type_info = Some(text),
                        // Imports
                        "import.source" => {
                            let source_path = text.trim_matches('"').trim_matches('\'');
                            imports.push(ImportInfo {
                                from: source_path.to_string(),
                                names: Vec::new(),
                                alias: None,
                            });
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
                let visibility = extract_solidity_visibility(&node, source);
                let docstring = extract_natspec(&node, source);
                let brief = docstring.as_ref().and_then(|d| extract_brief(d));

                let signature = build_solidity_signature(kind, name, type_info, source, &node);

                // Determine parent based on context
                let parent_name = if matches!(
                    kind,
                    SymbolKind::Class | SymbolKind::Interface | SymbolKind::Module
                ) {
                    None
                } else {
                    current_parent.as_deref()
                };

                let parent_id = parent_name.map(|p| Symbol::make_id(file_path, p, None));
                let symbol_source = node.utf8_text(source.as_bytes()).ok().map(String::from);

                let id = Symbol::make_id(file_path, name, parent_name);

                symbols.push(Symbol {
                    id,
                    file_path: file_path.to_string(),
                    name: name.to_string(),
                    qualified_name: parent_name.map(|p| format!("{}.{}", p, name)),
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
    /// Note: Edge extraction for Solidity is simplified due to grammar complexity.
    /// We look for identifiers that match known function names within function bodies.
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
            .filter(|s| s.kind == SymbolKind::Function)
            .map(|s| (s.line_start, s.line_end, s.id.clone()))
            .collect();

        // Build a set of known function/event names for matching
        let known_callables: std::collections::HashSet<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .map(|s| s.name.as_str())
            .collect();

        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(&self.calls_query, *root, source.as_bytes());

        for m in matches {
            for capture in m.captures {
                let node = capture.node;
                let text = node.utf8_text(source.as_bytes()).unwrap_or("");

                // Only consider identifiers that match known function names
                if !known_callables.contains(text) {
                    continue;
                }

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
                        .find(|s| s.name == text)
                        .map(|s| s.id.clone());

                    edges.push(Edge {
                        source_id,
                        target_id,
                        target_name: text.to_string(),
                        kind: EdgeKind::Calls,
                        line: Some(line),
                        col: Some(col),
                        context: Some(text.to_string()),
                    });
                }
            }
        }
    }

    /// Extract the main contract name from source.
    fn extract_contract_name(&self, source: &str) -> Option<String> {
        // Simple heuristic: find first contract/interface/library declaration
        for line in source.lines() {
            let trimmed = line.trim();
            for keyword in &["contract ", "interface ", "library "] {
                if trimmed.starts_with(keyword) {
                    let rest = &trimmed[keyword.len()..];
                    let name = rest
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                        .filter(|s| !s.is_empty());
                    if let Some(name) = name {
                        return Some(name.to_string());
                    }
                }
            }
        }
        None
    }
}

/// Extract visibility from a Solidity node.
fn extract_solidity_visibility(node: &Node, source: &str) -> Visibility {
    let text = node.utf8_text(source.as_bytes()).unwrap_or("");

    // Check for visibility keywords in the node text
    if text.contains("public") {
        Visibility::Public
    } else if text.contains("external") {
        Visibility::Public // external is similar to public for our purposes
    } else if text.contains("internal") {
        Visibility::Crate // internal is like crate-level visibility
    } else if text.contains("private") {
        Visibility::Private
    } else {
        // Default visibility in Solidity depends on context
        // State variables default to internal, functions to public in older versions
        Visibility::Private
    }
}

/// Extract NatSpec documentation from a Solidity node.
fn extract_natspec(node: &Node, source: &str) -> Option<String> {
    let mut doc_lines = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        let text = sibling.utf8_text(source.as_bytes()).unwrap_or("");

        if sibling.kind() == "comment" {
            if text.starts_with("///") {
                // Single-line NatSpec
                let content = text.trim_start_matches("///").trim();
                doc_lines.push(content.to_string());
            } else if text.starts_with("/**") {
                // Multi-line NatSpec
                doc_lines.push(super::parse_block_doc_comment(text));
                break;
            } else {
                break;
            }
        } else if sibling.kind() != "comment" {
            break;
        }

        prev = sibling.prev_sibling();
    }

    if doc_lines.is_empty() {
        return None;
    }

    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}

/// Build a signature string for a Solidity symbol.
fn build_solidity_signature(
    kind: SymbolKind,
    name: &str,
    type_info: Option<&str>,
    source: &str,
    node: &Node,
) -> Option<String> {
    match kind {
        SymbolKind::Function => {
            // Get the function signature up to the body
            let text = node.utf8_text(source.as_bytes()).ok()?;
            // Find where the body starts (first '{')
            if let Some(idx) = text.find('{') {
                let sig = text[..idx].trim();
                // Clean up multi-line signatures
                let sig = sig.lines().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
                Some(sig)
            } else {
                // Interface function (no body)
                Some(text.lines().next()?.trim().to_string())
            }
        }
        SymbolKind::Field => {
            // State variable
            if let Some(type_str) = type_info {
                Some(format!("{} {}", type_str, name))
            } else {
                None
            }
        }
        SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Class | SymbolKind::Interface => {
            // Get the first line
            let text = node.utf8_text(source.as_bytes()).ok()?;
            let first_line = text.lines().next()?.trim();
            Some(first_line.trim_end_matches('{').trim().to_string())
        }
        _ => None,
    }
}

impl Default for SolidityParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_contract() {
        let mut parser = SolidityParser::new();
        let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title A simple token contract
/// @notice This contract implements a basic token
contract SimpleToken {
    string public name;
    uint256 public totalSupply;

    /// @notice Transfer tokens to a recipient
    /// @param to The recipient address
    /// @param amount The amount to transfer
    function transfer(address to, uint256 amount) public returns (bool) {
        return true;
    }
}
"#;

        let result = parser.parse("Token.sol", source).unwrap();

        // Should have contract + state vars + function
        assert!(result.symbols.len() >= 3);

        let contract = result.symbols.iter().find(|s| s.name == "SimpleToken");
        assert!(contract.is_some());
        assert_eq!(contract.unwrap().kind, SymbolKind::Class);

        let transfer = result.symbols.iter().find(|s| s.name == "transfer");
        assert!(transfer.is_some());
        assert_eq!(transfer.unwrap().kind, SymbolKind::Function);
        assert_eq!(transfer.unwrap().visibility, Visibility::Public);
    }

    #[test]
    fn test_parse_interface() {
        let mut parser = SolidityParser::new();
        let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IERC20 {
    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
}
"#;

        let result = parser.parse("IERC20.sol", source).unwrap();

        let interface = result.symbols.iter().find(|s| s.name == "IERC20");
        assert!(interface.is_some());
        assert_eq!(interface.unwrap().kind, SymbolKind::Interface);

        // Should have interface + 3 functions
        let functions: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(functions.len(), 3);
    }

    #[test]
    fn test_parse_struct_and_enum() {
        let mut parser = SolidityParser::new();
        let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Test {
    struct User {
        address addr;
        uint256 balance;
    }

    enum Status {
        Pending,
        Active,
        Completed
    }
}
"#;

        let result = parser.parse("Test.sol", source).unwrap();

        let user_struct = result.symbols.iter().find(|s| s.name == "User");
        assert!(user_struct.is_some());
        assert_eq!(user_struct.unwrap().kind, SymbolKind::Struct);

        let status_enum = result.symbols.iter().find(|s| s.name == "Status");
        assert!(status_enum.is_some());
        assert_eq!(status_enum.unwrap().kind, SymbolKind::Enum);
    }
}
