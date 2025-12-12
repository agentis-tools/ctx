# Hybrid Code Intelligence System

A multi-engine architecture combining SQLite, DuckDB, and vector search to give AI agents complete codebase understanding with minimal context overhead.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         HYBRID ARCHITECTURE                             │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  ┌──────────────────────┐  ┌──────────────────────┐  ┌───────────────┐  │
│  │       SQLite         │  │       DuckDB         │  │    Vectors    │  │
│  │   (Transactional)    │  │    (Analytical)      │  │  (Semantic)   │  │
│  ├──────────────────────┤  ├──────────────────────┤  ├───────────────┤  │
│  │ • Source of truth    │  │ • Graph traversal    │  │ • "Find auth  │  │
│  │ • Incremental writes │  │ • Call graphs        │  │    functions" │  │
│  │ • File watching      │  │ • Impact analysis    │  │ • Similar     │  │
│  │ • Full source code   │  │ • Aggregations       │  │   code search │  │
│  │ • Symbol lookups     │  │ • Module deps        │  │ • Natural     │  │
│  │ • ACID guarantees    │  │ • Complex joins      │  │   language    │  │
│  └──────────┬───────────┘  └──────────┬───────────┘  └───────┬───────┘  │
│             │                         │                      │          │
│             └─────────────────────────┼──────────────────────┘          │
│                                       │                                 │
│                              ┌────────▼────────┐                        │
│                              │   Unified API   │                        │
│                              │   for Agents    │                        │
│                              └─────────────────┘                        │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

## Why Three Engines?

### SQLite: The Source of Truth

```
Strengths:
✓ Fast single-row lookups by ID
✓ ACID transactions for safe incremental updates
✓ Efficient file change detection
✓ Store full source code (BLOB-friendly)
✓ Lightweight, embedded, zero config
✓ sqlite-vec extension for vectors

Use for:
• "Get the source of function X"
• "Update index for changed file"
• "Store embedding for symbol"
• Incremental watch mode updates
```

### DuckDB: The Analytical Engine

```
Strengths:
✓ Columnar storage = fast scans
✓ Recursive CTEs for graph traversal
✓ Parallel query execution
✓ Complex aggregations and joins
✓ Can attach/query SQLite directly
✓ Built-in vector similarity functions

Use for:
• "What's the full call graph from main()?"
• "Impact analysis: what breaks if I change X?"
• "Module dependency graph"
• "Statistics across entire codebase"
```

### Vectors: Semantic Understanding

```
Strengths:
✓ Natural language queries
✓ Find conceptually similar code
✓ No need to know exact names
✓ Discover patterns across codebase

Use for:
• "Find functions that handle authentication"
• "Code similar to this error handler"
• "Where do we validate user input?"
• "Functions related to caching"
```

## Schema Design

### SQLite Schema (Primary Storage)

```sql
-- SQLite: codebase.sqlite

-- File tracking for incremental updates
CREATE TABLE files (
    path TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    size_bytes INTEGER,
    language TEXT,
    last_indexed INTEGER DEFAULT (unixepoch()),
    source BLOB                    -- Full file content (compressed)
);

-- All symbols (functions, structs, etc.)
CREATE TABLE symbols (
    id TEXT PRIMARY KEY,           -- 'src/walker.rs::discover_files'
    file_path TEXT NOT NULL,
    name TEXT NOT NULL,
    qualified_name TEXT,
    kind TEXT NOT NULL,
    visibility TEXT DEFAULT 'private',
    signature TEXT,
    brief TEXT,
    docstring TEXT,
    line_start INTEGER,
    line_end INTEGER,
    col_start INTEGER,
    col_end INTEGER,
    parent_id TEXT,
    source TEXT,                   -- Just this symbol's source
    FOREIGN KEY (file_path) REFERENCES files(path) ON DELETE CASCADE
);

-- Relationships between symbols
CREATE TABLE edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id TEXT NOT NULL,
    target_id TEXT,                -- NULL if unresolved/external
    target_name TEXT NOT NULL,
    kind TEXT NOT NULL,            -- 'calls', 'imports', 'extends', etc.
    line INTEGER,
    col INTEGER,
    context TEXT,                  -- Brief snippet: "let x = foo()"
    FOREIGN KEY (source_id) REFERENCES symbols(id) ON DELETE CASCADE
);

-- Module-level information
CREATE TABLE modules (
    file_path TEXT PRIMARY KEY,
    module_name TEXT,
    exports TEXT,                  -- JSON array
    imports TEXT,                  -- JSON array of {from, names}
    FOREIGN KEY (file_path) REFERENCES files(path) ON DELETE CASCADE
);

-- Vector embeddings (using sqlite-vec)
CREATE VIRTUAL TABLE symbol_vectors USING vec0(
    id TEXT PRIMARY KEY,
    embedding FLOAT[1536]          -- OpenAI ada-002 dimension
);

-- Indexes for fast lookups
CREATE INDEX idx_symbols_name ON symbols(name);
CREATE INDEX idx_symbols_file ON symbols(file_path);
CREATE INDEX idx_symbols_kind ON symbols(kind);
CREATE INDEX idx_symbols_parent ON symbols(parent_id);
CREATE INDEX idx_edges_source ON edges(source_id);
CREATE INDEX idx_edges_target ON edges(target_name);
CREATE INDEX idx_edges_kind ON edges(kind);
CREATE INDEX idx_files_hash ON files(content_hash);
```

### DuckDB Views (Analytical Layer)

```sql
-- DuckDB attaches SQLite for complex queries
-- Run once at startup:
ATTACH 'codebase.sqlite' AS code (TYPE sqlite);

-- Create optimized views for common analytical queries

-- Materialized call graph edges (for fast traversal)
CREATE TABLE call_graph AS
SELECT 
    e.source_id,
    e.target_name,
    s.file_path as source_file,
    s.name as source_name,
    s.kind as source_kind,
    t.file_path as target_file,
    t.id as target_id
FROM code.edges e
JOIN code.symbols s ON e.source_id = s.id
LEFT JOIN code.symbols t ON e.target_name = t.name
WHERE e.kind = 'calls';

-- Module dependency matrix
CREATE TABLE module_deps AS
SELECT 
    m.file_path as source_module,
    json_extract(imp.value, '$.from') as target_module,
    json_extract(imp.value, '$.names') as imported_names
FROM code.modules m,
     json_each(m.imports) imp
WHERE json_extract(imp.value, '$.from') NOT LIKE 'std%';

-- Symbol statistics by file
CREATE TABLE file_stats AS
SELECT 
    file_path,
    COUNT(*) as symbol_count,
    COUNT(*) FILTER (WHERE kind = 'function') as functions,
    COUNT(*) FILTER (WHERE kind = 'struct') as structs,
    COUNT(*) FILTER (WHERE kind = 'enum') as enums,
    COUNT(*) FILTER (WHERE visibility = 'public') as public_symbols
FROM code.symbols
GROUP BY file_path;
```

## Query Routing

The unified API routes queries to the appropriate engine:

```rust
pub struct CodeIntelligence {
    sqlite: Connection,      // rusqlite
    duckdb: Connection,      // duckdb
}

impl CodeIntelligence {
    /// Route to SQLite: single symbol lookup
    pub fn get_symbol(&self, id: &str) -> Option<Symbol> {
        // SQLite: fast point lookup
        self.sqlite.query_row(
            "SELECT * FROM symbols WHERE id = ?",
            [id],
            |row| Ok(Symbol::from_row(row))
        ).ok()
    }

    /// Route to SQLite: get full source
    pub fn get_source(&self, symbol_id: &str) -> Option<String> {
        // SQLite: retrieve stored source
        self.sqlite.query_row(
            "SELECT source FROM symbols WHERE id = ?",
            [symbol_id],
            |row| row.get(0)
        ).ok()
    }

    /// Route to DuckDB: call graph traversal
    pub fn call_graph(&self, start: &str, max_depth: i32) -> Vec<CallGraphNode> {
        // DuckDB: recursive CTE
        self.duckdb.prepare(&format!(r#"
            WITH RECURSIVE graph AS (
                SELECT target_name, target_file, 1 as depth
                FROM call_graph 
                WHERE source_name = '{}'
                
                UNION
                
                SELECT cg.target_name, cg.target_file, g.depth + 1
                FROM graph g
                JOIN call_graph cg ON cg.source_name = g.target_name
                WHERE g.depth < {}
            )
            SELECT DISTINCT * FROM graph ORDER BY depth
        "#, start, max_depth))
        .query_map([], CallGraphNode::from_row)
        .collect()
    }

    /// Route to DuckDB: impact analysis
    pub fn impact_analysis(&self, symbol: &str, max_depth: i32) -> Vec<ImpactNode> {
        // DuckDB: reverse call graph traversal
        self.duckdb.prepare(r#"
            WITH RECURSIVE impact AS (
                SELECT source_id, source_name, source_file, 1 as distance
                FROM call_graph WHERE target_name = ?
                
                UNION
                
                SELECT cg.source_id, cg.source_name, cg.source_file, i.distance + 1
                FROM impact i
                JOIN call_graph cg ON cg.target_name = i.source_name
                WHERE i.distance < ?
            )
            SELECT DISTINCT source_name, source_file, MIN(distance) as distance
            FROM impact 
            GROUP BY source_name, source_file
            ORDER BY distance
        "#).query_map([symbol, max_depth], ImpactNode::from_row)
        .collect()
    }

    /// Route to Vectors: semantic search
    pub fn semantic_search(&self, query: &str, limit: i32) -> Vec<SemanticMatch> {
        // 1. Generate embedding for query
        let query_embedding = self.embed(query);
        
        // 2. Vector similarity search in SQLite
        self.sqlite.prepare(r#"
            SELECT 
                s.id, s.name, s.kind, s.file_path, s.signature, s.brief,
                vec_distance_cosine(v.embedding, ?) as distance
            FROM symbol_vectors v
            JOIN symbols s ON v.id = s.id
            ORDER BY distance
            LIMIT ?
        "#).query_map([&query_embedding, limit], SemanticMatch::from_row)
        .collect()
    }

    /// Route to DuckDB: codebase statistics
    pub fn stats(&self) -> CodebaseStats {
        // DuckDB: aggregations
        self.duckdb.query_row(r#"
            SELECT 
                (SELECT COUNT(*) FROM code.files) as files,
                (SELECT COUNT(*) FROM code.symbols) as symbols,
                (SELECT COUNT(*) FROM code.edges) as edges,
                (SELECT SUM(functions) FROM file_stats) as functions,
                (SELECT SUM(structs) FROM file_stats) as structs
        "#, [], CodebaseStats::from_row)
    }

    /// Hybrid: find and explain
    pub fn explain_symbol(&self, name: &str) -> SymbolExplanation {
        // SQLite: get the symbol
        let symbol = self.find_symbol(name)?;
        
        // DuckDB: get its relationships
        let callers = self.callers(&symbol.id);
        let dependencies = self.dependencies(&symbol.id);
        
        // Vectors: find related code
        let related = self.semantic_search(&symbol.brief.unwrap_or(name), 5);
        
        SymbolExplanation { symbol, callers, dependencies, related }
    }
}
```

## Vector Search Deep Dive

### What Gets Embedded?

```rust
struct EmbeddingInput {
    /// Combination of name + signature + docstring + context
    text: String,
}

impl Symbol {
    fn to_embedding_text(&self) -> String {
        let mut parts = vec![
            self.name.clone(),
            self.kind.clone(),
        ];
        
        if let Some(sig) = &self.signature {
            parts.push(sig.clone());
        }
        
        if let Some(brief) = &self.brief {
            parts.push(brief.clone());
        }
        
        // Add semantic hints based on kind
        match self.kind.as_str() {
            "function" => parts.push("function method procedure".into()),
            "struct" => parts.push("struct type data structure".into()),
            "enum" => parts.push("enum enumeration variant".into()),
            "trait" => parts.push("trait interface contract".into()),
            _ => {}
        }
        
        parts.join(" ")
    }
}
```

### Embedding Strategy

```
Option 1: Local embeddings (fast, private)
├── all-MiniLM-L6-v2 (384 dimensions)
├── bge-small-en (384 dimensions)
└── nomic-embed-text (768 dimensions)

Option 2: API embeddings (higher quality)
├── OpenAI text-embedding-3-small (1536 dimensions)
├── Voyage code-2 (1536 dimensions, code-optimized)
└── Cohere embed-v3 (1024 dimensions)

Recommendation: Start with local for speed, 
upgrade to code-specific API model for production
```

### Semantic Query Examples

```sql
-- "Find functions that handle authentication"
-- Agent generates embedding for the query, then:
SELECT 
    s.name, s.file_path, s.signature, s.brief,
    vec_distance_cosine(v.embedding, :query_embedding) as relevance
FROM symbol_vectors v
JOIN symbols s ON v.id = s.id
WHERE s.kind = 'function'
ORDER BY relevance
LIMIT 10;

-- Results might return:
-- 1. verify_token (src/auth.rs) - "Verify JWT token validity"
-- 2. check_permissions (src/auth.rs) - "Check user permissions"  
-- 3. hash_password (src/crypto.rs) - "Hash password with bcrypt"
-- 4. validate_session (src/session.rs) - "Validate session token"
```

### Hybrid Search: Keywords + Vectors

```rust
/// Combine exact matches with semantic similarity
pub fn hybrid_search(&self, query: &str, limit: i32) -> Vec<SearchResult> {
    // 1. Exact/fuzzy name matches (SQLite)
    let exact_matches: Vec<_> = self.sqlite.prepare(r#"
        SELECT id, name, kind, file_path, signature, brief, 1.0 as score
        FROM symbols
        WHERE name LIKE ? OR name LIKE ?
        ORDER BY CASE WHEN name = ? THEN 0 ELSE 1 END
        LIMIT ?
    "#).query_map([
        format!("%{}%", query), 
        format!("%{}%", query.to_lowercase()),
        query,
        limit / 2
    ]).collect();

    // 2. Semantic matches (Vectors)
    let semantic_matches = self.semantic_search(query, limit / 2);

    // 3. Merge and deduplicate, preserving best scores
    let mut results: HashMap<String, SearchResult> = HashMap::new();
    
    for m in exact_matches {
        results.insert(m.id.clone(), SearchResult {
            symbol: m,
            match_type: MatchType::Exact,
            score: 1.0,
        });
    }
    
    for m in semantic_matches {
        results.entry(m.id.clone())
            .and_modify(|r| r.score = r.score.max(m.relevance))
            .or_insert(SearchResult {
                symbol: m,
                match_type: MatchType::Semantic,
                score: m.relevance,
            });
    }

    // Sort by score
    let mut results: Vec<_> = results.into_values().collect();
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    results.truncate(limit);
    results
}
```

## Incremental Updates

### File Watcher Integration

```rust
pub struct IncrementalIndexer {
    sqlite: Connection,
    parser: CodeParser,
    embedder: Embedder,
}

impl IncrementalIndexer {
    /// Check if file needs reindexing
    pub fn needs_update(&self, path: &Path) -> bool {
        let content = fs::read(path).ok();
        let new_hash = content.map(|c| hash(&c));
        
        let stored_hash: Option<String> = self.sqlite.query_row(
            "SELECT content_hash FROM files WHERE path = ?",
            [path.to_str()],
            |row| row.get(0)
        ).ok();
        
        new_hash != stored_hash
    }

    /// Update single file (fast, transactional)
    pub fn update_file(&mut self, path: &Path) -> Result<()> {
        let source = fs::read_to_string(path)?;
        let hash = hash(&source);
        
        // Parse AST
        let parsed = self.parser.parse(path, &source)?;
        
        // Begin transaction
        let tx = self.sqlite.transaction()?;
        
        // Clear old data for this file
        tx.execute("DELETE FROM symbols WHERE file_path = ?", [path.to_str()])?;
        tx.execute("DELETE FROM edges WHERE source_id LIKE ?", [format!("{}::%", path.display())])?;
        
        // Insert new symbols
        for symbol in &parsed.symbols {
            tx.execute(
                "INSERT INTO symbols VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                symbol.to_params()
            )?;
            
            // Generate and store embedding
            let embedding = self.embedder.embed(&symbol.to_embedding_text())?;
            tx.execute(
                "INSERT INTO symbol_vectors (id, embedding) VALUES (?, ?)",
                [&symbol.id, &embedding]
            )?;
        }
        
        // Insert edges
        for edge in &parsed.edges {
            tx.execute(
                "INSERT INTO edges VALUES (NULL, ?, ?, ?, ?, ?, ?, ?)",
                edge.to_params()
            )?;
        }
        
        // Update file record
        tx.execute(
            "INSERT OR REPLACE INTO files VALUES (?, ?, ?, ?, unixepoch(), ?)",
            [path.to_str(), &hash, &source.len().to_string(), &parsed.language, &source]
        )?;
        
        tx.commit()?;
        Ok(())
    }

    /// Refresh DuckDB materialized views after updates
    pub fn refresh_analytics(&self, duckdb: &Connection) -> Result<()> {
        duckdb.execute_batch(r#"
            DROP TABLE IF EXISTS call_graph;
            CREATE TABLE call_graph AS
            SELECT ... FROM code.edges e JOIN code.symbols s ...;
            
            DROP TABLE IF EXISTS file_stats;
            CREATE TABLE file_stats AS
            SELECT ... FROM code.symbols GROUP BY file_path;
        "#)?;
        Ok(())
    }
}
```

### Watch Mode

```rust
pub fn watch_and_index(root: &Path, indexer: &mut IncrementalIndexer) {
    let (tx, rx) = channel();
    
    let mut watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            tx.send(event).ok();
        }
    }).unwrap();
    
    watcher.watch(root, RecursiveMode::Recursive).unwrap();
    
    println!("👀 Watching for changes...");
    
    let mut pending_refresh = false;
    
    for event in rx {
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                for path in event.paths {
                    if is_source_file(&path) && indexer.needs_update(&path) {
                        println!("📝 Indexing: {}", path.display());
                        if let Err(e) = indexer.update_file(&path) {
                            eprintln!("⚠️  Error: {}", e);
                        }
                        pending_refresh = true;
                    }
                }
            }
            EventKind::Remove(_) => {
                for path in event.paths {
                    println!("🗑️  Removing: {}", path.display());
                    indexer.remove_file(&path).ok();
                    pending_refresh = true;
                }
            }
            _ => {}
        }
        
        // Batch refresh analytics (debounced)
        if pending_refresh {
            indexer.refresh_analytics(&duckdb).ok();
            pending_refresh = false;
        }
    }
}
```

## CLI Commands

```bash
# Index with all engines
ctx index [--watch]
# Creates:
#   .ctx/codebase.sqlite (source of truth + vectors)
#   .ctx/analytics.duckdb (materialized views)

# Structural queries (routed to appropriate engine)
ctx query find <pattern>              # SQLite: fuzzy name match
ctx query callers <function>          # DuckDB: reverse call graph  
ctx query deps <symbol>               # DuckDB: forward dependencies
ctx query graph <start> [--depth=5]   # DuckDB: recursive traversal
ctx query impact <symbol>             # DuckDB: change impact analysis
ctx query stats                       # DuckDB: codebase statistics

# Semantic queries (routed to vectors)
ctx search "handle authentication"    # Vector: semantic search
ctx search "error handling patterns"  # Vector: find similar code
ctx similar <symbol>                  # Vector: symbols like this one

# Hybrid queries
ctx explain <symbol>                  # All engines: comprehensive view

# Source retrieval (SQLite)
ctx source <symbol>                   # Get full source code
ctx context <file>                    # Get file with dependencies
```

## API Response Examples

### Structural Query: Call Graph

```bash
$ ctx query graph run --depth=3
```

```json
{
  "root": "main::run",
  "nodes": [
    {"name": "run", "file": "src/main.rs", "kind": "function"},
    {"name": "discover_files", "file": "src/walker.rs", "kind": "function"},
    {"name": "generate_context", "file": "src/output.rs", "kind": "function"},
    {"name": "is_binary_file", "file": "src/walker.rs", "kind": "function"},
    {"name": "get_formatter", "file": "src/formatter.rs", "kind": "function"}
  ],
  "edges": [
    {"from": "run", "to": "discover_files", "kind": "calls"},
    {"from": "run", "to": "generate_context", "kind": "calls"},
    {"from": "discover_files", "to": "is_binary_file", "kind": "calls"},
    {"from": "generate_context", "to": "get_formatter", "kind": "calls"}
  ]
}
```

### Semantic Query: Natural Language Search

```bash
$ ctx search "format file sizes for display"
```

```json
{
  "query": "format file sizes for display",
  "results": [
    {
      "name": "format_size",
      "file": "src/walker.rs",
      "signature": "fn format_size(size: u64) -> String",
      "brief": "Format file size in human-readable format",
      "relevance": 0.92,
      "match_type": "semantic"
    },
    {
      "name": "render_tree",
      "file": "src/tree.rs",
      "signature": "fn render_tree(node: &TreeNode, ...) -> String",
      "brief": "Render the tree to an ASCII string with optional sizes",
      "relevance": 0.78,
      "match_type": "semantic"
    }
  ]
}
```

### Hybrid Query: Explain Symbol

```bash
$ ctx explain FileEntry
```

```json
{
  "symbol": {
    "name": "FileEntry",
    "kind": "struct",
    "file": "src/walker.rs",
    "visibility": "public",
    "signature": "struct FileEntry { absolute_path: PathBuf, relative_path: PathBuf, size: u64 }",
    "brief": "Represents a discovered file with its metadata"
  },
  "callers": [
    {"name": "discover_files", "file": "src/walker.rs", "context": "entries.push(FileEntry { ... })"},
    {"name": "format_file", "file": "src/formatter.rs", "context": "fn format_file(&self, entry: &FileEntry, ...)"}
  ],
  "dependencies": [
    {"name": "PathBuf", "kind": "type", "from": "std::path"}
  ],
  "related": [
    {"name": "WalkerConfig", "relevance": 0.85, "reason": "Also used in file discovery"},
    {"name": "ContextResult", "relevance": 0.72, "reason": "Similar metadata structure"}
  ]
}
```

## Performance Characteristics

| Operation | Engine | Latency | Notes |
|-----------|--------|---------|-------|
| Get symbol by ID | SQLite | <1ms | Direct B-tree lookup |
| Get source code | SQLite | <5ms | BLOB retrieval |
| Update single file | SQLite | 10-50ms | Transaction + inserts |
| Find by name pattern | SQLite | 5-20ms | Index scan |
| Call graph (depth 5) | DuckDB | 20-100ms | Recursive CTE |
| Impact analysis | DuckDB | 30-150ms | Reverse traversal |
| Codebase stats | DuckDB | 5-20ms | Pre-aggregated |
| Semantic search | Vectors | 10-50ms | Depends on index size |
| Hybrid search | All | 30-100ms | Parallel execution |

## Storage Footprint

For a 50-file, 10K LOC codebase:

| Component | Size |
|-----------|------|
| SQLite (symbols, edges, source) | ~5 MB |
| SQLite (vectors, 1536-dim) | ~15 MB |
| DuckDB (materialized views) | ~2 MB |
| **Total** | **~22 MB** |

Compare to loading full source into context: **~40K tokens × $0.01 = $0.40 per query**

## Implementation Checklist

```
Phase 1: SQLite Foundation
├── [ ] Schema creation
├── [ ] File tracking with hashes
├── [ ] Symbol extraction (tree-sitter)
├── [ ] Edge extraction (calls, imports, types)
├── [ ] Basic queries (find, get_source)
└── [ ] Incremental updates

Phase 2: DuckDB Analytics  
├── [ ] Attach SQLite database
├── [ ] Materialized call graph view
├── [ ] Recursive traversal queries
├── [ ] Impact analysis
├── [ ] Module dependency graph
└── [ ] Statistics aggregations

Phase 3: Vector Search
├── [ ] sqlite-vec integration
├── [ ] Embedding generation (local or API)
├── [ ] Semantic search queries
├── [ ] Hybrid search (exact + semantic)
└── [ ] Similar symbol discovery

Phase 4: CLI & Watch Mode
├── [ ] index command
├── [ ] query subcommands
├── [ ] search command
├── [ ] watch mode with debouncing
└── [ ] Output formatting (table, JSON)

Phase 5: Agent Integration
├── [ ] MCP server wrapper
├── [ ] Streaming responses
├── [ ] Context budget management
└── [ ] Query explanation/reasoning
```

## Future: MCP Server

Expose as a Model Context Protocol server for direct agent integration:

```json
{
  "mcpServers": {
    "codebase": {
      "command": "ctx",
      "args": ["serve", "--db", ".ctx/"],
      "tools": [
        "find_symbol",
        "get_source", 
        "call_graph",
        "impact_analysis",
        "semantic_search",
        "explain_symbol"
      ]
    }
  }
}
```

Then agents can directly invoke:

```
Agent: I need to add retry logic to API calls. Let me find where API calls happen.

Tool call: semantic_search("API calls http requests fetch")
Result: [fetch_data (src/api.rs), post_json (src/api.rs), ...]

Tool call: impact_analysis("fetch_data")  
Result: [get_user, sync_records, refresh_token] would be affected

Tool call: get_source("fetch_data")
Result: [20 lines of actual code]

Agent: Now I understand the structure. I'll modify fetch_data to add retry logic...
```
