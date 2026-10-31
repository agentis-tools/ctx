---
id: architecture
title: Architecture
sidebar_position: 7
---

# Architecture

This page describes how ctx works internally: the modules, the storage layout, and how indexing, search, and analytics fit together.

## Overview

ctx has two parts:

1. Context generation walks files, applies ignore rules, and formats the output.
2. Code intelligence parses code, extracts symbols and relationships, and stores them in a queryable index.

```
┌──────────────────────────────────────────────────────────────────────────┐
│                                ctx CLI                                   │
├──────────────────────────────┬───────────────────────────────────────────┤
│  Context generation          │  Code intelligence                        │
│                              │                                           │
│  File walker (ignore crate)  │  Parsers: tree-sitter (Rust, TS/JS,       │
│          │                   │  Python, Go, Zig, C, C++), solang-parser  │
│  Ignore rules (.gitignore,   │  (Solidity), optional LSP servers         │
│  .contextignore, built-in)   │          │                                │
│          │                   │  SQLite index (.ctx/codebase.sqlite)      │
│  Formatters (XML, Markdown,  │  symbols, edges, FTS5, embeddings,        │
│  plain, JSON)                │  fingerprints, compressed source          │
│          │                   │          │                                │
│  Streamed output             │  DuckDB analytics (default feature):      │
│                              │  graph traversal, sql, snapshots          │
├──────────────────────────────┴───────────────────────────────────────────┤
│  Embeddings: local fastembed, OpenAI, or Ollama                          │
└──────────────────────────────────────────────────────────────────────────┘
```

## Module structure

```
src/
├── main.rs            # CLI entry point, command routing
├── cli.rs             # Argument parsing (clap)
├── commands/          # One module per command (index, smart, sql, judge, lsp, ...)
├── walker.rs          # File discovery (ignore crate)
├── default_ignores.rs # Built-in ignore patterns (164)
├── formatter.rs       # Output formatters
├── output.rs          # Context generation
├── tree.rs            # Project tree rendering
├── tokens.rs          # Token counting
├── parser/            # Rust, TypeScript/JavaScript, Python, Go, Zig, C/C++, Solidity
├── lsp/               # Optional LSP extraction backend ([lsp.<language>])
├── db/                # SQLite schema, models, queries
├── index/             # Indexing, edge resolution, watch mode
├── analytics/         # DuckDB queries (stub when built without the duckdb feature)
├── embeddings/        # Provider trait; local, OpenAI and Ollama providers
├── smart.rs, rank.rs  # Smart context selection; PageRank for ctx map
├── fingerprint.rs     # MinHash fingerprints for ctx duplicates
├── check.rs, rules.rs # Architecture rules (ctx check)
├── score.rs           # Scorecard and gates (ctx score)
├── snapshot.rs        # Parquet snapshots (ctx snapshot)
├── judge.rs           # Model-assisted edge resolution (ctx judge edges)
├── harness/           # Claude Code and Codex hook generation
├── mcp/               # MCP server (mcp feature)
└── update.rs          # ctx self-update
```

## Storage: one SQLite file

The index lives in `.ctx/codebase.sqlite`. The schema version is 3; an index with another version reports a mismatch and is rebuilt with `ctx index --force`. The core tables:

```sql
-- Files: what has been indexed
CREATE TABLE files (
    path TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,      -- SHA-256, used for change detection
    size_bytes INTEGER,
    language TEXT,
    last_indexed INTEGER,
    source BLOB                      -- gzip-compressed source
);

-- Symbols: functions, methods, types, ...
CREATE TABLE symbols (
    id TEXT PRIMARY KEY,             -- see "Symbol IDs" below
    file_path TEXT NOT NULL,
    name TEXT NOT NULL,
    qualified_name TEXT,
    kind TEXT NOT NULL,              -- function, method, struct, enum, ...
    visibility TEXT DEFAULT 'private',
    signature TEXT,
    brief TEXT,                      -- first line of the doc comment
    docstring TEXT,
    line_start INTEGER,
    line_end INTEGER,
    col_start INTEGER,
    col_end INTEGER,
    parent_id TEXT,
    source TEXT,                     -- symbol source code
    FOREIGN KEY (file_path) REFERENCES files(path) ON DELETE CASCADE
);

-- Edges: relationships between symbols
CREATE TABLE edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id TEXT NOT NULL,         -- calling or referring symbol
    target_id TEXT,                  -- resolved target symbol, NULL if unresolved
    target_name TEXT NOT NULL,       -- name as written at the site
    kind TEXT NOT NULL,              -- calls, imports, uses, extends, implements, ...
    line INTEGER,
    col INTEGER,
    context TEXT,                    -- source line of the site
    FOREIGN KEY (source_id) REFERENCES symbols(id) ON DELETE CASCADE
);

-- Modules: file-level imports and exports
CREATE TABLE modules (
    file_path TEXT PRIMARY KEY,
    module_name TEXT,
    exports TEXT,                    -- JSON array
    imports TEXT,                    -- JSON array
    FOREIGN KEY (file_path) REFERENCES files(path) ON DELETE CASCADE
);

-- Embeddings for semantic search
CREATE TABLE embeddings (
    symbol_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,          -- local, openai or ollama
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL,
    vector TEXT NOT NULL,            -- JSON array of floats
    created_at INTEGER,
    FOREIGN KEY (symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
);
```

Other tables:

- `symbol_fts` is an FTS5 table over symbol name, kind, signature and docs, kept in sync by triggers. Keyword search uses it.
- `symbol_fingerprints` stores MinHash signatures for `ctx duplicates` and `ctx score`.
- `symbol_rank` caches PageRank scores for `ctx map`. Indexing clears it; the next `ctx map` recomputes it.
- `symbol_vectors` is a sqlite-vec `vec0` table for 1536-dimension vectors (see [Similarity search](#similarity-search)).
- `judgments` caches `ctx judge edges` answers, keyed by a SHA-256 hash of the question, the state shown to the model, and the model. It does not reference symbols, so re-indexing keeps the answers.

Storing a file is all-or-nothing: each file is written inside one SAVEPOINT, so a failure partway leaves no partial file record.

SQLite fits because the index is one file you can delete, copy or inspect with any SQLite tool, it needs no server, and FTS5 gives BM25 keyword ranking.

## Analytics: DuckDB

Commands that need analytical queries (`ctx sql`, `ctx query`, `ctx snapshot`, `ctx smart`, `ctx diff`, and parts of `ctx audit`) use an in-memory DuckDB connection over the SQLite index. DuckDB is part of the default build (`duckdb` feature). A build with `--no-default-features` (used for Windows MSVC, where DuckDB doesn't compile) replaces it with a stub that returns empty analytics results; `ctx sql` and `ctx snapshot` exit 2 there.

DuckDB reads SQLite through its `sqlite_scanner` extension, and ctx never downloads it:

1. If the extension is installed locally, ctx attaches the index read-only.
2. Otherwise ctx copies the tables into a private temporary DuckDB file and attaches that read-only as `code`.

Both paths work offline and behind egress allowlists. Graph traversal uses recursive CTEs over `code.edges`, for example:

```sql
WITH RECURSIVE graph AS (
    SELECT id, name, file_path, kind, 1 AS depth
    FROM code.symbols WHERE name = ?
    UNION ALL
    SELECT t.id, t.name, t.file_path, t.kind, g.depth + 1
    FROM graph g
    JOIN code.edges e ON e.source_id = g.id
    JOIN code.symbols t ON e.target_id = t.id
    WHERE g.depth < ?
)
SELECT DISTINCT name, file_path, kind, depth FROM graph;
```

## Parsing

ctx parses source with tree-sitter grammars, except Solidity, which uses `solang-parser`. Each parser extracts symbols and edges in one pass over the syntax tree. Cold indexing reuses one parser per worker thread instead of building one per file.

| Language | Crate |
|----------|-------|
| Rust | `tree-sitter-rust` |
| TypeScript | `tree-sitter-typescript` |
| JavaScript | `tree-sitter-javascript` |
| Python | `tree-sitter-python` |
| Go | `tree-sitter-go` |
| Zig | `tree-sitter-zig` |
| C | `tree-sitter-c` |
| C++ | `tree-sitter-cpp` |
| Solidity | `solang-parser` |

YAML files are detected but not parsed. Any language can also be indexed through a language server configured under `[lsp.<language>]`; see [Add a language via LSP](lsp-languages.md).

tree-sitter parses real syntax trees, tolerates incomplete code, and exposes the same API for every grammar.

## File walking

The `ignore` crate discovers files. ctx applies three layers of exclusions:

1. 164 built-in patterns for non-source files (`--no-default-ignores` disables them)
2. `.gitignore` (`--no-gitignore` disables it)
3. `.contextignore` for project-specific exclusions

Binary files are skipped.

## Incremental indexing

ctx stores a SHA-256 hash of each file's content. On the next run it re-parses only files whose hash changed and removes records for deleted files. Content hashes are more reliable than modification times. A no-op refresh also skips the unresolved-edge scan.

ctx also stores each file's source gzip-compressed in `files.source`, and each symbol's source in `symbols.source`.

## Symbol IDs

A symbol ID combines the file path, the parent (if any), the name, and usually the start line:

```
file_path::name@line
file_path::Parent::name@line
```

Examples:

```
src/auth/handler.ts::handleAuth@45
src/server.go::Server::Start
```

Go methods include their receiver type (`A.String` and `B.String` are different symbols, with qualified names `A.String` and `B.String`). When two symbols share a name and line, such as a C struct and its typedef, the ID gets a `#kind` suffix.

## Call graph extraction

Parsers record call sites as edges with the name as written (`target_name`), the line and column, and the source line as context. After parsing, the resolver binds `target_id` when it can identify the callee:

- In order: an exact qualified-name match, a call context that names the type (`TypeScriptParser::new()`), a name defined once in the codebase, or a name defined once in the calling file.
- C and C++ calls to a function declared in a header and defined in one `.c`/`.cc` file bind to the definition.
- Rust method-call syntax (`x.find(..)`) is never bound to a free function with the same name.
- Rust free functions passed as values (`spawn(worker::run_main)`) become `uses` edges, not calls, when exactly one function matches.

Edges that stay unresolved keep `target_id` empty. Two optional steps can bind more of them:

- LSP hybrid mode asks a language server for definitions (`textDocument/definition`) after it reports that loading has finished.
- `ctx judge edges` sends ambiguous calls to a decision model, which picks a same-named candidate or `external`. Answers are cached in `judgments` and re-applied offline on later runs. See [ctx judge](commands/judge.md).

## Embeddings

### Providers

| Provider | Model | Dimension |
|----------|-------|-----------|
| `local` (default) | fastembed all-MiniLM-L6-v2, downloaded on first run | 384 |
| `openai` | `text-embedding-3-small` (needs `OPENAI_API_KEY`) | 1536 |
| `ollama` | `nomic-embed-text` by default (`OLLAMA_EMBED_MODEL`) | probed from the model |

Each embedding row records its provider, model and dimension. Semantic commands refuse to rank a query against a corpus from another provider, model or dimension; rebuild it with `ctx embed --force --provider <provider>`.

### Similarity search

The `symbol_vectors` table is a sqlite-vec `vec0` index for 1536-dimension vectors. When vectors of that size are indexed, semantic search runs a KNN query inside SQLite:

```sql
SELECT symbol_id, distance
FROM symbol_vectors
WHERE embedding MATCH ?
ORDER BY distance
LIMIT ?
```

ctx converts the L2 distance to a score with `1 / (1 + distance)`. For other dimensions (the default 384-dimension local model, most Ollama models), or when the extension can't load, ctx loads the stored vectors and ranks them by cosine similarity in Rust.

## Output streaming

Context generation writes the tree first, then each file as soon as it is formatted, and flushes stdout after each file. `--no-stream` buffers the whole output instead.

## Performance

### Latency budgets

The perf harness ([`perf/README.md`](https://github.com/agentis-tools/ctx/blob/main/perf/README.md)) enforces these budgets. CI runs it as an advisory `perf` job on every pull request. Each scenario runs a prebuilt `ctx` binary against a deterministic synthetic fixture and takes the median of 5 timed runs:

| Operation | Fixture | Budget |
|-----------|---------|--------|
| `ctx index`, incremental (1 changed file) | 2,000 files | 300 ms |
| `ctx score --against HEAD` (3 changed files) | 2,000 files | 2 s |
| `ctx check --against HEAD` (3 changed files) | 2,000 files | 1 s |
| `ctx map --budget 2000` (warm rank cache) | 2,000 files | 500 ms |
| `ctx sql` (aggregate query over `v1`) | 2,000 files | 500 ms |
| `ctx index`, cold (full suite only) | ~150k LOC | 60 s |

Peak memory (`ru_maxrss`) across the incremental-index, score, check, and map runs must stay below 300 MB. CI multiplies budgets by 1.5 (`CTX_PERF_BUDGET_SCALE`) because hosted runners are slower and noisier. Medians must also stay within 1.20x of a committed per-runner-class baseline (`perf/baselines/`).

## Data flow

### Context generation

```
patterns → walker → ignore rules → binary detection → formatter → streamed output
```

### Indexing

```
walker → hash check → parser → symbols and edges → SQLite (one SAVEPOINT per file)
       → edge resolution → optional LSP / judge → FTS triggers update
```

### Queries

```
keyword search   → SQLite FTS5
semantic search  → sqlite-vec KNN, or cosine similarity in Rust
graph queries    → DuckDB recursive CTEs over the attached index
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` | CLI argument parsing |
| `ignore`, `globset` | File walking and glob matching |
| `rusqlite` (bundled SQLite), `sqlite-vec` | Index storage and vector search |
| `duckdb` | Analytical queries (default feature) |
| `tree-sitter`, `tree-sitter-*` | Parsing |
| `solang-parser` | Solidity parsing |
| `flate2`, `sha2` | Source compression and content hashing |
| `fastembed` | Local embeddings |
| `reqwest` | HTTP for OpenAI, Ollama, judge and self-update |
| `notify` | Watch mode |
| `rmcp` | MCP server (`mcp` feature) |
