//! Data models for code intelligence.

use serde::{Deserialize, Serialize};

/// Represents a tracked file in the codebase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: String,
    pub content_hash: String,
    pub size_bytes: i64,
    pub language: Option<String>,
    pub last_indexed: i64,
}

/// The kind of symbol (function, struct, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Const,
    Static,
    Type,
    Macro,
    Module,
    Field,
    Variant,
    Interface,
    Class,
    Variable,
    Parameter,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Impl => "impl",
            SymbolKind::Const => "const",
            SymbolKind::Static => "static",
            SymbolKind::Type => "type",
            SymbolKind::Macro => "macro",
            SymbolKind::Module => "module",
            SymbolKind::Field => "field",
            SymbolKind::Variant => "variant",
            SymbolKind::Interface => "interface",
            SymbolKind::Class => "class",
            SymbolKind::Variable => "variable",
            SymbolKind::Parameter => "parameter",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "function" => Some(SymbolKind::Function),
            "method" => Some(SymbolKind::Method),
            "struct" => Some(SymbolKind::Struct),
            "enum" => Some(SymbolKind::Enum),
            "trait" => Some(SymbolKind::Trait),
            "impl" => Some(SymbolKind::Impl),
            "const" => Some(SymbolKind::Const),
            "static" => Some(SymbolKind::Static),
            "type" => Some(SymbolKind::Type),
            "macro" => Some(SymbolKind::Macro),
            "module" => Some(SymbolKind::Module),
            "field" => Some(SymbolKind::Field),
            "variant" => Some(SymbolKind::Variant),
            "interface" => Some(SymbolKind::Interface),
            "class" => Some(SymbolKind::Class),
            "variable" => Some(SymbolKind::Variable),
            "parameter" => Some(SymbolKind::Parameter),
            _ => None,
        }
    }
}

/// Visibility of a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    #[default]
    Private,
    Crate,
    Super,
    InPath,
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
            Visibility::Crate => "crate",
            Visibility::Super => "super",
            Visibility::InPath => "in_path",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "public" | "pub" => Visibility::Public,
            "crate" | "pub(crate)" => Visibility::Crate,
            "super" | "pub(super)" => Visibility::Super,
            "in_path" => Visibility::InPath,
            _ => Visibility::Private,
        }
    }
}

/// A code symbol (function, struct, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    /// Unique identifier: 'file_path::symbol_name' or 'file_path::parent::symbol_name'
    pub id: String,
    /// Path to the file containing this symbol
    pub file_path: String,
    /// Simple name of the symbol
    pub name: String,
    /// Fully qualified name (e.g., module::submodule::name)
    pub qualified_name: Option<String>,
    /// Kind of symbol
    pub kind: SymbolKind,
    /// Visibility
    pub visibility: Visibility,
    /// Function/method signature or type definition
    pub signature: Option<String>,
    /// Brief one-line description (from doc comment)
    pub brief: Option<String>,
    /// Full docstring
    pub docstring: Option<String>,
    /// Starting line (1-indexed)
    pub line_start: u32,
    /// Ending line (1-indexed)
    pub line_end: u32,
    /// Starting column (0-indexed)
    pub col_start: u32,
    /// Ending column (0-indexed)
    pub col_end: u32,
    /// Parent symbol ID (e.g., impl block for methods)
    pub parent_id: Option<String>,
    /// Source code of just this symbol
    pub source: Option<String>,
}

impl Symbol {
    /// Create a unique ID for this symbol.
    pub fn make_id(file_path: &str, name: &str, parent: Option<&str>) -> String {
        match parent {
            Some(p) => format!("{}::{}::{}", file_path, p, name),
            None => format!("{}::{}", file_path, name),
        }
    }

    /// Create a unique ID for this symbol with line number for disambiguation.
    pub fn make_id_with_line(
        file_path: &str,
        name: &str,
        parent: Option<&str>,
        line: u32,
    ) -> String {
        match parent {
            Some(p) => format!("{}::{}::{}@{}", file_path, p, name, line),
            None => format!("{}::{}@{}", file_path, name, line),
        }
    }

    /// Generate text for embedding (semantic search).
    pub fn to_embedding_text(&self) -> String {
        let mut parts = vec![self.name.clone(), self.kind.as_str().to_string()];

        if let Some(ref sig) = self.signature {
            parts.push(sig.clone());
        }

        if let Some(ref brief) = self.brief {
            parts.push(brief.clone());
        }

        // Add semantic hints based on kind
        match self.kind {
            SymbolKind::Function | SymbolKind::Method => {
                parts.push("function method procedure".into());
            }
            SymbolKind::Struct | SymbolKind::Class => {
                parts.push("struct type data structure class".into());
            }
            SymbolKind::Enum => {
                parts.push("enum enumeration variant".into());
            }
            SymbolKind::Trait | SymbolKind::Interface => {
                parts.push("trait interface contract".into());
            }
            _ => {}
        }

        parts.join(" ")
    }
}

/// The kind of relationship between symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Calls,
    Imports,
    Uses,
    Extends,
    Implements,
    Returns,
    Parameter,
    Field,
    Contains,
}

impl EdgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeKind::Calls => "calls",
            EdgeKind::Imports => "imports",
            EdgeKind::Uses => "uses",
            EdgeKind::Extends => "extends",
            EdgeKind::Implements => "implements",
            EdgeKind::Returns => "returns",
            EdgeKind::Parameter => "parameter",
            EdgeKind::Field => "field",
            EdgeKind::Contains => "contains",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "calls" => Some(EdgeKind::Calls),
            "imports" => Some(EdgeKind::Imports),
            "uses" => Some(EdgeKind::Uses),
            "extends" => Some(EdgeKind::Extends),
            "implements" => Some(EdgeKind::Implements),
            "returns" => Some(EdgeKind::Returns),
            "parameter" => Some(EdgeKind::Parameter),
            "field" => Some(EdgeKind::Field),
            "contains" => Some(EdgeKind::Contains),
            _ => None,
        }
    }
}

/// A relationship between symbols.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// Source symbol ID
    pub source_id: String,
    /// Target symbol ID (None if unresolved/external)
    pub target_id: Option<String>,
    /// Target name (for unresolved references)
    pub target_name: String,
    /// Kind of relationship
    pub kind: EdgeKind,
    /// Line where the reference occurs
    pub line: Option<u32>,
    /// Column where the reference occurs
    pub col: Option<u32>,
    /// Brief context snippet
    pub context: Option<String>,
}

/// Module-level information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub file_path: String,
    pub module_name: Option<String>,
    pub exports: Vec<String>,
    pub imports: Vec<ImportInfo>,
}

/// Import information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInfo {
    pub from: String,
    pub names: Vec<String>,
    pub alias: Option<String>,
}

/// Result of parsing a file.
#[derive(Debug, Clone)]
pub struct ParseResult {
    #[allow(dead_code)]
    pub file_path: String,
    pub language: String,
    pub symbols: Vec<Symbol>,
    pub edges: Vec<Edge>,
    pub module: Option<ModuleInfo>,
}

/// Statistics about the codebase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodebaseStats {
    pub files: i64,
    pub symbols: i64,
    pub edges: i64,
    pub functions: i64,
    pub structs: i64,
    pub enums: i64,
    pub traits: i64,
}
