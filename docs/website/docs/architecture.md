---
id: architecture
title: Architecture
sidebar_position: 7
---

# Architecture

This document explains how ctx works under the hood, the design decisions made, and how the components fit together.

## High-Level Overview

ctx is built around two main capabilities:

1. **Context Generation** - Walk files, apply filters, format output
2. **Code Intelligence** - Parse code, extract symbols, build queryable database

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              ctx CLI                                        │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌────────────────────────┐    ┌─────────────────────────────────────────┐  │
│  │  Context Generation    │    │           Code Intelligence             │  │
│  │                        │    │                                         │  │
│  │  ┌──────────────────┐  │    │  ┌─────────────────────────────────┐    │  │
│  │  │  File Walker     │  │    │  │         Tree-sitter              │   │  │
│  │  │  (ignore crate)  │  │    │  │    (Multi-language parsing)      │   │  │
│  │  └────────┬─────────┘  │    │  └──────────────┬──────────────────┘    │  │
│  │           │            │    │                 │                       │  │
│  │  ┌────────▼─────────┐  │    │  ┌──────────────▼──────────────────┐    │  │
│  │  │  Ignore System   │  │    │  │           SQLite                │    │  │
│  │  │  (.gitignore,    │  │    │  │  • Symbols table                │    │  │
│  │  │   .contextignore,│  │    │  │  • Edges table (call graph)     │    │  │
│  │  │   built-in)      │  │    │  │  • FTS5 search index            │    │  │
│  │  └────────┬─────────┘  │    │  │  • Embeddings table             │    │  │
│  │           │            │    │  │  • Compressed source            │    │  │
│  │  ┌────────▼─────────┐  │    │  └──────────────┬──────────────────┘    │  │
│  │  │   Formatters     │  │    │                 │                       │  │
│  │  │  (XML, Markdown, │  │    │  ┌──────────────▼──────────────────┐    │  │
│  │  │   Plain)         │  │    │  │          DuckDB                 │    │  │
│  │  └────────┬─────────┘  │    │  │  (In-memory analytical queries) │    │  │
│  │           │            │    │  │  • Recursive CTEs               │    │  │
│  │  ┌────────▼─────────┐  │    │  │  • Graph traversal              │    │  │
│  │  │  Output Stream   │  │    │  │  • Aggregations                 │    │  │
│  │  └──────────────────┘  │    │  └─────────────────────────────────┘    │  │
│  └────────────────────────┘    └─────────────────────────────────────────┘  │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                        Embeddings Module                              │  │
│  │  ┌─────────────────────┐    ┌─────────────────────────────────────┐   │  │
│  │  │   Local Provider    │    │         OpenAI Provider             │   │  │
│  │  │   (fastembed)       │    │    (text-embedding-3-small)         │   │  │
│  │  │   384 dimensions    │    │         1536 dimensions             │   │  │
│  │  └─────────────────────┘    └─────────────────────────────────────┘   │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Module Structure

```
src/
├── main.rs           # CLI entry point, command routing
├── cli.rs            # Argument parsing (clap)
├── walker.rs         # File discovery (ignore crate)
├── default_ignores.rs # Built-in ignore patterns (170+)
├── formatter.rs      # Output formatters (XML, Markdown, Plain)
├── output.rs         # Context generation logic
├── tree.rs           # ASCII tree visualization
├── parser/
│   ├── mod.rs        # Parser coordinator, shared utilities
│   ├── rust.rs       # Rust parser (tree-sitter-rust)
│   ├── typescript.rs # TS/JS parser (tree-sitter-typescript)
│   ├── python.rs     # Python parser (tree-sitter-python)
│   ├── go.rs         # Go parser (tree-sitter-go)
│   ├── yaml.rs       # YAML parser (tree-sitter-yaml)
│   └── solidity.rs   # Solidity parser (tree-sitter-solidity)
├── db/
│   ├── mod.rs        # Database module exports
│   ├── models.rs     # Data structures (Symbol, Edge, etc.)
│   └── schema.rs     # SQLite operations
├── index/
│   └── mod.rs        # Indexing logic, watch mode
├── analytics/
│   └── mod.rs        # DuckDB queries, graph analysis
└── embeddings/
    ├── mod.rs        # Embedding traits, similarity search
    ├── local.rs      # Local fastembed provider
    └── openai.rs     # OpenAI API provider
```

## Storage: Single SQLite File

All data lives in `.ctx/codebase.sqlite`:

```sql
-- Files table: track what's been indexed
CREATE TABLE files (
    path TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,      -- SHA256 for change detection
    size_bytes INTEGER,
    language TEXT,
    last_indexed INTEGER,
    source BLOB                      -- gzip compressed source
);

-- Symbols table: functions, classes, etc.
CREATE TABLE symbols (
    id TEXT PRIMARY KEY,             -- "file_path::name@line"
    file_path TEXT NOT NULL,
    name TEXT NOT NULL,
    qualified_name TEXT,
    kind TEXT NOT NULL,              -- function, struct, enum, etc.
    visibility TEXT DEFAULT 'private',
    signature TEXT,
    brief TEXT,                      -- First line of docstring
    docstring TEXT,
    line_start INTEGER,
    line_end INTEGER,
    col_start INTEGER,
    col_end INTEGER,
    parent_id TEXT,
    source TEXT,                     -- Symbol source code
    FOREIGN KEY (file_path) REFERENCES files(path) ON DELETE CASCADE
);

-- Edges table: relationships between symbols
CREATE TABLE edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id TEXT NOT NULL,         -- Caller symbol ID
    target_id TEXT,                  -- Resolved symbol ID (if found)
    target_name TEXT NOT NULL,       -- Called function name
    kind TEXT NOT NULL,              -- 'calls', 'extends', 'implements', 'imports'
    line INTEGER,
    col INTEGER,
    context TEXT,                    -- Code snippet
    FOREIGN KEY (source_id) REFERENCES symbols(id) ON DELETE CASCADE
);

-- Modules table: file-level import/export info
CREATE TABLE modules (
    file_path TEXT PRIMARY KEY,
    module_name TEXT,
    exports TEXT,                    -- JSON array
    imports TEXT,                    -- JSON array of ImportInfo
    FOREIGN KEY (file_path) REFERENCES files(path) ON DELETE CASCADE
);

-- FTS5 virtual table for full-text search
CREATE VIRTUAL TABLE symbol_fts USING fts5(
    id, name, kind, signature, brief, docstring,
    content='symbols',
    content_rowid='rowid'
);

-- Embeddings for semantic search
CREATE TABLE embeddings (
    symbol_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,          -- 'local' or 'openai'
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL,
    vector TEXT NOT NULL,            -- JSON array of floats
    created_at INTEGER,
    FOREIGN KEY (symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
);
```

### Why SQLite?

1. **Single file** - Easy to manage, backup, delete, share
2. **ACID transactions** - Safe concurrent access
3. **FTS5** - Built-in full-text search with BM25 ranking
4. **Widespread support** - Can inspect with any SQLite tool
5. **Fast writes** - Optimized for insert-heavy indexing
6. **No server** - No daemon processes needed

## Analytics: DuckDB

For complex analytical queries (recursive CTEs, graph traversal), we use DuckDB as an in-memory query engine:

```rust
// Open in-memory DuckDB
let conn = Connection::open_in_memory()?;

// Attach SQLite database read-only
conn.execute(
    "ATTACH 'codebase.sqlite' AS code (TYPE sqlite, READ_ONLY)",
    [],
)?;

// Run recursive CTE for call graph
conn.execute("
    WITH RECURSIVE graph AS (
        -- Base case: starting symbol
        SELECT name, file_path, kind, 1 as depth, id
        FROM code.symbols WHERE name = ?
        
        UNION ALL
        
        -- Recursive case: follow edges
        SELECT t.name, t.file_path, t.kind, g.depth + 1, t.id
        FROM graph g
        JOIN code.edges e ON e.source_id = g.id
        LEFT JOIN code.symbols t ON e.target_name = t.name
        WHERE g.depth < ?
    )
    SELECT DISTINCT name, file_path, kind, depth FROM graph
", [start_name, max_depth])?;
```

### Why DuckDB?

1. **Columnar storage** - Efficient for analytical queries
2. **Recursive CTEs** - Native support for graph traversal
3. **SQLite attachment** - Query SQLite data directly, no ETL
4. **In-memory** - No additional files to manage
5. **OLAP optimized** - Fast aggregations and joins

### No Separate File

DuckDB runs entirely in-memory and reads from SQLite:
- No data duplication
- No sync issues
- Single source of truth
- Simpler deployment

## Parsing: Tree-sitter

We use tree-sitter for parsing source code:

```rust
// Create parser with language grammar
let mut parser = Parser::new();
parser.set_language(tree_sitter_rust::language())?;

// Parse source into AST
let tree = parser.parse(source, None)?;

// Query for patterns using S-expression syntax
let query = Query::new(
    tree_sitter_rust::language(),
    r#"
    (function_item 
      name: (identifier) @func.name
      body: (block) @func.body)
    "#
)?;

// Extract matches
let mut cursor = QueryCursor::new();
for match_ in cursor.matches(&query, tree.root_node(), source.as_bytes()) {
    for capture in match_.captures {
        let name = &query.capture_names()[capture.index as usize];
        let text = capture.node.utf8_text(source.as_bytes())?;
        // Process capture...
    }
}
```

### Why Tree-sitter?

1. **Fast** - Incremental parsing, handles large files
2. **Accurate** - Real parser, not regex-based
3. **Multi-language** - Same API for all languages
4. **Error tolerant** - Parses incomplete/invalid code
5. **Battle-tested** - Used by GitHub, Neovim, Zed, etc.

### Language Crates

| Language | Crate | Grammar |
|----------|-------|---------|
| Rust | `tree-sitter-rust` | Official |
| TypeScript | `tree-sitter-typescript` | Official |
| JavaScript | `tree-sitter-javascript` | Official |
| Python | `tree-sitter-python` | Official |
| Go | `tree-sitter-go` | Official |
| YAML | `tree-sitter-yaml` | Community |
| Solidity | `tree-sitter-solidity` | Community |

## File Walking

The `ignore` crate handles file discovery:

```rust
let mut builder = WalkBuilder::new(root);

// Respect .gitignore
builder.git_ignore(config.use_gitignore);
builder.git_global(config.use_gitignore);
builder.git_exclude(config.use_gitignore);

// Add .contextignore support
builder.add_custom_ignore_filename(".contextignore");

// Build overrides for default ignores and -i flags
let mut override_builder = OverrideBuilder::new(&root);
for pattern in DEFAULT_IGNORES {
    override_builder.add(&format!("!{}", pattern))?;
}

builder.overrides(override_builder.build()?);

// Walk and collect files
for entry in builder.build() {
    if is_binary_file(&entry.path()) {
        continue;
    }
    // Process file...
}
```

### Three-Tier Ignore System

1. **Built-in patterns** - 170+ common non-source patterns
2. **`.gitignore`** - Standard git ignores (unless `--no-gitignore`)
3. **`.contextignore`** - Project-specific exclusions

## Incremental Indexing

We track file changes using content hashes:

```rust
fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn needs_update(&self, path: &str, new_hash: &str) -> bool {
    match self.get_file_hash(path)? {
        Some(stored_hash) => stored_hash != new_hash,
        None => true,  // File not in database
    }
}
```

Benefits:
- Only changed files are re-parsed
- Unchanged files are skipped instantly
- Hash comparison is faster than timestamp checks
- Content-based (not modified time which can be unreliable)

## Source Compression

We store compressed source for `ctx source` queries:

```rust
fn compress_source(content: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(content.as_bytes())?;
    encoder.finish()?
}
```

Typical compression: 60-70% size reduction.

## Symbol IDs

Symbols are identified by a composite key:

```
file_path::name@line
```

Examples:
```
src/auth/handler.ts::handleAuth@45
src/parser/mod.rs::CodeParser::parse@86
src/db/models.rs::Symbol::make_id@156
```

This ensures uniqueness even with:
- Same function name in different files
- Overloaded functions
- Nested functions
- Multiple impls of the same name

## Call Graph Extraction

We extract calls using tree-sitter queries:

```scheme
; Rust function calls
(call_expression
  function: (identifier) @call.name) @call.expr

; Method calls  
(call_expression
  function: (field_expression
    field: (field_identifier) @method_call.name)) @method_call.expr

; Scoped calls (module::function)
(call_expression
  function: (scoped_identifier
    name: (identifier) @scoped_call.name)) @scoped_call.expr
```

Edges are stored with context:
```rust
edges.push(Edge {
    source_id: caller_id,
    target_id: resolved_target,  // If found in codebase
    target_name: "processData",
    kind: EdgeKind::Calls,
    line: Some(15),
    col: Some(4),
    context: Some("let result = processData(input);"),
});
```

## Embeddings Architecture

### Provider Trait

```rust
pub trait EmbeddingProvider: Send + Sync {
    fn name(&self) -> &str;
    fn dimension(&self) -> usize;
    fn embed(&self, text: &str) -> Result<Embedding>;
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>>;
}
```

### Local Provider (fastembed)

```rust
pub struct LocalProvider {
    model: TextEmbedding,
}

impl LocalProvider {
    pub fn new() -> Result<Self> {
        // Downloads ~90MB model on first run
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2)
        )?;
        Ok(Self { model })
    }
}
```

### OpenAI Provider

```rust
pub struct OpenAIProvider {
    api_key: String,
    model: String,  // "text-embedding-3-small"
}

impl OpenAIProvider {
    pub fn embed(&self, text: &str) -> Result<Embedding> {
        // HTTPS request to OpenAI API
        let response = self.client.post("https://api.openai.com/v1/embeddings")
            .json(&json!({
                "model": self.model,
                "input": text
            }))
            .send()?;
        // Parse response...
    }
}
```

### Similarity Search

```rust
pub fn semantic_search(
    db: &Database,
    query_embedding: &Embedding,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    // Load all embeddings from database
    let all_embeddings = db.get_all_embeddings()?;
    
    // Compute cosine similarity for each
    let mut scored: Vec<_> = all_embeddings
        .into_iter()
        .map(|(id, name, kind, file, line, vector)| {
            let score = cosine_similarity(&query_embedding.vector, &vector);
            SearchResult { symbol_id: id, name, kind, file_path: file, line, score }
        })
        .collect();
    
    // Sort by score descending
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    scored.truncate(limit);
    
    Ok(scored)
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm_a * norm_b)
}
```

## Output Streaming

Context generation streams output for efficiency:

```rust
pub fn stream_context(
    root: &Path,
    entries: &[FileEntry],
    format: &OutputFormat,
    include_tree: bool,
    show_sizes: bool,
) -> io::Result<ContextResult> {
    let formatter = get_formatter(format);
    
    // Generate and output tree
    if include_tree {
        let tree = generate_tree(root, entries, show_sizes);
        let tree_block = formatter.format_tree(&tree);
        print!("{}", formatter.stream_start(Some(&tree_block)));
    }
    
    // Stream each file
    for entry in entries {
        let content = fs::read_to_string(&entry.absolute_path)?;
        let file_block = formatter.format_file(entry, &content);
        print!("{}{}", formatter.separator(), file_block);
        io::stdout().flush()?;  // Immediate output
    }
    
    println!("{}", formatter.stream_end());
    
    Ok(ContextResult { ... })
}
```

## Performance Characteristics

### Indexing
- ~2000 files in less than 10 seconds
- Incremental updates: under 1 second for changed files
- Memory: ~100MB for large codebases

### Queries
- Symbol search: under 10ms
- Call graph (depth 5): under 50ms
- Impact analysis: under 100ms
- Semantic search: ~100ms (depends on embedding count)

### Storage
- Symbols: ~500 bytes each (uncompressed)
- Compressed source: ~30% of original size
- Embeddings: ~1.5KB per symbol (384d) or ~6KB (1536d)
- Typical project: 10-50MB database

## Design Principles

1. **Single file** - One `.ctx/codebase.sqlite`, no complexity
2. **Incremental** - Only do work when needed
3. **Portable** - Standard formats, no daemon processes
4. **Fast** - Rust + SQLite + DuckDB = speed
5. **Accurate** - Real parsers, not regex hacks
6. **Offline-first** - Local embeddings work without internet

## Data Flow

### Context Generation

```
User Request → File Patterns → Walker → Ignore Filters → 
→ Binary Detection → Formatter → Stream Output
```

### Indexing

```
Walker → Parser Selection → Tree-sitter Parse → 
→ Symbol Extraction → Edge Extraction → 
→ Hash Check → SQLite Insert → FTS Trigger Update
```

### Query

```
User Query → SQLite FTS5 (keyword) → Results
     OR
User Query → Load Embeddings → Cosine Similarity → Results
     OR  
User Query → DuckDB Recursive CTE → Graph Traversal → Results
```

## Dependencies

Key crates and their roles:

| Crate | Purpose |
|-------|---------|
| `clap` | CLI argument parsing |
| `ignore` | .gitignore-aware file walking |
| `globset` | Glob pattern matching |
| `rusqlite` | SQLite database operations |
| `duckdb` | In-memory analytical queries |
| `tree-sitter` | AST parsing |
| `tree-sitter-*` | Language grammars |
| `flate2` | gzip compression |
| `sha2` | Content hashing |
| `fastembed` | Local embedding generation |
| `native-tls` | HTTPS for OpenAI API |
| `serde/serde_json` | Serialization |
| `notify` | File system watching |
