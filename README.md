# ctx

A fast CLI tool that generates AI-ready context from your codebase, with built-in code intelligence for understanding symbol relationships.

## Two Tools in One

**Context Generation** - Select files using glob patterns and get formatted output perfect for LLMs:
```bash
ctx src/ | pbcopy  # Copy source files to clipboard
```

**Code Intelligence** - Build a searchable index of your codebase with call graphs and impact analysis:
```bash
ctx index          # Index your codebase
ctx search "auth"  # Find symbols
ctx query callers handleLogin  # Who calls this function?
```

## Features

### Context Generation
- **Glob pattern support** - Select files with patterns like `"src/**/*.rs"` or `"**/*.ts"`
- **Smart ignore system** - Respects `.gitignore` and `.contextignore`
- **Built-in filtering** - Excludes binary files, `node_modules`, build artifacts, and 170+ patterns
- **Multiple output formats** - XML (default), Markdown, or plain text
- **Project tree visualization** - ASCII tree showing file structure
- **Streaming output** - Files output as processed, pipeable to clipboard

### Code Intelligence
- **Multi-language parsing** - Rust, TypeScript, JavaScript, JSX/TSX, Python, Solidity, YAML
- **Symbol extraction** - Functions, classes, interfaces, structs, enums, traits
- **Rich relationship tracking** - Calls, extends, implements, and imports edges
- **Call graph analysis** - Track function calls and dependencies
- **Impact analysis** - See what would be affected by changing a symbol
- **Keyword search** - FTS5-powered search across symbols and documentation
- **Semantic search** - Embedding-based natural language search (OpenAI)
- **Watch mode** - Automatic reindexing on file changes

## Installation

```bash
cargo install --path .
```

Or build from source:
```bash
cargo build --release
# Binary at ./target/release/ctx
```

## Quick Start

### Generate Context for LLMs

```bash
# All files in current directory
ctx

# Specific patterns
ctx "src/**/*.rs" "**/*.ts"

# Copy to clipboard (macOS)
ctx src/ | pbcopy

# Markdown format
ctx --format markdown src/
```

### Code Intelligence

```bash
# Build the index (creates .ctx/codebase.sqlite)
ctx index

# Search for symbols (keyword matching)
ctx search "handleRequest"

# Semantic search (natural language, requires OpenAI API key)
export OPENAI_API_KEY=sk-...
ctx embed                          # Generate embeddings once
ctx semantic "authentication logic" # Search by meaning

# Find all callers of a function
ctx query callers authenticate

# See what a function depends on
ctx query deps processPayment

# Visualize call graph
ctx query graph main --depth 3

# Impact analysis - what breaks if I change this?
ctx query impact validateInput

# Watch for changes and auto-reindex
ctx index --watch
```

## Output Formats

### XML (default)
```xml
<context>
<project_tree>
my-project/
├── src/
│   ├── main.rs
│   └── lib.rs
└── Cargo.toml
</project_tree>
<project_files>
<file name="main.rs" path="/src/main.rs">
fn main() {
    println!("Hello, world!");
}
</file>
</project_files>
</context>
```

### Markdown
```markdown
# Project Context

## Project Tree
\`\`\`
my-project/
├── src/
│   └── main.rs
└── Cargo.toml
\`\`\`

## /src/main.rs
\`\`\`rs
fn main() {
    println!("Hello, world!");
}
\`\`\`
```

## Code Intelligence Commands

### `ctx index`
Build or update the code intelligence database.

```bash
ctx index              # Incremental index
ctx index --force      # Full reindex (clears database)
ctx index --watch      # Watch mode with auto-reindex
ctx index --verbose    # Show files being indexed
```

### `ctx search <query>`
Search for symbols using semantic matching.

```bash
ctx search "auth"              # Find symbols related to auth
ctx search "parse config"      # Natural language search
ctx search "handleRequest" --limit 10
```

### `ctx query`
Query the code intelligence database.

```bash
# Find symbols by name pattern
ctx query find "handle*" --kind function

# Show callers of a function
ctx query callers myFunction --depth 3

# Show dependencies of a symbol
ctx query deps MyClass

# Visualize call graph (text, json, or dot format)
ctx query graph entryPoint --depth 5 --output dot

# Impact analysis
ctx query impact criticalFunction --depth 5

# Codebase statistics
ctx query stats

# List all indexed files
ctx query files
```

### `ctx explain <symbol>`
Get detailed information about a symbol including its relationships.

```bash
ctx explain handleAuth
```

### `ctx source <symbol>`
Retrieve the source code for a symbol.

```bash
ctx source MyClass::processData
```

## Ignore System

Three-tier ignore system:

1. **`.gitignore`** - Respected by default (disable with `--no-gitignore`)
2. **`.contextignore`** - Project-specific ignores, same syntax as `.gitignore`
3. **Built-in patterns** - Common non-source files (disable with `--no-default-ignores`)

### Example `.contextignore`
```gitignore
# Exclude test fixtures
fixtures/
__mocks__/

# Exclude generated code
*.generated.ts
*.pb.go

# Exclude vendored dependencies
vendor/
third_party/
```

## Supported Languages

| Language | Extensions | Symbol Extraction | Edge Types |
|----------|-----------|-------------------|------------|
| Rust | `.rs` | Functions, structs, enums, traits, impls | Calls, Implements, Imports |
| TypeScript | `.ts` | Functions, classes, interfaces, types, enums | Calls, Extends, Implements, Imports |
| TSX | `.tsx` | Functions, components, interfaces | Calls, Extends, Implements, Imports |
| JavaScript | `.js`, `.mjs`, `.cjs` | Functions, classes, arrow functions | Calls, Extends, Imports |
| JSX | `.jsx` | Functions, components | Calls, Extends, Imports |
| Python | `.py`, `.pyi` | Functions, classes, methods, constants | Calls, Extends, Imports |
| Solidity | `.sol` | Contracts, functions, events, structs | Calls |
| YAML | `.yaml`, `.yml` | File tracking (no symbols) | N/A |

## Architecture

```
.ctx/
└── codebase.sqlite    # SQLite database with FTS5 search and embeddings
```

- **SQLite** - Persistent storage for symbols, edges, embeddings, and compressed source
- **DuckDB** - In-memory analytical engine for recursive graph queries
- **Tree-sitter** - Fast, accurate parsing for all supported languages
- **OpenAI** - Optional embedding generation for semantic search

## CLI Reference

```
ctx - Generate AI-ready context from your codebase

USAGE:
    ctx [OPTIONS] [PATTERNS]...
    ctx <COMMAND>

COMMANDS:
    index     Build or update the code intelligence index
    query     Query the code intelligence database
    search    Search for symbols using keyword matching
    semantic  Search using embeddings (natural language)
    embed     Generate embeddings for semantic search
    source    Get the source code for a symbol
    explain   Explain a symbol with its relationships

CONTEXT OPTIONS:
    -f, --format <FORMAT>    Output format [default: xml] [values: xml, markdown, md, plain]
        --no-gitignore       Disable .gitignore pattern matching
    -i, --ignore <PATTERN>   Additional ignore patterns
        --no-default-ignores Disable built-in ignore patterns
        --show-sizes         Show file sizes in project tree
        --no-tree            Disable project tree in output
        --no-stream          Buffer output instead of streaming
        --stats              Print stats after completion

INDEX OPTIONS:
    -w, --watch    Watch for changes and reindex automatically
    -v, --verbose  Show verbose output
    -f, --force    Force full reindex (clears existing database)
```

## Performance

- Indexes ~2000 files in under 10 seconds
- Incremental updates only reindex changed files
- Compressed source storage (~70% size reduction)
- In-memory DuckDB for fast analytical queries

