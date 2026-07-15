//! TypeScript/JavaScript/JSX/TSX code parsing using tree-sitter.

use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::db::{
    Edge, EdgeKind, ImportInfo, ModuleInfo, ParseResult, Symbol, SymbolKind, Visibility,
};
use crate::parser::{
    extract_brief, extract_call_edges, find_symbol_kind, is_def_capture, CallCapturePatterns,
    SymbolKindMapping,
};

/// Symbol kind mappings for TypeScript/JavaScript capture names.
const TS_SYMBOL_MAPPINGS: &[SymbolKindMapping] = &[
    SymbolKindMapping::new("func", SymbolKind::Function),
    SymbolKindMapping::new("arrow", SymbolKind::Function),
    SymbolKindMapping::new("funcexpr", SymbolKind::Function),
    SymbolKindMapping::new("class", SymbolKind::Class),
    SymbolKindMapping::new("method", SymbolKind::Method),
    SymbolKindMapping::new("interface", SymbolKind::Interface),
    SymbolKindMapping::new("type", SymbolKind::Type),
    SymbolKindMapping::new("enum", SymbolKind::Enum),
];

/// Variant of JS/TS language being parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsVariant {
    JavaScript,
    TypeScript,
    Jsx,
    Tsx,
}

impl JsVariant {
    pub fn as_str(&self) -> &'static str {
        match self {
            JsVariant::JavaScript => "javascript",
            JsVariant::TypeScript => "typescript",
            JsVariant::Jsx => "jsx",
            JsVariant::Tsx => "tsx",
        }
    }
}

/// TypeScript/JavaScript parser supporting JS, TS, JSX, and TSX.
pub struct TypeScriptParser {
    js_parser: Parser,
    ts_parser: Parser,
    tsx_parser: Parser,
    js_language: Language,
    ts_language: Language,
    tsx_language: Language,
}

impl TypeScriptParser {
    /// Create a new TypeScript/JavaScript parser.
    pub fn new() -> Self {
        let js_language: Language = tree_sitter_javascript::LANGUAGE.into();
        let ts_language: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let tsx_language: Language = tree_sitter_typescript::LANGUAGE_TSX.into();

        let mut js_parser = Parser::new();
        js_parser
            .set_language(&js_language)
            .expect("Failed to set JavaScript language");

        let mut ts_parser = Parser::new();
        ts_parser
            .set_language(&ts_language)
            .expect("Failed to set TypeScript language");

        let mut tsx_parser = Parser::new();
        tsx_parser
            .set_language(&tsx_language)
            .expect("Failed to set TSX language");

        Self {
            js_parser,
            ts_parser,
            tsx_parser,
            js_language,
            ts_language,
            tsx_language,
        }
    }

    /// Get the appropriate parser and language for a variant.
    fn get_parser_and_language(&mut self, variant: JsVariant) -> (&mut Parser, &Language) {
        match variant {
            JsVariant::JavaScript | JsVariant::Jsx => (&mut self.js_parser, &self.js_language),
            JsVariant::TypeScript => (&mut self.ts_parser, &self.ts_language),
            JsVariant::Tsx => (&mut self.tsx_parser, &self.tsx_language),
        }
    }

    /// Create the symbols query for a given language.
    /// Note: Uses different queries for JS vs TS since they have different node types.
    fn create_symbols_query(language: &Language, is_typescript: bool) -> Query {
        if is_typescript {
            // TypeScript/TSX query - includes interfaces, type aliases, enums
            Query::new(
                language,
                r#"
                ; Functions
                (function_declaration
                    name: (identifier) @func.name
                ) @func.def

                ; Arrow functions assigned to variables
                (lexical_declaration
                    (variable_declarator
                        name: (identifier) @arrow.name
                        value: (arrow_function)
                    )
                ) @arrow.def

                ; Classes
                (class_declaration
                    name: (type_identifier) @class.name
                ) @class.def

                ; Methods in classes
                (method_definition
                    name: (property_identifier) @method.name
                ) @method.def

                ; Interfaces (TypeScript)
                (interface_declaration
                    name: (type_identifier) @interface.name
                ) @interface.def

                ; Type aliases (TypeScript)
                (type_alias_declaration
                    name: (type_identifier) @type.name
                ) @type.def

                ; Enums (TypeScript)
                (enum_declaration
                    name: (identifier) @enum.name
                ) @enum.def

                ; Export statements
                (export_statement) @export.def

                ; Import statements
                (import_statement
                    source: (string) @import.source
                ) @import.def
                "#,
            )
            .expect("Invalid TypeScript symbols query")
        } else {
            // JavaScript/JSX query - no interfaces, type aliases, or enums
            Query::new(
                language,
                r#"
                ; Functions
                (function_declaration
                    name: (identifier) @func.name
                ) @func.def

                ; Arrow functions assigned to variables
                (lexical_declaration
                    (variable_declarator
                        name: (identifier) @arrow.name
                        value: (arrow_function)
                    )
                ) @arrow.def

                ; Classes
                (class_declaration
                    name: (identifier) @class.name
                ) @class.def

                ; Methods in classes
                (method_definition
                    name: (property_identifier) @method.name
                ) @method.def

                ; Export statements
                (export_statement) @export.def

                ; Import statements
                (import_statement
                    source: (string) @import.source
                ) @import.def
                "#,
            )
            .expect("Invalid JavaScript symbols query")
        }
    }

    /// Create the calls query for a given language.
    fn create_calls_query(language: &Language) -> Query {
        Query::new(
            language,
            r#"
            ; Function calls
            (call_expression
                function: (identifier) @call.name
            ) @call.expr

            ; Method calls
            (call_expression
                function: (member_expression
                    property: (property_identifier) @method_call.name
                )
            ) @method_call.expr

            ; New expressions
            (new_expression
                constructor: (identifier) @new.name
            ) @new.expr
            "#,
        )
        .expect("Invalid TypeScript calls query")
    }

    /// Create the inheritance query for a given language.
    fn create_inheritance_query(language: &Language, is_typescript: bool) -> Query {
        if is_typescript {
            Query::new(
                language,
                r#"
                ; Class extends
                (class_declaration
                    name: (type_identifier) @class.name
                    (class_heritage
                        (extends_clause
                            value: (identifier) @extends.name
                        )
                    )?
                    (class_heritage
                        (implements_clause
                            (type_identifier) @implements.name
                        )
                    )?
                ) @class.def

                ; Interface extends
                (interface_declaration
                    name: (type_identifier) @interface.name
                    (extends_type_clause
                        (type_identifier) @interface_extends.name
                    )?
                ) @interface.def
                "#,
            )
            .expect("Invalid TypeScript inheritance query")
        } else {
            // JavaScript uses a simpler class_heritage structure
            Query::new(
                language,
                r#"
                ; Class extends (JavaScript) - heritage is directly the identifier
                (class_declaration
                    name: (identifier) @class.name
                ) @class.def
                "#,
            )
            .expect("Invalid JavaScript inheritance query")
        }
    }

    /// Parse a TypeScript/JavaScript source file.
    pub fn parse(
        &mut self,
        file_path: &str,
        source: &str,
        variant: JsVariant,
    ) -> Option<ParseResult> {
        let (parser, language) = self.get_parser_and_language(variant);
        let tree = parser.parse(source, None)?;
        let root = tree.root_node();

        let is_typescript = matches!(variant, JsVariant::TypeScript | JsVariant::Tsx);
        let symbols_query = Self::create_symbols_query(language, is_typescript);
        let calls_query = Self::create_calls_query(language);
        let inheritance_query = Self::create_inheritance_query(language, is_typescript);

        let mut symbols = Vec::new();
        let mut edges = Vec::new();
        let mut imports = Vec::new();
        let mut exports = Vec::new();

        // Extract symbols
        Self::extract_symbols(
            &symbols_query,
            &root,
            file_path,
            source,
            &mut symbols,
            &mut imports,
            &mut exports,
        );

        // Extract edges (calls)
        extract_call_edges(
            &calls_query,
            &root,
            source,
            &symbols,
            &mut edges,
            &CallCapturePatterns::TYPESCRIPT,
        );

        // Extract inheritance edges (extends/implements)
        Self::extract_inheritance_edges(
            &inheritance_query,
            &root,
            file_path,
            source,
            &symbols,
            &mut edges,
        );

        let module = ModuleInfo {
            file_path: file_path.to_string(),
            module_name: super::extract_module_name(file_path, &["index"]),
            exports,
            imports,
        };

        Some(ParseResult {
            file_path: file_path.to_string(),
            language: variant.as_str().to_string(),
            symbols,
            edges,
            module: Some(module),
        })
    }

    /// Extract symbols from the AST.
    fn extract_symbols(
        query: &Query,
        root: &Node,
        file_path: &str,
        source: &str,
        symbols: &mut Vec<Symbol>,
        imports: &mut Vec<ImportInfo>,
        exports: &mut Vec<String>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, *root, source.as_bytes());

        let mut current_class: Option<String> = None;

        while let Some(m) = matches.next() {
            let mut name: Option<&str> = None;
            let mut kind: Option<SymbolKind> = None;
            let mut def_node: Option<Node> = None;
            let mut is_export = false;
            let mut import_source: Option<&str> = None;

            for capture in m.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let capture_str = capture_name;
                let node = capture.node;
                let text = node.utf8_text(source.as_bytes()).unwrap_or("");

                // Try to match standard symbol patterns
                if let Some(k) = find_symbol_kind(capture_str, TS_SYMBOL_MAPPINGS) {
                    name = Some(text);
                    kind = Some(k);
                    // Track current class for method parent resolution
                    if k == SymbolKind::Class {
                        current_class = Some(text.to_string());
                    }
                } else {
                    // Handle special cases and .def captures
                    match capture_str {
                        // Exports
                        "export.def" => {
                            is_export = true;
                            def_node = Some(node);
                        }
                        // Imports (must be before generic .def handling)
                        "import.source" => {
                            import_source = Some(text.trim_matches('"').trim_matches('\''));
                        }
                        "import.def" => {
                            push_ts_import(&node, import_source, source, imports);
                            import_source = None;
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
                let symbol = build_ts_symbol(
                    file_path,
                    source,
                    name,
                    kind,
                    &node,
                    is_export,
                    current_class.as_deref(),
                );

                // Track exports
                if symbol.visibility == Visibility::Public {
                    exports.push(name.to_string());
                }

                symbols.push(symbol);
            }
        }
    }

    /// Extract inheritance edges (Extends/Implements) from the AST.
    fn extract_inheritance_edges(
        query: &Query,
        root: &Node,
        _file_path: &str,
        source: &str,
        symbols: &[Symbol],
        edges: &mut Vec<Edge>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, *root, source.as_bytes());

        while let Some(m) = matches.next() {
            let mut class_name: Option<&str> = None;
            let mut interface_name: Option<&str> = None;
            let mut extends_names: Vec<&str> = Vec::new();
            let mut implements_names: Vec<&str> = Vec::new();
            let mut interface_extends_names: Vec<&str> = Vec::new();
            let mut def_node: Option<Node> = None;

            for capture in m.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let node = capture.node;
                let text = node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "class.name" => {
                        class_name = Some(text);
                    }
                    "class.def" => {
                        def_node = Some(node);
                    }
                    "extends.name" => {
                        extends_names.push(text);
                    }
                    "implements.name" => {
                        implements_names.push(text);
                    }
                    "interface.name" => {
                        interface_name = Some(text);
                    }
                    "interface.def" => {
                        def_node = Some(node);
                    }
                    "interface_extends.name" => {
                        interface_extends_names.push(text);
                    }
                    _ => {}
                }
            }

            // Handle class extends and implements
            if let (Some(name), Some(node)) = (class_name, def_node) {
                let line = node.start_position().row as u32 + 1;
                let col = node.start_position().column as u32;

                if let Some(source_id) = symbols
                    .iter()
                    .find(|s| s.name == name && s.kind == SymbolKind::Class)
                    .map(|s| &s.id)
                {
                    for extends_name in extends_names {
                        edges.push(create_inheritance_edge(
                            source_id,
                            extends_name,
                            SymbolKind::Class,
                            EdgeKind::Extends,
                            format!("class {} extends {}", name, extends_name),
                            line,
                            col,
                            symbols,
                        ));
                    }
                    for implements_name in implements_names {
                        edges.push(create_inheritance_edge(
                            source_id,
                            implements_name,
                            SymbolKind::Interface,
                            EdgeKind::Implements,
                            format!("class {} implements {}", name, implements_name),
                            line,
                            col,
                            symbols,
                        ));
                    }
                }
            }

            // Handle interface extends
            if let (Some(name), Some(node)) = (interface_name, def_node) {
                let line = node.start_position().row as u32 + 1;
                let col = node.start_position().column as u32;

                if let Some(source_id) = symbols
                    .iter()
                    .find(|s| s.name == name && s.kind == SymbolKind::Interface)
                    .map(|s| &s.id)
                {
                    for extends_name in interface_extends_names {
                        edges.push(create_inheritance_edge(
                            source_id,
                            extends_name,
                            SymbolKind::Interface,
                            EdgeKind::Extends,
                            format!("interface {} extends {}", name, extends_name),
                            line,
                            col,
                            symbols,
                        ));
                    }
                }
            }
        }
    }
}

/// Record an import from an `import.def` match into the module's import list.
fn push_ts_import(
    node: &Node,
    import_source: Option<&str>,
    source: &str,
    imports: &mut Vec<ImportInfo>,
) {
    let src = import_source
        .map(String::from)
        .or_else(|| extract_import_source(node, source));
    if let Some(src) = src {
        imports.push(ImportInfo {
            from: src,
            names: extract_import_names(node, source),
            alias: None,
        });
    }
}

/// Build a [`Symbol`] from a resolved definition capture.
fn build_ts_symbol(
    file_path: &str,
    source: &str,
    name: &str,
    kind: SymbolKind,
    node: &Node,
    is_export: bool,
    current_class: Option<&str>,
) -> Symbol {
    let visibility = if is_export || is_exported(node, source) {
        Visibility::Public
    } else {
        Visibility::Private
    };

    let docstring = extract_jsdoc(node, source);
    let brief = docstring.as_ref().and_then(|d| extract_brief(d));
    let signature = build_signature(kind, name, source, node);

    // Determine parent for methods
    let parent_name = if kind == SymbolKind::Method {
        current_class
    } else {
        None
    };

    let parent_id = parent_name.map(|p| Symbol::make_id(file_path, p, None));
    let symbol_source = node.utf8_text(source.as_bytes()).ok().map(String::from);
    let id = Symbol::make_id(file_path, name, parent_name);

    Symbol {
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
    }
}

/// Create an inheritance edge (extends or implements).
#[allow(clippy::too_many_arguments)]
fn create_inheritance_edge(
    source_id: &str,
    target_name: &str,
    target_kind: SymbolKind,
    edge_kind: EdgeKind,
    context: String,
    line: u32,
    col: u32,
    symbols: &[Symbol],
) -> Edge {
    let target_id = symbols
        .iter()
        .find(|s| s.name == target_name && s.kind == target_kind)
        .map(|s| s.id.clone());

    Edge {
        source_id: source_id.to_string(),
        target_id,
        target_name: target_name.to_string(),
        kind: edge_kind,
        line: Some(line),
        col: Some(col),
        context: Some(context),
    }
}

/// Check if a node is exported.
fn is_exported(node: &Node, source: &str) -> bool {
    // Check if parent is an export statement
    if let Some(parent) = node.parent() {
        let parent_text = parent.utf8_text(source.as_bytes()).unwrap_or("");
        if parent_text.starts_with("export ") {
            return true;
        }
    }

    // Check for 'export' keyword in the node text itself
    let text = node.utf8_text(source.as_bytes()).unwrap_or("");
    text.starts_with("export ")
}

/// Extract import source (module path) from an import statement.
fn extract_import_source(node: &Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            let text = child.utf8_text(source.as_bytes()).ok()?;
            return Some(text.trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

/// Extract import names from an import statement.
fn extract_import_names(node: &Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let text = node.utf8_text(source.as_bytes()).unwrap_or("");

    // Parse import { a, b, c } from 'module'
    if let Some(start) = text.find('{') {
        if let Some(end) = text.find('}') {
            let names_str = &text[start + 1..end];
            for name in names_str.split(',') {
                let name = name.trim();
                // Handle 'as' aliases
                let actual_name = if let Some(idx) = name.find(" as ") {
                    name[..idx].trim()
                } else {
                    name
                };
                if !actual_name.is_empty() {
                    names.push(actual_name.to_string());
                }
            }
        }
    }

    // Parse import defaultExport from 'module'
    if names.is_empty() && !text.contains('{') && !text.contains('*') {
        if let Some(import_idx) = text.find("import ") {
            let rest = &text[import_idx + 7..];
            if let Some(from_idx) = rest.find(" from ") {
                let name = rest[..from_idx].trim();
                if !name.is_empty() && !name.starts_with('{') {
                    names.push(name.to_string());
                }
            }
        }
    }

    // Parse import * as name from 'module'
    if names.is_empty() && text.contains("* as ") {
        if let Some(start) = text.find("* as ") {
            let rest = &text[start + 5..];
            if let Some(end) = rest.find(" from ") {
                let name = rest[..end].trim();
                names.push(format!("* as {}", name));
            }
        }
    }

    names
}

/// Extract JSDoc comment from a node.
fn extract_jsdoc(node: &Node, source: &str) -> Option<String> {
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        let kind = sibling.kind();

        if kind == "comment" {
            let text = sibling.utf8_text(source.as_bytes()).unwrap_or("");
            if text.starts_with("/**") {
                // Multi-line JSDoc
                return Some(super::parse_block_doc_comment(text));
            } else if text.starts_with("//") {
                // Single-line comment
                let content = text.trim_start_matches("//").trim();
                return Some(content.to_string());
            }
        } else if kind != "comment" {
            break;
        }

        prev = sibling.prev_sibling();
    }

    None
}

/// Build a signature string for a symbol.
fn build_signature(kind: SymbolKind, name: &str, source: &str, node: &Node) -> Option<String> {
    match kind {
        SymbolKind::Function | SymbolKind::Method => {
            let text = node.utf8_text(source.as_bytes()).ok()?;
            // Find where the body starts (first '{')
            if let Some(idx) = text.find('{') {
                let sig = text[..idx].trim();
                // Clean up multi-line signatures
                let sig = sig.lines().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
                Some(sig)
            } else if let Some(idx) = text.find("=>") {
                // Arrow function
                let sig = text[..idx + 2].trim();
                Some(sig.to_string())
            } else {
                Some(text.lines().next()?.trim().to_string())
            }
        }
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum => {
            let text = node.utf8_text(source.as_bytes()).ok()?;
            let first_line = text.lines().next()?.trim();
            Some(first_line.trim_end_matches('{').trim().to_string())
        }
        SymbolKind::Type => {
            let text = node.utf8_text(source.as_bytes()).ok()?;
            Some(text.lines().next()?.trim().to_string())
        }
        _ => Some(name.to_string()),
    }
}

impl Default for TypeScriptParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_function() {
        let mut parser = TypeScriptParser::new();
        let source = r#"
/**
 * Greets a user by name.
 */
export function greet(name: string): string {
    return `Hello, ${name}!`;
}
"#;

        let result = parser
            .parse("test.ts", source, JsVariant::TypeScript)
            .unwrap();
        assert!(!result.symbols.is_empty());

        let func = result.symbols.iter().find(|s| s.name == "greet");
        assert!(func.is_some());
        let func = func.unwrap();
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.visibility, Visibility::Public);
    }

    #[test]
    fn test_parse_class() {
        let mut parser = TypeScriptParser::new();
        let source = r#"
export class Counter {
    private count: number = 0;

    increment(): void {
        this.count++;
    }

    getCount(): number {
        return this.count;
    }
}
"#;

        let result = parser
            .parse("test.ts", source, JsVariant::TypeScript)
            .unwrap();

        let class = result.symbols.iter().find(|s| s.name == "Counter");
        assert!(class.is_some());
        assert_eq!(class.unwrap().kind, SymbolKind::Class);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn test_parse_interface() {
        let mut parser = TypeScriptParser::new();
        let source = r#"
interface User {
    id: number;
    name: string;
    email?: string;
}
"#;

        let result = parser
            .parse("test.ts", source, JsVariant::TypeScript)
            .unwrap();

        let interface = result.symbols.iter().find(|s| s.name == "User");
        assert!(interface.is_some());
        assert_eq!(interface.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn test_parse_arrow_function() {
        let mut parser = TypeScriptParser::new();
        let source = r#"
const add = (a: number, b: number): number => a + b;

const multiply = (a: number, b: number): number => {
    return a * b;
};
"#;

        let result = parser
            .parse("test.ts", source, JsVariant::TypeScript)
            .unwrap();

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 2);
    }

    #[test]
    fn test_parse_jsx() {
        let mut parser = TypeScriptParser::new();
        let source = r#"
function Button({ onClick, children }) {
    return <button onClick={onClick}>{children}</button>;
}
"#;

        let result = parser.parse("test.jsx", source, JsVariant::Jsx).unwrap();

        let func = result.symbols.iter().find(|s| s.name == "Button");
        assert!(func.is_some());
        assert_eq!(func.unwrap().kind, SymbolKind::Function);
    }

    #[test]
    fn test_parse_tsx() {
        let mut parser = TypeScriptParser::new();
        let source = r#"
interface Props {
    name: string;
}

function Greeting({ name }: Props): JSX.Element {
    return <h1>Hello, {name}!</h1>;
}
"#;

        let result = parser.parse("test.tsx", source, JsVariant::Tsx).unwrap();

        let interface = result.symbols.iter().find(|s| s.name == "Props");
        assert!(interface.is_some());

        let func = result.symbols.iter().find(|s| s.name == "Greeting");
        assert!(func.is_some());
    }

    #[test]
    fn test_parse_javascript() {
        let mut parser = TypeScriptParser::new();
        let source = r#"
/**
 * Calculate the sum of an array.
 */
function sum(numbers) {
    return numbers.reduce((a, b) => a + b, 0);
}

class Calculator {
    add(a, b) {
        return a + b;
    }
}
"#;

        let result = parser
            .parse("test.js", source, JsVariant::JavaScript)
            .unwrap();

        let func = result.symbols.iter().find(|s| s.name == "sum");
        assert!(func.is_some());

        let class = result.symbols.iter().find(|s| s.name == "Calculator");
        assert!(class.is_some());
    }

    #[test]
    fn test_extract_extends_edges() {
        let mut parser = TypeScriptParser::new();
        let source = r#"
class Animal {
    name: string;
}

class Dog extends Animal {
    bark(): void {
        console.log("Woof!");
    }
}

class Cat extends Animal {
    meow(): void {
        console.log("Meow!");
    }
}
"#;

        let result = parser
            .parse("test.ts", source, JsVariant::TypeScript)
            .unwrap();

        let extends_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Extends)
            .collect();

        // Dog extends Animal, Cat extends Animal
        assert_eq!(extends_edges.len(), 2);

        assert!(extends_edges
            .iter()
            .any(|e| { e.source_id.contains("Dog") && e.target_name == "Animal" }));

        assert!(extends_edges
            .iter()
            .any(|e| { e.source_id.contains("Cat") && e.target_name == "Animal" }));
    }

    #[test]
    fn test_extract_implements_edges() {
        let mut parser = TypeScriptParser::new();
        let source = r#"
interface Printable {
    print(): void;
}

interface Serializable {
    serialize(): string;
}

class Document implements Printable, Serializable {
    print(): void {
        console.log("Printing...");
    }
    serialize(): string {
        return "{}";
    }
}
"#;

        let result = parser
            .parse("test.ts", source, JsVariant::TypeScript)
            .unwrap();

        let implements_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Implements)
            .collect();

        // Document implements Printable and Serializable
        assert!(
            !implements_edges.is_empty(),
            "Expected at least 1 implements edge"
        );

        assert!(implements_edges.iter().any(|e| {
            e.source_id.contains("Document")
                && (e.target_name == "Printable" || e.target_name == "Serializable")
        }));
    }

    #[test]
    fn test_extract_interface_extends_edges() {
        let mut parser = TypeScriptParser::new();
        let source = r#"
interface Base {
    id: number;
}

interface Extended extends Base {
    name: string;
}
"#;

        let result = parser
            .parse("test.ts", source, JsVariant::TypeScript)
            .unwrap();

        let extends_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Extends)
            .collect();

        // Extended extends Base
        assert!(extends_edges
            .iter()
            .any(|e| { e.source_id.contains("Extended") && e.target_name == "Base" }));
    }

    #[test]
    fn test_imports_stored_in_module() {
        // NOTE: Import edges are now stored in module.imports rather than as edges
        // because edges require a source_id that references an existing symbol (FK constraint)
        let mut parser = TypeScriptParser::new();
        let source = r#"
import { useState, useEffect } from 'react';
import axios from 'axios';
"#;

        let result = parser
            .parse("test.ts", source, JsVariant::TypeScript)
            .unwrap();

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
            .any(|n| n.contains("useState") || n.contains("useEffect") || n.contains("axios")));
    }
}
