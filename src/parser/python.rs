//! Python code parsing using tree-sitter.

use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::db::{
    Edge, EdgeKind, ImportInfo, ModuleInfo, ParseResult, Symbol, SymbolKind, Visibility,
};
use crate::parser::{
    extract_brief, extract_call_edges, find_symbol_kind, is_def_capture, CallCapturePatterns,
    SymbolKindMapping,
};

/// Symbol kind mappings for Python capture names.
const PYTHON_SYMBOL_MAPPINGS: &[SymbolKindMapping] = &[
    SymbolKindMapping::new("func", SymbolKind::Function),
    SymbolKindMapping::new("decorated_func", SymbolKind::Function),
    SymbolKindMapping::new("class", SymbolKind::Class),
    SymbolKindMapping::new("decorated_class", SymbolKind::Class),
    SymbolKindMapping::new("method", SymbolKind::Method),
    SymbolKindMapping::new("decorated_method", SymbolKind::Method),
];

/// Python parser.
pub struct PythonParser {
    parser: Parser,
    language: Language,
}

impl PythonParser {
    /// Create a new Python parser.
    pub fn new() -> Self {
        let language: Language = tree_sitter_python::LANGUAGE.into();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .expect("Failed to set Python language");

        Self { parser, language }
    }

    /// Create the symbols query.
    fn create_symbols_query(&self) -> Query {
        Query::new(
            &self.language,
            r#"
            ; Functions (top-level only - not inside decorated_definition)
            (module
                (function_definition
                    name: (identifier) @func.name
                ) @func.def
            )

            ; Classes (top-level only - not inside decorated_definition)
            (module
                (class_definition
                    name: (identifier) @class.name
                ) @class.def
            )

            ; Functions inside classes (methods)
            (class_definition
                body: (block
                    (function_definition
                        name: (identifier) @method.name
                    ) @method.def
                )
            )

            ; Decorated functions (top-level)
            (module
                (decorated_definition
                    definition: (function_definition
                        name: (identifier) @decorated_func.name
                    ) @decorated_func.inner
                ) @decorated_func.def
            )

            ; Decorated classes (top-level)
            (module
                (decorated_definition
                    definition: (class_definition
                        name: (identifier) @decorated_class.name
                    ) @decorated_class.inner
                ) @decorated_class.def
            )

            ; Class inheritance (for Extends edges)
            (class_definition
                name: (identifier) @class_inherit.name
                superclasses: (argument_list) @class_inherit.bases
            ) @class_inherit.def

            ; Decorated methods inside classes
            (class_definition
                body: (block
                    (decorated_definition
                        definition: (function_definition
                            name: (identifier) @decorated_method.name
                        ) @decorated_method.inner
                    ) @decorated_method.def
                )
            )

            ; Import statements
            (import_statement
                name: (dotted_name) @import.name
            ) @import.def

            (import_from_statement
                module_name: (dotted_name)? @import_from.module
            ) @import_from.def

            ; Assignments (for module-level constants)
            (expression_statement
                (assignment
                    left: (identifier) @assign.name
                )
            ) @assign.def
            "#,
        )
        .expect("Invalid Python symbols query")
    }

    /// Create the calls query.
    fn create_calls_query(&self) -> Query {
        Query::new(
            &self.language,
            r#"
            ; Function calls
            (call
                function: (identifier) @call.name
            ) @call.expr

            ; Method calls
            (call
                function: (attribute
                    attribute: (identifier) @method_call.name
                )
            ) @method_call.expr

            ; Constructor calls (same as function calls in Python)
            "#,
        )
        .expect("Invalid Python calls query")
    }

    /// Create the inheritance query.
    fn create_inheritance_query(&self) -> Query {
        Query::new(
            &self.language,
            r#"
            ; Class inheritance
            (class_definition
                name: (identifier) @class.name
                superclasses: (argument_list
                    (identifier) @base.name
                )
            ) @class.def

            ; Class inheritance with attribute access (e.g., module.ClassName)
            (class_definition
                name: (identifier) @class_attr.name
                superclasses: (argument_list
                    (attribute
                        attribute: (identifier) @base_attr.name
                    )
                )
            ) @class_attr.def
            "#,
        )
        .expect("Invalid Python inheritance query")
    }

    /// Parse a Python source file.
    pub fn parse(&mut self, file_path: &str, source: &str) -> Option<ParseResult> {
        let tree = self.parser.parse(source, None)?;
        let root = tree.root_node();

        let symbols_query = self.create_symbols_query();
        let calls_query = self.create_calls_query();
        let inheritance_query = self.create_inheritance_query();

        let mut symbols = Vec::new();
        let mut edges = Vec::new();
        let mut imports = Vec::new();
        let mut exports = Vec::new();

        // Extract symbols
        self.extract_symbols(
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
            &CallCapturePatterns::STANDARD,
        );

        // Extract inheritance edges (extends)
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
            module_name: super::extract_module_name(file_path, &["__init__", "__main__"]),
            exports,
            imports,
        };

        Some(ParseResult {
            file_path: file_path.to_string(),
            language: "python".to_string(),
            symbols,
            edges,
            module: Some(module),
        })
    }

    #[allow(clippy::too_many_arguments)]
    /// Extract symbols from the AST.
    fn extract_symbols(
        &self,
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

        // Track class context for methods
        let class_ranges: Vec<(u32, u32, String)> = self.find_class_ranges(root, source);
        let mut import_module: Option<String> = None;

        while let Some(m) = matches.next() {
            let mut name: Option<&str> = None;
            let mut kind: Option<SymbolKind> = None;
            let mut def_node: Option<Node> = None;
            let mut parent_class: Option<&str> = None;

            for capture in m.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let capture_str = capture_name;
                let node = capture.node;
                let text = node.utf8_text(source.as_bytes()).unwrap_or("");

                // Try to match standard symbol patterns (func, class, method, etc.)
                if let Some(k) = find_symbol_kind(capture_str, PYTHON_SYMBOL_MAPPINGS) {
                    name = Some(text);
                    kind = Some(k);
                } else if is_def_capture(capture_str) {
                    def_node = Some(node);
                    // For method definitions, find the parent class
                    parent_class = find_parent_class(capture_str, &node, &class_ranges);
                } else {
                    // Handle imports and other special cases
                    match handle_import_capture(
                        capture_str,
                        text,
                        &node,
                        source,
                        &mut import_module,
                    ) {
                        ImportCaptureResult::SimpleImport(info) => imports.push(info),
                        ImportCaptureResult::FromImport(info) => imports.push(info),
                        ImportCaptureResult::ModuleName | ImportCaptureResult::NotImport => {
                            // Check for module-level constants
                            if capture_str == "assign.name" && is_python_constant(text) {
                                name = Some(text);
                                kind = Some(SymbolKind::Const);
                            }
                        }
                    }
                }
            }

            // Create symbol if we have enough information
            if let (Some(name), Some(kind), Some(node)) = (name, kind, def_node) {
                let symbol =
                    create_python_symbol(file_path, name, kind, &node, source, parent_class);
                if symbol.visibility == Visibility::Public {
                    exports.push(name.to_string());
                }
                symbols.push(symbol);
            }
        }
    }

    /// Find all class ranges in the file.
    fn find_class_ranges(&self, root: &Node, source: &str) -> Vec<(u32, u32, String)> {
        let mut ranges = Vec::new();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "class_definition" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(source.as_bytes()).unwrap_or("");
                    ranges.push((
                        child.start_position().row as u32 + 1,
                        child.end_position().row as u32 + 1,
                        name.to_string(),
                    ));
                }
            } else if child.kind() == "decorated_definition" {
                // Check for decorated classes
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "class_definition" {
                        if let Some(name_node) = inner_child.child_by_field_name("name") {
                            let name = name_node.utf8_text(source.as_bytes()).unwrap_or("");
                            ranges.push((
                                child.start_position().row as u32 + 1,
                                child.end_position().row as u32 + 1,
                                name.to_string(),
                            ));
                        }
                    }
                }
            }
        }

        ranges
    }

    /// Extract inheritance edges (Extends) from the AST.
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
            let mut base_names: Vec<&str> = Vec::new();
            let mut class_node: Option<Node> = None;

            for capture in m.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let node = capture.node;
                let text = node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "class.name" | "class_attr.name" => {
                        class_name = Some(text);
                    }
                    "class.def" | "class_attr.def" => {
                        class_node = Some(node);
                    }
                    "base.name" | "base_attr.name" => {
                        base_names.push(text);
                    }
                    _ => {}
                }
            }

            if let (Some(class_name), Some(node)) = (class_name, class_node) {
                let line = node.start_position().row as u32 + 1;
                let col = node.start_position().column as u32;

                // Find the class symbol
                let source_id = symbols
                    .iter()
                    .find(|s| s.name == class_name && s.kind == SymbolKind::Class)
                    .map(|s| s.id.clone());

                if let Some(source_id) = source_id {
                    for base_name in base_names {
                        // Skip object (implicit base in Python 3)
                        if base_name == "object" {
                            continue;
                        }

                        // Try to resolve the target
                        let target_id = symbols
                            .iter()
                            .find(|s| s.name == base_name && s.kind == SymbolKind::Class)
                            .map(|s| s.id.clone());

                        edges.push(Edge {
                            source_id: source_id.clone(),
                            target_id,
                            target_name: base_name.to_string(),
                            kind: EdgeKind::Extends,
                            line: Some(line),
                            col: Some(col),
                            context: Some(format!("class {}({}):", class_name, base_name)),
                        });
                    }
                }
            }
        }
    }
}

/// Extract visibility from a Python symbol name.
/// In Python, names starting with _ are private, __ are more private.
fn extract_visibility(name: &str) -> Visibility {
    if name.starts_with("__") && !name.ends_with("__") {
        // Name mangling (strongly private)
        Visibility::Private
    } else if name.starts_with('_') {
        // Convention for private
        Visibility::Private
    } else {
        Visibility::Public
    }
}

/// Extract docstring from a function/class definition.
fn extract_docstring(node: &Node, source: &str) -> Option<String> {
    // In Python, docstrings are the first statement in a function/class body
    // Look for body -> block -> expression_statement -> string

    let body = node.child_by_field_name("body")?;

    // Get first child of block
    let mut cursor = body.walk();
    let first_child = body.children(&mut cursor).next()?;
    if first_child.kind() == "expression_statement" {
        let mut inner_cursor = first_child.walk();
        for inner_child in first_child.children(&mut inner_cursor) {
            if inner_child.kind() == "string" {
                let text = inner_child.utf8_text(source.as_bytes()).ok()?;
                return Some(clean_docstring(text));
            }
        }
    }
    None
}

/// Clean up a Python docstring (remove quotes, normalize whitespace).
fn clean_docstring(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_start_matches("\"\"\"")
        .trim_start_matches("'''")
        .trim_end_matches("\"\"\"")
        .trim_end_matches("'''")
        .trim_start_matches('"')
        .trim_start_matches('\'')
        .trim_end_matches('"')
        .trim_end_matches('\'');

    // Normalize indentation
    let lines: Vec<&str> = trimmed.lines().collect();
    if lines.len() <= 1 {
        return trimmed.trim().to_string();
    }

    // Find minimum indentation (excluding first line)
    let min_indent = lines
        .iter()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

    let mut result = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            result.push(line.trim());
        } else if line.len() >= min_indent {
            result.push(&line[min_indent..]);
        } else {
            result.push(line.trim());
        }
    }

    result.join("\n").trim().to_string()
}

/// Build a signature string for a Python symbol.
fn build_signature(kind: SymbolKind, name: &str, source: &str, node: &Node) -> Option<String> {
    match kind {
        SymbolKind::Function | SymbolKind::Method => {
            let text = node.utf8_text(source.as_bytes()).ok()?;

            // Find the parameters
            let first_line = text.lines().next()?;
            let sig = first_line.trim();

            // Remove trailing colon
            let sig = sig.trim_end_matches(':').trim();

            // Check for async
            let is_async = sig.starts_with("async ");

            // Build clean signature
            if is_async {
                Some(format!("async def {}", &sig[10..]))
            } else if sig.starts_with("def ") {
                Some(sig.to_string())
            } else {
                Some(format!("def {}", sig))
            }
        }
        SymbolKind::Class => {
            let text = node.utf8_text(source.as_bytes()).ok()?;
            let first_line = text.lines().next()?.trim();
            Some(first_line.trim_end_matches(':').trim().to_string())
        }
        _ => Some(name.to_string()),
    }
}

/// Result of handling an import capture.
enum ImportCaptureResult {
    /// A simple import (import x)
    SimpleImport(ImportInfo),
    /// Module name for a from-import (from x import ...) - stored in import_module
    ModuleName,
    /// Completed from-import with module and names
    FromImport(ImportInfo),
    /// Not an import capture
    NotImport,
}

/// Handle import-related captures in Python.
fn handle_import_capture(
    capture_str: &str,
    text: &str,
    node: &Node,
    source: &str,
    import_module: &mut Option<String>,
) -> ImportCaptureResult {
    match capture_str {
        "import.name" => ImportCaptureResult::SimpleImport(ImportInfo {
            from: text.to_string(),
            names: vec![text.to_string()],
            alias: None,
        }),
        "import_from.module" => {
            *import_module = Some(text.to_string());
            ImportCaptureResult::ModuleName
        }
        "import_from.def" => {
            let module_name = import_module
                .take()
                .or_else(|| extract_import_from_module(node, source));
            if let Some(module) = module_name {
                let names = extract_import_names(node, source);
                ImportCaptureResult::FromImport(ImportInfo {
                    from: module,
                    names,
                    alias: None,
                })
            } else {
                ImportCaptureResult::NotImport
            }
        }
        _ => ImportCaptureResult::NotImport,
    }
}

/// Create a Python symbol from extracted information.
fn create_python_symbol(
    file_path: &str,
    name: &str,
    kind: SymbolKind,
    node: &Node,
    source: &str,
    parent_class: Option<&str>,
) -> Symbol {
    let visibility = extract_visibility(name);
    let docstring = extract_docstring(node, source);
    let brief = docstring.as_ref().and_then(|d| extract_brief(d));
    let signature = build_signature(kind, name, source, node);

    let parent_name = if kind == SymbolKind::Method {
        parent_class
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

/// Extract module name from an import_from_statement.
fn extract_import_from_module(node: &Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "dotted_name" {
            // This is the module name (first dotted_name in import_from_statement)
            return child.utf8_text(source.as_bytes()).ok().map(String::from);
        }
        if child.kind() == "relative_import" {
            // Handle relative imports like "from . import foo"
            return child.utf8_text(source.as_bytes()).ok().map(String::from);
        }
    }
    None
}

/// Extract the original name from an aliased import node (e.g., "y as z" -> "y").
fn extract_aliased_name(node: &Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "dotted_name" || child.kind() == "identifier" {
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }
    None
}

/// Find the parent class for a method definition node.
fn find_parent_class<'a>(
    capture_str: &str,
    node: &Node,
    class_ranges: &'a [(u32, u32, String)],
) -> Option<&'a str> {
    let prefix = capture_str.trim_end_matches(".def");
    if prefix == "method" || prefix == "decorated_method" {
        let line = node.start_position().row as u32 + 1;
        class_ranges
            .iter()
            .find(|(start, end, _)| line >= *start && line <= *end)
            .map(|(_, _, name)| name.as_str())
    } else {
        None
    }
}

/// Check if a name is a Python constant (UPPER_CASE).
fn is_python_constant(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit())
}

/// Parse import names from text as a fallback.
fn parse_import_names_from_text(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    if text.contains(" import *") {
        names.push("*".to_string());
    } else if let Some(import_idx) = text.find(" import ") {
        for name in text[import_idx + 8..].split(',') {
            let name = name.trim();
            let actual_name = name.split(" as ").next().unwrap_or("").trim();
            if !actual_name.is_empty() {
                names.push(actual_name.to_string());
            }
        }
    }
    names
}

/// Extract import names from an import_from_statement.
fn extract_import_names(node: &Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    let mut seen_module = false;

    for child in node.children(&mut cursor) {
        let kind = child.kind();

        // Skip keywords
        if kind == "import" || kind == "from" || kind == "import_prefix" {
            continue;
        }

        // The first dotted_name is the module, skip it
        if kind == "dotted_name" && !seen_module {
            seen_module = true;
            continue;
        }

        // Handle imported names (subsequent dotted_name nodes)
        if kind == "dotted_name" {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                names.push(text.to_string());
            }
            continue;
        }

        // Handle aliased imports: from x import y as z
        if kind == "aliased_import" {
            if let Some(name) = extract_aliased_name(&child, source) {
                names.push(name);
            }
            continue;
        }

        // Handle wildcard imports: from x import *
        if kind == "wildcard_import" {
            names.push("*".to_string());
            continue;
        }

        // Look for identifiers in other node types (like import lists)
        let mut inner_cursor = child.walk();
        for inner in child.children(&mut inner_cursor) {
            if inner.kind() == "dotted_name" || inner.kind() == "identifier" {
                if let Ok(text) = inner.utf8_text(source.as_bytes()) {
                    names.push(text.to_string());
                }
            } else if inner.kind() == "aliased_import" {
                if let Some(name) = extract_aliased_name(&inner, source) {
                    names.push(name);
                }
            }
        }
    }

    // Fallback: parse from text if we got nothing
    if names.is_empty() {
        names = parse_import_names_from_text(node.utf8_text(source.as_bytes()).unwrap_or(""));
    }

    // Deduplicate and filter
    names.sort();
    names.dedup();
    names.retain(|n| !n.is_empty());
    names
}

/// Truncate context to a maximum length.
impl Default for PythonParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_function() {
        let mut parser = PythonParser::new();
        let source = r#"
def greet(name: str) -> str:
    """Greet a user by name."""
    return f"Hello, {name}!"
"#;

        let result = parser.parse("test.py", source).unwrap();
        assert!(!result.symbols.is_empty());

        let func = result.symbols.iter().find(|s| s.name == "greet");
        assert!(func.is_some());
        let func = func.unwrap();
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.visibility, Visibility::Public);
        assert!(func.docstring.is_some());
        assert_eq!(func.brief.as_ref().unwrap(), "Greet a user by name.");
    }

    #[test]
    fn test_parse_class() {
        let mut parser = PythonParser::new();
        let source = r#"
class Counter:
    """A simple counter class."""

    def __init__(self, start: int = 0):
        """Initialize the counter."""
        self.count = start

    def increment(self):
        """Increment the counter."""
        self.count += 1

    def get_count(self) -> int:
        """Get the current count."""
        return self.count
"#;

        let result = parser.parse("test.py", source).unwrap();

        let class = result.symbols.iter().find(|s| s.name == "Counter");
        assert!(class.is_some());
        assert_eq!(class.unwrap().kind, SymbolKind::Class);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 3);

        // Check that __init__ is private
        let init = methods.iter().find(|m| m.name == "__init__").unwrap();
        assert_eq!(init.visibility, Visibility::Private);
    }

    #[test]
    fn test_parse_private_function() {
        let mut parser = PythonParser::new();
        let source = r#"
def public_func():
    pass

def _private_func():
    pass

def __very_private():
    pass
"#;

        let result = parser.parse("test.py", source).unwrap();

        let public = result
            .symbols
            .iter()
            .find(|s| s.name == "public_func")
            .unwrap();
        assert_eq!(public.visibility, Visibility::Public);

        let private = result
            .symbols
            .iter()
            .find(|s| s.name == "_private_func")
            .unwrap();
        assert_eq!(private.visibility, Visibility::Private);

        let very_private = result
            .symbols
            .iter()
            .find(|s| s.name == "__very_private")
            .unwrap();
        assert_eq!(very_private.visibility, Visibility::Private);
    }

    #[test]
    fn test_parse_async_function() {
        let mut parser = PythonParser::new();
        let source = r#"
async def fetch_data(url: str) -> dict:
    """Fetch data from a URL."""
    async with aiohttp.ClientSession() as session:
        async with session.get(url) as response:
            return await response.json()
"#;

        let result = parser.parse("test.py", source).unwrap();

        let func = result.symbols.iter().find(|s| s.name == "fetch_data");
        assert!(func.is_some());
        let func = func.unwrap();
        assert_eq!(func.kind, SymbolKind::Function);
        assert!(func.signature.as_ref().unwrap().contains("async def"));
    }

    #[test]
    fn test_parse_decorated_function() {
        let mut parser = PythonParser::new();
        let source = r#"
@app.route("/")
def index():
    """Handle the index route."""
    return "Hello, World!"

@staticmethod
def static_method():
    pass
"#;

        let result = parser.parse("test.py", source).unwrap();

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 2);
    }

    #[test]
    fn test_extract_calls() {
        let mut parser = PythonParser::new();
        let source = r#"
def foo():
    bar()
    baz()

def bar():
    pass

def baz():
    pass
"#;

        let result = parser.parse("test.py", source).unwrap();

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
    fn test_parse_constant() {
        let mut parser = PythonParser::new();
        let source = "MAX_SIZE = 100\nAPI_KEY = \"secret\"\nregular_var = 42\n";

        let result = parser.parse("test.py", source).unwrap();

        // Should only capture UPPER_CASE names as constants
        let consts: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Const)
            .collect();
        assert_eq!(consts.len(), 2);
        assert!(consts.iter().any(|c| c.name == "MAX_SIZE"));
        assert!(consts.iter().any(|c| c.name == "API_KEY"));
    }

    #[test]
    fn test_clean_docstring() {
        assert_eq!(
            clean_docstring("\"\"\"Simple docstring.\"\"\""),
            "Simple docstring."
        );
        assert_eq!(
            clean_docstring("'''Multi-line\n    docstring.'''"),
            "Multi-line\ndocstring."
        );
    }

    #[test]
    fn test_extract_inheritance_edges() {
        let mut parser = PythonParser::new();
        let source = r#"
class Animal:
    pass

class Dog(Animal):
    pass

class Cat(Animal):
    pass

class Hybrid(Dog, Cat):
    """A hybrid animal."""
    pass
"#;

        let result = parser.parse("test.py", source).unwrap();

        let extends_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Extends)
            .collect();

        // Dog extends Animal, Cat extends Animal, Hybrid extends Dog and Cat
        assert_eq!(extends_edges.len(), 4);

        // Check Dog -> Animal
        assert!(extends_edges
            .iter()
            .any(|e| { e.source_id.contains("Dog") && e.target_name == "Animal" }));

        // Check Cat -> Animal
        assert!(extends_edges
            .iter()
            .any(|e| { e.source_id.contains("Cat") && e.target_name == "Animal" }));

        // Check Hybrid -> Dog
        assert!(extends_edges
            .iter()
            .any(|e| { e.source_id.contains("Hybrid") && e.target_name == "Dog" }));

        // Check Hybrid -> Cat
        assert!(extends_edges
            .iter()
            .any(|e| { e.source_id.contains("Hybrid") && e.target_name == "Cat" }));
    }

    #[test]
    fn test_imports_stored_in_module() {
        // NOTE: Import edges are now stored in module.imports rather than as edges
        // because edges require a source_id that references an existing symbol (FK constraint)
        let mut parser = PythonParser::new();
        let source = r#"
import os
from typing import List, Dict
from collections import defaultdict
"#;

        let result = parser.parse("test.py", source).unwrap();

        // Imports should be in module info, not as edges
        let module = result.module.unwrap();
        assert!(
            !module.imports.is_empty(),
            "Expected imports in module info"
        );

        // Check we captured the imports - look for typing imports
        let all_imports: Vec<_> = module.imports.iter().flat_map(|i| i.names.iter()).collect();
        assert!(all_imports
            .iter()
            .any(|n| n.contains("os") || n.contains("List") || n.contains("Dict")));
    }
}
