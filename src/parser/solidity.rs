//! Solidity-specific code parsing using solang-parser.
//!
//! This module uses solang-parser (the parser from the Hyperledger Solang compiler)
//! instead of tree-sitter due to tree-sitter version incompatibilities in the ecosystem.

use solang_parser::pt::{
    self, ContractPart, ContractTy, Expression, FunctionAttribute, FunctionTy, Loc, SourceUnitPart,
    VariableAttribute, Visibility as SolVisibility,
};

use crate::db::{
    Edge, EdgeKind, ImportInfo, ModuleInfo, ParseResult, Symbol, SymbolKind, Visibility,
};
use crate::parser::extract_brief;

/// Solidity-specific parser using solang-parser.
pub struct SolidityParser;

impl SolidityParser {
    /// Create a new Solidity parser.
    pub fn new() -> Self {
        Self
    }

    /// Parse a Solidity source file.
    pub fn parse(&mut self, file_path: &str, source: &str) -> Option<ParseResult> {
        let (tree, comments) = solang_parser::parse(source, 0).ok()?;

        let mut symbols = Vec::new();
        let mut edges = Vec::new();
        let mut imports = Vec::new();
        let mut exports = Vec::new();

        // Build a map of doc comments by their end location
        let doc_comments = extract_doc_comments(&comments, source);

        // Process each top-level item
        for part in &tree.0 {
            match part {
                SourceUnitPart::ContractDefinition(def) => {
                    let contract_name = def.name.as_ref().map(|id| id.name.clone());
                    let contract_kind = match def.ty {
                        ContractTy::Contract(_) => SymbolKind::Class,
                        ContractTy::Interface(_) => SymbolKind::Interface,
                        ContractTy::Library(_) => SymbolKind::Module,
                        ContractTy::Abstract(_) => SymbolKind::Class,
                    };

                    // Build signature
                    let signature = def.name.as_ref().map(|name| {
                        let ty_str = match def.ty {
                            ContractTy::Contract(_) => "contract",
                            ContractTy::Interface(_) => "interface",
                            ContractTy::Library(_) => "library",
                            ContractTy::Abstract(_) => "abstract contract",
                        };
                        format!("{} {}", ty_str, name.name)
                    });

                    if let Some(ref name) = contract_name {
                        let id = Symbol::make_id(file_path, name, None);
                        exports.push(name.clone());

                        push_symbol(
                            &mut symbols,
                            file_path,
                            source,
                            name,
                            None,
                            contract_kind,
                            Visibility::Public,
                            signature,
                            &def.loc,
                            &doc_comments,
                        );

                        // Process contract parts
                        extract_contract_parts(
                            &def.parts,
                            file_path,
                            source,
                            name,
                            &id,
                            &doc_comments,
                            &mut symbols,
                            &mut edges,
                        );
                    }
                }

                SourceUnitPart::ImportDirective(import) => {
                    if let Some(import_info) = extract_import(import) {
                        imports.push(import_info);
                    }
                }

                SourceUnitPart::FunctionDefinition(func) => {
                    // Free function (not in a contract)
                    if let Some(symbol) =
                        extract_function(func, file_path, source, None, None, &doc_comments)
                    {
                        symbols.push(symbol);
                    }
                }

                SourceUnitPart::StructDefinition(def) => {
                    if let Some(ref name) = def.name {
                        push_symbol(
                            &mut symbols,
                            file_path,
                            source,
                            &name.name,
                            None,
                            SymbolKind::Struct,
                            Visibility::Public,
                            Some(format!("struct {}", name.name)),
                            &def.loc,
                            &doc_comments,
                        );
                    }
                }

                SourceUnitPart::EnumDefinition(def) => {
                    if let Some(ref name) = def.name {
                        push_symbol(
                            &mut symbols,
                            file_path,
                            source,
                            &name.name,
                            None,
                            SymbolKind::Enum,
                            Visibility::Public,
                            Some(format!("enum {}", name.name)),
                            &def.loc,
                            &doc_comments,
                        );
                    }
                }

                SourceUnitPart::ErrorDefinition(def) => {
                    if let Some(ref name) = def.name {
                        push_symbol(
                            &mut symbols,
                            file_path,
                            source,
                            &name.name,
                            None,
                            SymbolKind::Type,
                            Visibility::Public,
                            Some(format!("error {}", name.name)),
                            &def.loc,
                            &doc_comments,
                        );
                    }
                }

                SourceUnitPart::EventDefinition(def) => {
                    if let Some(ref name) = def.name {
                        push_symbol(
                            &mut symbols,
                            file_path,
                            source,
                            &name.name,
                            None,
                            SymbolKind::Function,
                            Visibility::Public,
                            Some(format!("event {}", name.name)),
                            &def.loc,
                            &doc_comments,
                        );
                    }
                }

                _ => {}
            }
        }

        // Extract function call edges from function bodies
        extract_call_edges(file_path, source, &symbols, &mut edges);

        let module = ModuleInfo {
            file_path: file_path.to_string(),
            module_name: extract_contract_name(source),
            exports,
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
}

impl Default for SolidityParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a `Symbol` from the common location/doc-comment fields and push it.
///
/// Performs the shared dance (`loc_to_lines`, `find_doc_comment`, `extract_brief`,
/// `Symbol::make_id`, `extract_source`) once so the per-arm bodies stay thin.
/// `parent` carries `(contract_name, contract_id)` for contract members; pass
/// `None` for top-level symbols (yields no `qualified_name`/`parent_id`).
#[allow(clippy::too_many_arguments)]
fn push_symbol(
    symbols: &mut Vec<Symbol>,
    file_path: &str,
    source: &str,
    name: &str,
    parent: Option<(&str, &str)>,
    kind: SymbolKind,
    visibility: Visibility,
    signature: Option<String>,
    loc: &Loc,
    doc_comments: &[(u32, String)],
) {
    let (line_start, line_end, col_start, col_end) = loc_to_lines(loc, source);
    let docstring = find_doc_comment(doc_comments, line_start);
    let brief = docstring.as_ref().and_then(|d| extract_brief(d));

    let parent_name = parent.map(|(contract_name, _)| contract_name);
    let qualified_name = parent.map(|(contract_name, _)| format!("{}.{}", contract_name, name));
    let parent_id = parent.map(|(_, contract_id)| contract_id.to_string());

    symbols.push(Symbol {
        id: Symbol::make_id(file_path, name, parent_name),
        file_path: file_path.to_string(),
        name: name.to_string(),
        qualified_name,
        kind,
        visibility,
        signature,
        brief,
        docstring,
        line_start,
        line_end,
        col_start,
        col_end,
        parent_id,
        source: extract_source(source, line_start, line_end),
    });
}

/// Extract contract parts (functions, state variables, events, structs, enums).
#[allow(clippy::too_many_arguments)]
fn extract_contract_parts(
    parts: &[ContractPart],
    file_path: &str,
    source: &str,
    contract_name: &str,
    contract_id: &str,
    doc_comments: &[(u32, String)],
    symbols: &mut Vec<Symbol>,
    edges: &mut Vec<Edge>,
) {
    for part in parts {
        match part {
            ContractPart::FunctionDefinition(func) => {
                if let Some(symbol) = extract_function(
                    func,
                    file_path,
                    source,
                    Some(contract_name),
                    Some(contract_id),
                    doc_comments,
                ) {
                    symbols.push(symbol);
                }
            }

            ContractPart::VariableDefinition(var) => {
                if let Some(ref name) = var.name {
                    let visibility = extract_variable_visibility(&var.attrs);
                    let type_str = format_type(&var.ty);

                    push_symbol(
                        symbols,
                        file_path,
                        source,
                        &name.name,
                        Some((contract_name, contract_id)),
                        SymbolKind::Field,
                        visibility,
                        Some(format!("{} {}", type_str, name.name)),
                        &var.loc,
                        doc_comments,
                    );
                }
            }

            ContractPart::EventDefinition(event) => {
                if let Some(ref name) = event.name {
                    push_symbol(
                        symbols,
                        file_path,
                        source,
                        &name.name,
                        Some((contract_name, contract_id)),
                        SymbolKind::Function,
                        Visibility::Public,
                        Some(format!("event {}", name.name)),
                        &event.loc,
                        doc_comments,
                    );
                }
            }

            ContractPart::StructDefinition(def) => {
                if let Some(ref name) = def.name {
                    push_symbol(
                        symbols,
                        file_path,
                        source,
                        &name.name,
                        Some((contract_name, contract_id)),
                        SymbolKind::Struct,
                        Visibility::Public,
                        Some(format!("struct {}", name.name)),
                        &def.loc,
                        doc_comments,
                    );
                }
            }

            ContractPart::EnumDefinition(def) => {
                if let Some(ref name) = def.name {
                    push_symbol(
                        symbols,
                        file_path,
                        source,
                        &name.name,
                        Some((contract_name, contract_id)),
                        SymbolKind::Enum,
                        Visibility::Public,
                        Some(format!("enum {}", name.name)),
                        &def.loc,
                        doc_comments,
                    );
                }
            }

            ContractPart::ErrorDefinition(def) => {
                if let Some(ref name) = def.name {
                    push_symbol(
                        symbols,
                        file_path,
                        source,
                        &name.name,
                        Some((contract_name, contract_id)),
                        SymbolKind::Type,
                        Visibility::Public,
                        Some(format!("error {}", name.name)),
                        &def.loc,
                        doc_comments,
                    );
                }
            }

            ContractPart::Using(using) => {
                // Create an edge for using-for directives
                if let Some(ref ty) = using.ty {
                    let type_name = format_type(ty);
                    edges.push(Edge {
                        source_id: contract_id.to_string(),
                        target_id: None,
                        target_name: type_name,
                        kind: EdgeKind::Uses,
                        line: None,
                        col: None,
                        context: None,
                    });
                }
            }

            _ => {}
        }
    }
}

/// Extract a function definition.
fn extract_function(
    func: &pt::FunctionDefinition,
    file_path: &str,
    source: &str,
    parent_name: Option<&str>,
    parent_id: Option<&str>,
    doc_comments: &[(u32, String)],
) -> Option<Symbol> {
    let (line_start, line_end, col_start, col_end) = loc_to_lines(&func.loc, source);
    let docstring = find_doc_comment(doc_comments, line_start);
    let brief = docstring.as_ref().and_then(|d| extract_brief(d));

    // Get function name (constructors/fallback/receive may not have a name)
    let name = match &func.ty {
        FunctionTy::Constructor => "constructor".to_string(),
        FunctionTy::Fallback => "fallback".to_string(),
        FunctionTy::Receive => "receive".to_string(),
        FunctionTy::Function | FunctionTy::Modifier => func.name.as_ref()?.name.clone(),
    };

    let kind = match &func.ty {
        FunctionTy::Modifier => SymbolKind::Function, // Could use a Modifier kind
        _ => SymbolKind::Function,
    };

    let visibility = extract_function_visibility(&func.attributes);
    let signature = build_function_signature(func, source);

    let qualified_name = parent_name.map(|p| format!("{}.{}", p, name));

    Some(Symbol {
        id: Symbol::make_id(file_path, &name, parent_name),
        file_path: file_path.to_string(),
        name,
        qualified_name,
        kind,
        visibility,
        signature,
        brief,
        docstring,
        line_start,
        line_end,
        col_start,
        col_end,
        parent_id: parent_id.map(String::from),
        source: extract_source(source, line_start, line_end),
    })
}

/// Extract import information.
fn extract_import(import: &pt::Import) -> Option<ImportInfo> {
    match import {
        pt::Import::Plain(path, _) => Some(ImportInfo {
            from: path_to_string(path),
            names: Vec::new(),
            alias: None,
        }),
        pt::Import::GlobalSymbol(path, alias, _) => Some(ImportInfo {
            from: path_to_string(path),
            names: Vec::new(),
            alias: Some(alias.name.clone()),
        }),
        pt::Import::Rename(path, renames, _) => Some(ImportInfo {
            from: path_to_string(path),
            names: renames.iter().map(|(id, _)| id.name.clone()).collect(),
            alias: None,
        }),
    }
}

/// Convert import path to string.
fn path_to_string(path: &pt::ImportPath) -> String {
    match path {
        pt::ImportPath::Filename(lit) => lit.string.clone(),
        pt::ImportPath::Path(ident_path) => ident_path
            .identifiers
            .iter()
            .map(|id| id.name.as_str())
            .collect::<Vec<_>>()
            .join("."),
    }
}

/// Extract function visibility from attributes.
fn extract_function_visibility(attrs: &[FunctionAttribute]) -> Visibility {
    for attr in attrs {
        if let FunctionAttribute::Visibility(vis) = attr {
            match vis {
                SolVisibility::Public(_) | SolVisibility::External(_) => {
                    return Visibility::Public;
                }
                SolVisibility::Internal(_) => return Visibility::Crate,
                SolVisibility::Private(_) => return Visibility::Private,
            }
        }
    }
    // Default visibility in Solidity is internal for state variables,
    // but functions without visibility are a compiler error in recent versions
    Visibility::Private
}

/// Extract variable visibility from attributes.
fn extract_variable_visibility(attrs: &[VariableAttribute]) -> Visibility {
    for attr in attrs {
        if let VariableAttribute::Visibility(vis) = attr {
            match vis {
                SolVisibility::Public(_) | SolVisibility::External(_) => {
                    return Visibility::Public;
                }
                SolVisibility::Internal(_) => return Visibility::Crate,
                SolVisibility::Private(_) => return Visibility::Private,
            }
        }
    }
    // Default visibility for state variables is internal
    Visibility::Crate
}

/// Build a function signature string.
fn build_function_signature(func: &pt::FunctionDefinition, source: &str) -> Option<String> {
    // Get the source text up to the function body
    let (start_line, _, _, _) = loc_to_lines(&func.loc, source);
    let lines: Vec<&str> = source.lines().collect();

    if start_line == 0 || start_line as usize > lines.len() {
        return None;
    }

    // Find the signature (up to the first '{' or ';')
    let mut sig_lines = Vec::new();
    for line in lines.iter().skip(start_line as usize - 1) {
        if let Some(idx) = line.find('{') {
            sig_lines.push(line[..idx].trim());
            break;
        } else if line.trim().ends_with(';') {
            sig_lines.push(line.trim().trim_end_matches(';'));
            break;
        } else {
            sig_lines.push(line.trim());
        }
    }

    let sig = sig_lines.join(" ");
    if sig.is_empty() {
        None
    } else {
        Some(sig)
    }
}

/// Format a type expression to a string.
fn format_type(ty: &Expression) -> String {
    match ty {
        Expression::Type(_, ty) => format_type_inner(ty),
        Expression::Variable(id) => id.name.clone(),
        Expression::MemberAccess(_, expr, member) => {
            format!("{}.{}", format_type(expr), member.name)
        }
        Expression::ArraySubscript(_, expr, size) => {
            let base = format_type(expr);
            match size {
                Some(s) => format!("{}[{}]", base, format_type(s)),
                None => format!("{}[]", base),
            }
        }
        _ => "unknown".to_string(),
    }
}

fn format_type_inner(ty: &pt::Type) -> String {
    match ty {
        pt::Type::Address => "address".to_string(),
        pt::Type::AddressPayable => "address payable".to_string(),
        pt::Type::Payable => "payable".to_string(),
        pt::Type::Bool => "bool".to_string(),
        pt::Type::String => "string".to_string(),
        pt::Type::Bytes(n) => format!("bytes{}", n),
        pt::Type::DynamicBytes => "bytes".to_string(),
        pt::Type::Int(n) => format!("int{}", n),
        pt::Type::Uint(n) => format!("uint{}", n),
        pt::Type::Rational => "rational".to_string(),
        pt::Type::Mapping { key, value, .. } => {
            format!("mapping({} => {})", format_type(key), format_type(value))
        }
        pt::Type::Function { .. } => "function".to_string(),
    }
}

/// Convert a Loc to line/column numbers (1-indexed lines, 0-indexed columns).
fn loc_to_lines(loc: &Loc, source: &str) -> (u32, u32, u32, u32) {
    match loc {
        Loc::File(_, start, end) => {
            let (start_line, start_col) = offset_to_line_col(source, *start);
            let (end_line, end_col) = offset_to_line_col(source, *end);
            (start_line, end_line, start_col, end_col)
        }
        _ => (1, 1, 0, 0),
    }
}

/// Convert byte offset to line and column.
fn offset_to_line_col(source: &str, offset: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut col = 0u32;
    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Extract source code for a range of lines.
fn extract_source(source: &str, start_line: u32, end_line: u32) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    if start_line == 0 || end_line == 0 {
        return None;
    }
    let start = (start_line as usize).saturating_sub(1);
    let end = (end_line as usize).min(lines.len());
    if start >= lines.len() {
        return None;
    }
    Some(lines[start..end].join("\n"))
}

/// Extract the main contract name from source (simple heuristic).
fn extract_contract_name(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        for keyword in &["contract ", "interface ", "library "] {
            if let Some(rest) = trimmed.strip_prefix(keyword) {
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

/// Extract doc comments from the comment list.
/// Returns a list of (end_line, comment_text) for NatSpec comments.
fn extract_doc_comments(comments: &[pt::Comment], source: &str) -> Vec<(u32, String)> {
    let mut result = Vec::new();

    for comment in comments {
        match comment {
            pt::Comment::DocLine(loc, text) => {
                let (_, end_line, _, _) = loc_to_lines(loc, source);
                // Strip the leading "///" and trim
                let content = text.trim_start_matches("///").trim();
                result.push((end_line, content.to_string()));
            }
            pt::Comment::DocBlock(loc, text) => {
                let (_, end_line, _, _) = loc_to_lines(loc, source);
                // Parse the block comment
                let content = parse_doc_block(text);
                result.push((end_line, content));
            }
            _ => {}
        }
    }

    result
}

/// Parse a doc block comment (/** ... */).
fn parse_doc_block(text: &str) -> String {
    text.trim_start_matches("/**")
        .trim_end_matches("*/")
        .lines()
        .map(|l| l.trim().trim_start_matches('*').trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Find doc comment that ends just before the given line.
fn find_doc_comment(comments: &[(u32, String)], target_line: u32) -> Option<String> {
    // Look for comments that end on the line just before target_line
    // or on the same line (for inline comments)
    let mut best: Option<&(u32, String)> = None;

    for comment in comments {
        // Comment should end on line before or same line
        if comment.0 < target_line && comment.0 >= target_line.saturating_sub(3) {
            match best {
                None => best = Some(comment),
                Some(b) if comment.0 > b.0 => best = Some(comment),
                _ => {}
            }
        }
    }

    // Collect consecutive doc comments
    if let Some((end_line, _)) = best {
        let mut doc_lines: Vec<&str> = Vec::new();
        for comment in comments {
            // Collect all comments that are close to each other leading up to end_line
            if comment.0 <= *end_line && comment.0 >= end_line.saturating_sub(10) {
                doc_lines.push(&comment.1);
            }
        }
        if !doc_lines.is_empty() {
            return Some(doc_lines.join("\n"));
        }
    }

    None
}

/// Extract function call edges by re-parsing and walking the AST.
fn extract_call_edges(file_path: &str, source: &str, symbols: &[Symbol], edges: &mut Vec<Edge>) {
    // Build a map of function line ranges to their IDs
    let func_ranges: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .map(|s| (s.line_start, s.line_end, s.id.clone()))
        .collect();

    // Re-parse to walk expressions
    // NOTE: This currently only extracts calls from within contract definitions.
    // Free functions (functions defined at file scope, outside contracts) are not
    // visited for call extraction. This is a known limitation - free function calls
    // won't appear in the call graph. To fix, we'd need to also handle
    // SourceUnitPart::FunctionDefinition for top-level functions.
    if let Ok((tree, _)) = solang_parser::parse(source, 0) {
        for part in &tree.0 {
            match part {
                SourceUnitPart::ContractDefinition(def) => {
                    for cpart in &def.parts {
                        if let ContractPart::FunctionDefinition(func) = cpart {
                            extract_modifier_edges(func, source, &func_ranges, symbols, edges);
                            if let Some(ref body) = func.body {
                                extract_calls_from_statement(
                                    body,
                                    file_path,
                                    source,
                                    &func_ranges,
                                    symbols,
                                    edges,
                                );
                            }
                        }
                    }
                }
                SourceUnitPart::FunctionDefinition(func) => {
                    // Handle free functions (top-level functions outside contracts)
                    extract_modifier_edges(func, source, &func_ranges, symbols, edges);
                    if let Some(ref body) = func.body {
                        extract_calls_from_statement(
                            body,
                            file_path,
                            source,
                            &func_ranges,
                            symbols,
                            edges,
                        );
                    }
                }
                _ => {}
            }
        }
    }
}

/// Emit `calls` edges for each modifier (or base-contract) invocation applied to
/// a function via `FunctionAttribute::BaseOrModifier`.
///
/// This also covers constructor base-contract invocations
/// (e.g. `constructor() Ownable(msg.sender)`); those are treated the same way -
/// an edge to the base name is emitted, resolving `target_id` when a matching
/// symbol exists and leaving it `None` otherwise (mirroring unresolved calls).
fn extract_modifier_edges(
    func: &pt::FunctionDefinition,
    source: &str,
    func_ranges: &[(u32, u32, String)],
    symbols: &[Symbol],
    edges: &mut Vec<Edge>,
) {
    for attr in &func.attributes {
        if let FunctionAttribute::BaseOrModifier(_, base) = attr {
            // The modifier/base name is the last segment of the identifier path.
            let name = match base.name.identifiers.last() {
                Some(id) => id.name.clone(),
                None => continue,
            };

            let (line, _, col, _) = loc_to_lines(&base.loc, source);

            // Find which function this attribute is on.
            let source_id = func_ranges
                .iter()
                .find(|(start, end, _)| line >= *start && line <= *end)
                .map(|(_, _, id)| id.clone());

            if let Some(source_id) = source_id {
                let target_id = symbols
                    .iter()
                    .find(|s| s.name == name && s.kind == SymbolKind::Function)
                    .map(|s| s.id.clone());

                edges.push(Edge {
                    source_id,
                    target_id,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: Some(line),
                    col: Some(col),
                    context: None,
                });
            }
        }
    }
}

/// Extract calls from a list of statements.
fn extract_calls_from_statements(
    statements: &[pt::Statement],
    file_path: &str,
    source: &str,
    func_ranges: &[(u32, u32, String)],
    symbols: &[Symbol],
    edges: &mut Vec<Edge>,
) {
    for stmt in statements {
        extract_calls_from_statement(stmt, file_path, source, func_ranges, symbols, edges);
    }
}

/// Extract calls from a single statement.
fn extract_calls_from_statement(
    stmt: &pt::Statement,
    file_path: &str,
    source: &str,
    func_ranges: &[(u32, u32, String)],
    symbols: &[Symbol],
    edges: &mut Vec<Edge>,
) {
    match stmt {
        pt::Statement::Expression(_, expr) => {
            extract_calls_from_expr(expr, file_path, source, func_ranges, symbols, edges);
        }
        pt::Statement::VariableDefinition(_, _, Some(expr)) => {
            extract_calls_from_expr(expr, file_path, source, func_ranges, symbols, edges);
        }
        pt::Statement::If(_, cond, then_stmt, else_stmt) => {
            extract_calls_from_expr(cond, file_path, source, func_ranges, symbols, edges);
            extract_calls_from_statement(then_stmt, file_path, source, func_ranges, symbols, edges);
            if let Some(else_s) = else_stmt {
                extract_calls_from_statement(
                    else_s,
                    file_path,
                    source,
                    func_ranges,
                    symbols,
                    edges,
                );
            }
        }
        pt::Statement::While(_, cond, body) => {
            extract_calls_from_expr(cond, file_path, source, func_ranges, symbols, edges);
            extract_calls_from_statement(body, file_path, source, func_ranges, symbols, edges);
        }
        pt::Statement::For(_, init, cond, update, body) => {
            if let Some(init_stmt) = init {
                extract_calls_from_statement(
                    init_stmt,
                    file_path,
                    source,
                    func_ranges,
                    symbols,
                    edges,
                );
            }
            if let Some(cond_expr) = cond {
                extract_calls_from_expr(cond_expr, file_path, source, func_ranges, symbols, edges);
            }
            if let Some(update_expr) = update {
                extract_calls_from_expr(
                    update_expr,
                    file_path,
                    source,
                    func_ranges,
                    symbols,
                    edges,
                );
            }
            if let Some(body_stmt) = body {
                extract_calls_from_statement(
                    body_stmt,
                    file_path,
                    source,
                    func_ranges,
                    symbols,
                    edges,
                );
            }
        }
        pt::Statement::DoWhile(_, body, cond) => {
            extract_calls_from_statement(body, file_path, source, func_ranges, symbols, edges);
            extract_calls_from_expr(cond, file_path, source, func_ranges, symbols, edges);
        }
        pt::Statement::Block { statements, .. } => {
            extract_calls_from_statements(
                statements,
                file_path,
                source,
                func_ranges,
                symbols,
                edges,
            );
        }
        pt::Statement::Return(_, Some(expr)) => {
            extract_calls_from_expr(expr, file_path, source, func_ranges, symbols, edges);
        }
        pt::Statement::Emit(_, expr) => {
            extract_calls_from_expr(expr, file_path, source, func_ranges, symbols, edges);
        }
        pt::Statement::Try(_, expr, _, catch_clauses) => {
            extract_calls_from_expr(expr, file_path, source, func_ranges, symbols, edges);
            for clause in catch_clauses {
                match clause {
                    pt::CatchClause::Simple(_, _, stmt) => {
                        extract_calls_from_statement(
                            stmt,
                            file_path,
                            source,
                            func_ranges,
                            symbols,
                            edges,
                        );
                    }
                    pt::CatchClause::Named(_, _, _, stmt) => {
                        extract_calls_from_statement(
                            stmt,
                            file_path,
                            source,
                            func_ranges,
                            symbols,
                            edges,
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

/// Extract calls from an expression.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::only_used_in_recursion)]
fn extract_calls_from_expr(
    expr: &Expression,
    file_path: &str,
    source: &str,
    func_ranges: &[(u32, u32, String)],
    symbols: &[Symbol],
    edges: &mut Vec<Edge>,
) {
    match expr {
        Expression::FunctionCall(loc, func_expr, _args) => {
            let (line, _, col, _) = loc_to_lines(loc, source);

            // Get the function name and, for qualified calls like
            // `LibraryName.fn(...)`, the fully qualified name so the resolver can
            // disambiguate a bare name shared across files/languages.
            let (func_name, context) = match func_expr.as_ref() {
                Expression::Variable(id) => (Some(id.name.clone()), None),
                Expression::MemberAccess(_, object, member) => {
                    // Only capture the qualifier for a simple `Name.member` call;
                    // chained/complex receivers stay unqualified (context: None).
                    let context = match object.as_ref() {
                        Expression::Variable(id) => Some(format!("{}.{}", id.name, member.name)),
                        _ => None,
                    };
                    (Some(member.name.clone()), context)
                }
                _ => (None, None),
            };

            if let Some(name) = func_name {
                // Find which function this call is in
                let source_id = func_ranges
                    .iter()
                    .find(|(start, end, _)| line >= *start && line <= *end)
                    .map(|(_, _, id)| id.clone());

                if let Some(source_id) = source_id {
                    // Try to resolve target
                    let target_id = symbols
                        .iter()
                        .find(|s| s.name == name && s.kind == SymbolKind::Function)
                        .map(|s| s.id.clone());

                    edges.push(Edge {
                        source_id,
                        target_id,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: Some(line),
                        col: Some(col),
                        context,
                    });
                }
            }

            // Recurse into arguments
            for arg in _args {
                extract_calls_from_expr(arg, file_path, source, func_ranges, symbols, edges);
            }
        }

        Expression::FunctionCallBlock(loc, func_expr, _) => {
            // Handle block-style function calls (used with modifiers)
            let (line, _, col, _) = loc_to_lines(loc, source);
            let (func_name, context) = match func_expr.as_ref() {
                Expression::Variable(id) => (Some(id.name.clone()), None),
                Expression::MemberAccess(_, object, member) => {
                    let context = match object.as_ref() {
                        Expression::Variable(id) => Some(format!("{}.{}", id.name, member.name)),
                        _ => None,
                    };
                    (Some(member.name.clone()), context)
                }
                _ => (None, None),
            };

            if let Some(name) = func_name {
                let source_id = func_ranges
                    .iter()
                    .find(|(start, end, _)| line >= *start && line <= *end)
                    .map(|(_, _, id)| id.clone());

                if let Some(source_id) = source_id {
                    let target_id = symbols
                        .iter()
                        .find(|s| s.name == name && s.kind == SymbolKind::Function)
                        .map(|s| s.id.clone());

                    edges.push(Edge {
                        source_id,
                        target_id,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: Some(line),
                        col: Some(col),
                        context,
                    });
                }
            }
        }

        // Recurse into sub-expressions
        Expression::Add(_, l, r)
        | Expression::Subtract(_, l, r)
        | Expression::Multiply(_, l, r)
        | Expression::Divide(_, l, r)
        | Expression::Modulo(_, l, r)
        | Expression::Power(_, l, r)
        | Expression::BitwiseOr(_, l, r)
        | Expression::BitwiseAnd(_, l, r)
        | Expression::BitwiseXor(_, l, r)
        | Expression::ShiftLeft(_, l, r)
        | Expression::ShiftRight(_, l, r)
        | Expression::And(_, l, r)
        | Expression::Or(_, l, r)
        | Expression::Equal(_, l, r)
        | Expression::NotEqual(_, l, r)
        | Expression::Less(_, l, r)
        | Expression::More(_, l, r)
        | Expression::LessEqual(_, l, r)
        | Expression::MoreEqual(_, l, r)
        | Expression::Assign(_, l, r)
        | Expression::AssignAdd(_, l, r)
        | Expression::AssignSubtract(_, l, r)
        | Expression::AssignMultiply(_, l, r)
        | Expression::AssignDivide(_, l, r)
        | Expression::AssignModulo(_, l, r)
        | Expression::AssignOr(_, l, r)
        | Expression::AssignAnd(_, l, r)
        | Expression::AssignXor(_, l, r)
        | Expression::AssignShiftLeft(_, l, r)
        | Expression::AssignShiftRight(_, l, r) => {
            extract_calls_from_expr(l, file_path, source, func_ranges, symbols, edges);
            extract_calls_from_expr(r, file_path, source, func_ranges, symbols, edges);
        }

        Expression::Not(_, e)
        | Expression::BitwiseNot(_, e)
        | Expression::Negate(_, e)
        | Expression::UnaryPlus(_, e)
        | Expression::PreIncrement(_, e)
        | Expression::PreDecrement(_, e)
        | Expression::PostIncrement(_, e)
        | Expression::PostDecrement(_, e)
        | Expression::Parenthesis(_, e) => {
            extract_calls_from_expr(e, file_path, source, func_ranges, symbols, edges);
        }

        Expression::ConditionalOperator(_, cond, then_e, else_e) => {
            extract_calls_from_expr(cond, file_path, source, func_ranges, symbols, edges);
            extract_calls_from_expr(then_e, file_path, source, func_ranges, symbols, edges);
            extract_calls_from_expr(else_e, file_path, source, func_ranges, symbols, edges);
        }

        Expression::ArraySubscript(_, arr, idx) => {
            extract_calls_from_expr(arr, file_path, source, func_ranges, symbols, edges);
            if let Some(i) = idx {
                extract_calls_from_expr(i, file_path, source, func_ranges, symbols, edges);
            }
        }

        Expression::ArraySlice(_, arr, start, end) => {
            extract_calls_from_expr(arr, file_path, source, func_ranges, symbols, edges);
            if let Some(s) = start {
                extract_calls_from_expr(s, file_path, source, func_ranges, symbols, edges);
            }
            if let Some(e) = end {
                extract_calls_from_expr(e, file_path, source, func_ranges, symbols, edges);
            }
        }

        Expression::MemberAccess(_, e, _) => {
            extract_calls_from_expr(e, file_path, source, func_ranges, symbols, edges);
        }

        Expression::New(_, e) => {
            extract_calls_from_expr(e, file_path, source, func_ranges, symbols, edges);
        }

        Expression::List(_, exprs) => {
            for e in exprs {
                if let (_, Some(param)) = e {
                    if let Some(ref init) = param.name {
                        // This is a named parameter with potential expression
                        let _ = init; // Just for clarity - the name itself doesn't contain calls
                    }
                }
            }
        }

        Expression::ArrayLiteral(_, exprs) => {
            for e in exprs {
                extract_calls_from_expr(e, file_path, source, func_ranges, symbols, edges);
            }
        }

        _ => {}
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

        assert_eq!(result.language, "solidity");

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

    #[test]
    fn test_parse_events_and_errors() {
        let mut parser = SolidityParser::new();
        let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Test {
    event Transfer(address indexed from, address indexed to, uint256 value);
    error InsufficientBalance(uint256 available, uint256 required);
}
"#;

        let result = parser.parse("Test.sol", source).unwrap();

        let transfer = result.symbols.iter().find(|s| s.name == "Transfer");
        assert!(transfer.is_some());

        let error = result
            .symbols
            .iter()
            .find(|s| s.name == "InsufficientBalance");
        assert!(error.is_some());
        assert_eq!(error.unwrap().kind, SymbolKind::Type);
    }

    #[test]
    fn test_parse_constructor_and_modifiers() {
        let mut parser = SolidityParser::new();
        let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Ownable {
    address public owner;

    constructor() {
        owner = msg.sender;
    }

    modifier onlyOwner() {
        require(msg.sender == owner, "Not owner");
        _;
    }

    function transferOwnership(address newOwner) public onlyOwner {
        owner = newOwner;
    }
}
"#;

        let result = parser.parse("Ownable.sol", source).unwrap();

        let constructor = result.symbols.iter().find(|s| s.name == "constructor");
        assert!(constructor.is_some());

        let modifier = result.symbols.iter().find(|s| s.name == "onlyOwner");
        assert!(modifier.is_some());

        // Applying the `onlyOwner` modifier to `transferOwnership` should produce a
        // `calls` edge to the modifier, with `target_id` resolved to its symbol.
        let modifier_edge = result
            .edges
            .iter()
            .find(|e| e.target_name == "onlyOwner")
            .expect("expected a calls edge to the onlyOwner modifier");
        assert_eq!(modifier_edge.kind, EdgeKind::Calls);
        assert_eq!(
            modifier_edge.target_id.as_deref(),
            Some(modifier.unwrap().id.as_str()),
            "modifier edge target_id must resolve to the onlyOwner symbol"
        );
    }

    #[test]
    fn test_parse_imports() {
        let mut parser = SolidityParser::new();
        let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./IERC20.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

contract MyToken is IERC20 {}
"#;

        let result = parser.parse("MyToken.sol", source).unwrap();

        assert!(result.module.is_some());
        let module = result.module.unwrap();
        assert_eq!(module.imports.len(), 2);
        assert_eq!(module.imports[0].from, "./IERC20.sol");
    }

    #[test]
    fn test_parse_library() {
        let mut parser = SolidityParser::new();
        let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library SafeMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
"#;

        let result = parser.parse("SafeMath.sol", source).unwrap();

        let library = result.symbols.iter().find(|s| s.name == "SafeMath");
        assert!(library.is_some());
        assert_eq!(library.unwrap().kind, SymbolKind::Module);
    }

    #[test]
    fn test_function_calls_extracted() {
        let mut parser = SolidityParser::new();
        let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Test {
    function helper() internal pure returns (uint256) {
        return 42;
    }

    function main() public pure returns (uint256) {
        return helper();
    }
}
"#;

        let result = parser.parse("Test.sol", source).unwrap();

        // Should have edges for the call from main to helper
        assert!(!result.edges.is_empty());
        let call = result.edges.iter().find(|e| e.target_name == "helper");
        assert!(call.is_some());
        assert_eq!(call.unwrap().kind, EdgeKind::Calls);
    }
}
