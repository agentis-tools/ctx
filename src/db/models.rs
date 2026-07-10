//! Data models for code intelligence.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

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
}

impl FromStr for SymbolKind {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "function" => Ok(SymbolKind::Function),
            "method" => Ok(SymbolKind::Method),
            "struct" => Ok(SymbolKind::Struct),
            "enum" => Ok(SymbolKind::Enum),
            "trait" => Ok(SymbolKind::Trait),
            "impl" => Ok(SymbolKind::Impl),
            "const" => Ok(SymbolKind::Const),
            "static" => Ok(SymbolKind::Static),
            "type" => Ok(SymbolKind::Type),
            "macro" => Ok(SymbolKind::Macro),
            "module" => Ok(SymbolKind::Module),
            "field" => Ok(SymbolKind::Field),
            "variant" => Ok(SymbolKind::Variant),
            "interface" => Ok(SymbolKind::Interface),
            "class" => Ok(SymbolKind::Class),
            "variable" => Ok(SymbolKind::Variable),
            "parameter" => Ok(SymbolKind::Parameter),
            _ => Err(()),
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
}

impl FromStr for Visibility {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "public" | "pub" => Ok(Visibility::Public),
            "crate" | "pub(crate)" => Ok(Visibility::Crate),
            "super" | "pub(super)" => Ok(Visibility::Super),
            "in_path" => Ok(Visibility::InPath),
            _ => Ok(Visibility::Private),
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

/// A stored MinHash fingerprint for a function/method symbol.
///
/// See [`crate::fingerprint`] for how fingerprints are computed. The
/// `minhash` blob is the 128-permutation signature serialized as 1024
/// little-endian bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    /// Symbol id (`path::[parent::]name@line`).
    pub symbol_id: String,
    /// Index-relative path of the file containing the symbol.
    pub file_path: String,
    /// MinHash signature: 128 u64 words, little-endian (1024 bytes).
    pub minhash: Vec<u8>,
    /// Number of normalized tokens in the symbol's token stream.
    pub token_count: i64,
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
}

impl FromStr for EdgeKind {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "calls" => Ok(EdgeKind::Calls),
            "imports" => Ok(EdgeKind::Imports),
            "uses" => Ok(EdgeKind::Uses),
            "extends" => Ok(EdgeKind::Extends),
            "implements" => Ok(EdgeKind::Implements),
            "returns" => Ok(EdgeKind::Returns),
            "parameter" => Ok(EdgeKind::Parameter),
            "field" => Ok(EdgeKind::Field),
            "contains" => Ok(EdgeKind::Contains),
            _ => Err(()),
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
