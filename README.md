# ctx

[![Crates.io](https://img.shields.io/crates/v/agentis-ctx)](https://crates.io/crates/agentis-ctx)
[![CI](https://github.com/saldestechnology/ctx/actions/workflows/ci.yml/badge.svg)](https://github.com/saldestechnology/ctx/actions)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](#license)
[![Rust Version](https://img.shields.io/badge/rust-1.91%2B-orange)](https://www.rust-lang.org)
[![Docs](https://img.shields.io/badge/docs-saldestechnology.github.io%2Fctx-blue)](https://saldestechnology.github.io/ctx/)

A fast CLI tool that generates AI-ready context from your codebase, with built-in code intelligence for understanding symbol relationships.

📖 **Documentation:** https://saldestechnology.github.io/ctx/

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
- **Multiple output formats** - XML (default), Markdown, JSON, or plain text
- **Project tree visualization** - ASCII tree showing file structure
- **Streaming output** - Files output as processed, pipeable to clipboard
- **Token counting** - Count tokens for LLM context window management

### Code Intelligence
- **Multi-language parsing** - Rust, TypeScript, JavaScript, JSX/TSX, Python, Go, Solidity, YAML
- **Symbol extraction** - Functions, classes, interfaces, structs, enums, traits
- **Rich relationship tracking** - Calls, extends, implements, and imports edges
- **Call graph analysis** - Track function calls and dependencies
- **Impact analysis** - See what would be affected by changing a symbol
- **Keyword search** - FTS5-powered search across symbols and documentation
- **Semantic search** - Embedding-based natural language search (local or OpenAI)
- **Watch mode** - Automatic reindexing on file changes

### Advanced Features
- **Smart context selection** - AI-powered file selection based on task description
- **Diff-aware context** - Generate context focused on git changes
- **PR review context** - GitHub integration for pull request analysis
- **Code quality audit** - Automated quality analysis with CI integration
- **Interactive shell** - REPL for codebase exploration
- **MCP server** - Claude Desktop integration via Model Context Protocol

## Feature Flags

- **`duckdb`** (enabled by default) — Enables DuckDB-powered analytics (call graphs, impact analysis, complexity analysis). Disable with `--no-default-features` on platforms where DuckDB cannot compile (e.g. Windows MSVC without C++ build tools).
- **`mcp`** — Enable Model Context Protocol server support for Claude Desktop integration.

## Installation

From crates.io (the package is `agentis-ctx`; it installs the `ctx` binary):

```bash
cargo install agentis-ctx

# On Windows (MSVC without C++ build tools), skip the DuckDB feature:
cargo install agentis-ctx --no-default-features
```

From a local checkout:

```bash
cargo install --path .
```

Or build from source:
```bash
cargo build --release
# Binary at ./target/release/ctx
```

### With MCP Support (for Claude Desktop)
```bash
cargo build --release --features mcp
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

# JSON format
ctx --format json src/

# Count tokens only
ctx --count-only src/

# Limit output to token budget
ctx --max-tokens 8000 src/
```

### Code Intelligence

```bash
# Build the index (creates .ctx/codebase.sqlite)
ctx index

# Search for symbols (keyword matching)
ctx search "handleRequest"

# Generate embeddings for semantic search
ctx embed                          # Local model (default, ~90MB download)
ctx embed --openai                 # OpenAI API (requires OPENAI_API_KEY)

# Semantic search (natural language)
ctx semantic "authentication logic"
ctx semantic "error handling" --openai

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
```
my-project/
├── src/
│   └── main.rs
└── Cargo.toml
```

## /src/main.rs
```rust
fn main() {
    println!("Hello, world!");
}
```
```

### JSON
```json
{
  "project_tree": "my-project/\n├── src/\n│   └── main.rs\n└── Cargo.toml",
  "files": [
    {
      "name": "main.rs",
      "path": "/src/main.rs",
      "content": "fn main() {\n    println!(\"Hello, world!\");\n}"
    }
  ]
}
```

## Code Intelligence Commands

### `ctx index`
Build or update the code intelligence database.

```bash
ctx index                    # Incremental index
ctx index --force            # Full reindex (clears database)
ctx index --watch            # Watch mode with auto-reindex
ctx index --verbose          # Show files being indexed
ctx index --parallel         # Use parallel parsing (faster on multi-core)
ctx index --no-gitignore     # Include gitignored files
ctx index -i "tests/"        # Additional ignore patterns
ctx index -p "src/**/*.rs"   # Only index specific patterns
```

### `ctx search <query>`
Search for symbols using keyword matching (FTS5).

```bash
ctx search "auth"                  # Find symbols related to auth
ctx search "handleRequest"         # Find exact symbol names
ctx search "parse config" --limit 10
ctx search "handler" --output json # JSON output
```

### `ctx semantic <query>`
Search using embeddings for natural language queries.

```bash
ctx semantic "authentication logic"     # Local embeddings (default)
ctx semantic "error handling" --openai  # OpenAI embeddings
ctx semantic "database queries" --limit 20
```

### `ctx embed`
Generate embeddings for semantic search.

```bash
ctx embed                    # Generate with local model
ctx embed --openai           # Generate with OpenAI API
ctx embed --force            # Re-embed all symbols
ctx embed --verbose          # Show progress
ctx embed --batch-size 100   # Custom batch size
ctx embed --watch            # Watch for index changes and auto-embed
```

### `ctx query`
Query the code intelligence database.

```bash
# Find symbols by name pattern
ctx query find "handle*" --kind function
ctx query find "User*" --file "src/models/*.rs"

# Show callers of a function
ctx query callers myFunction --depth 3

# Show dependencies of a symbol
ctx query deps MyClass --depth 2

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
ctx explain MyClass --file "src/models/*.rs"
ctx explain process --kind function
```

### `ctx source <symbol>`
Retrieve the source code for a symbol.

```bash
ctx source MyClass::processData
ctx source authenticate --file "src/auth/*.rs"
```

## Smart Context Selection

Intelligently select files relevant to a task using semantic search and call graph analysis:

```bash
ctx smart "add user authentication" --max-tokens 8000
ctx smart "fix login bug" --explain      # Show selection reasoning
ctx smart "refactor parser" --dry-run    # Preview without output
ctx smart "add caching" --openai         # Use OpenAI embeddings
ctx smart "update API" --depth 3         # Deeper call graph expansion
ctx smart "fix tests" --top 20           # More initial semantic matches
```

**Options:**
- `--max-tokens <N>` - Maximum tokens in output (default: 8000)
- `--depth <N>` - Call graph expansion depth (default: 2)
- `--top <N>` - Number of initial semantic matches (default: 10)
- `--explain` - Show selection reasoning for each file
- `--dry-run` - Preview selection without generating context
- `--openai` - Use OpenAI embeddings instead of local model

## Diff-Aware Context

Get context for changed files with automatic dependency expansion:

```bash
ctx diff                      # Changes since HEAD~1
ctx diff main                 # Changes vs main branch
ctx diff HEAD~3               # Changes in last 3 commits
ctx diff --staged             # Only staged changes
ctx diff --summary            # Include change summary
ctx diff --changes-only       # No context expansion
ctx diff --max-tokens 10000   # Custom token budget
ctx diff --depth 2            # Call graph context depth
```

## PR Review Context

Generate context for GitHub pull request review:

```bash
ctx review 123                      # PR #123 in current repo
ctx review 123 --repo owner/name    # Specify repository
ctx review 123 --include-comments   # Include PR comments
ctx review 123 --summary            # Include change summary
ctx review 123 --changes-only       # Only changed files
```

**Requirements:** GitHub CLI (`gh`) must be installed and authenticated.

## Code Quality Audit

Automated quality analysis with CI integration:

```bash
ctx audit                          # Full quality report
ctx audit --min-score 7.0          # Quality gate (exit 1 if below)
ctx audit --output json            # JSON output for CI
ctx audit --output markdown        # Markdown report
ctx audit --categories complexity,duplication  # Specific categories
ctx audit --incremental            # Only changed files (pre-commit)
```

**Categories:**
- `complexity` - Function complexity (fan-out/fan-in analysis)
- `duplication` - Potential code duplication
- `coverage` - Documentation coverage
- `modularity` - Module coupling analysis
- `naming` - Naming convention checks

## Complexity Analysis

Analyze code complexity and identify high fan-out functions:

```bash
ctx complexity                     # Default threshold (10)
ctx complexity --threshold 20      # Custom threshold
ctx complexity --warnings-only     # Only show issues
ctx complexity --output json       # JSON output
```

## Duplicate Detection

Detect duplicate or similar code blocks:

```bash
ctx duplicates                     # Default settings
ctx duplicates --similarity 90     # 90% similarity threshold
ctx duplicates --min-lines 10      # Minimum 10 lines
ctx duplicates --output json       # JSON output
```

## Dependency Graph

Generate dependency graph visualizations:

```bash
ctx graph                          # DOT format (default)
ctx graph --output mermaid         # Mermaid diagram
ctx graph --output json            # JSON format
ctx graph --by-file                # Group by file/module
ctx graph --filter "src/auth/*"    # Filter to specific files
ctx graph --depth 5                # Maximum depth
```

## Interactive Shell

REPL for codebase exploration:

```bash
ctx shell                   # Start shell
ctx shell --vi              # Vi editing mode
ctx shell --no-history      # Disable history
ctx shell --history ~/.my_ctx_history  # Custom history file
```

**Shell Commands:**
- `find <pattern>` - Find symbols by name
- `search <query>` - Hybrid search (text + semantic)
- `source <symbol>` - Show source code
- `explain <symbol>` - Explain symbol with relationships
- `callers <fn>` - Show function callers
- `callees <fn>` - Show function callees
- `impact <symbol>` - Impact analysis
- `complexity` - Show high-complexity functions
- `stats` - Codebase statistics
- `audit` - Run code quality audit
- `cd <path>` - Set file path context
- `pwd` - Show current context
- `clear` - Clear screen
- `help` - Show help
- `exit` - Exit shell

## MCP Server (Claude Desktop)

Expose ctx to AI assistants via Model Context Protocol:

```bash
# Build with MCP support
cargo build --release --features mcp

# Run MCP server
ctx serve --mcp
```

Configure Claude Desktop (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "ctx": {
      "command": "ctx",
      "args": ["serve", "--mcp"],
      "cwd": "/path/to/project"
    }
  }
}
```

**Available MCP Tools:**
- `search_symbols` - Search for symbols by name pattern
- `get_definition` - Get the source code for a symbol
- `find_references` - Find all references to a symbol
- `get_callers` - Get functions that call a given function
- `get_callees` - Get functions called by a given function
- `get_file` - Read a file's contents
- `get_file_tree` - List files in the project
- `smart_context` - Intelligently select files for a task

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

### Built-in Ignore Patterns
The tool automatically ignores:
- Version control (`.git/`, `.svn/`, `.hg/`)
- IDE directories (`.vscode/`, `.idea/`)
- Lock files (`package-lock.json`, `yarn.lock`, `Cargo.lock`)
- Dependencies (`node_modules/`, `vendor/`, `Pods/`)
- Build outputs (`dist/`, `build/`, `target/`, `.next/`)
- Cache directories (`.cache/`, `tmp/`)
- Binary files and media

## Supported Languages

| Language | Extensions | Symbol Extraction | Edge Types |
|----------|-----------|-------------------|------------|
| Rust | `.rs` | Functions, structs, enums, traits, impls | Calls, Implements, Imports |
| TypeScript | `.ts` | Functions, classes, interfaces, types, enums | Calls, Extends, Implements, Imports |
| TSX | `.tsx` | Functions, components, interfaces | Calls, Extends, Implements, Imports |
| JavaScript | `.js`, `.mjs`, `.cjs` | Functions, classes, arrow functions | Calls, Extends, Imports |
| JSX | `.jsx` | Functions, components | Calls, Extends, Imports |
| Python | `.py`, `.pyi` | Functions, classes, methods, constants | Calls, Extends, Imports |
| Go | `.go` | Functions, structs, interfaces, methods | Calls, Implements, Imports |
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
- **fastembed** - Local embedding generation (all-MiniLM-L6-v2, 384 dimensions)
- **OpenAI** - Optional embedding generation (text-embedding-3-small, 1536 dimensions)
- **sqlite-vec** - Fast vector similarity search

## CLI Reference

```
ctx - Generate AI-ready context from your codebase

USAGE:
    ctx [OPTIONS] [PATTERNS]...
    ctx <COMMAND>

COMMANDS:
    index       Build or update the code intelligence index
    query       Query the code intelligence database
    search      Search for symbols using keyword matching
    semantic    Search using embeddings (natural language)
    embed       Generate embeddings for semantic search
    source      Get the source code for a symbol
    explain     Explain a symbol with its relationships
    smart       Intelligently select files for a task
    diff        Generate context for changed files
    review      Generate context for PR review (GitHub)
    audit       Run code quality analysis
    complexity  Analyze code complexity
    duplicates  Detect duplicate code blocks
    graph       Generate dependency graph
    shell       Interactive codebase explorer
    serve       Start MCP server (with --mcp flag, requires mcp feature)

CONTEXT OPTIONS:
    -f, --format <FORMAT>    Output format [default: xml] [values: xml, markdown, md, plain, json]
        --no-gitignore       Disable .gitignore pattern matching
    -i, --ignore <PATTERN>   Additional ignore patterns
        --no-default-ignores Disable built-in ignore patterns
        --show-sizes         Show file sizes in project tree
        --no-tree            Disable project tree in output
        --no-stream          Buffer output instead of streaming
        --stats              Print stats after completion
        --count-only         Only count tokens, don't output
        --max-tokens <N>     Limit output to N tokens
        --encoding <ENC>     Tokenizer encoding [default: cl100k_base]

INDEX OPTIONS:
    -w, --watch              Watch for changes and reindex automatically
    -v, --verbose            Show verbose output
        --force              Force full reindex (clears existing database)
    -j, --parallel           Use parallel parsing (faster on multi-core)
        --no-gitignore       Disable .gitignore pattern matching
        --no-default-ignores Disable built-in ignore patterns
    -i, --ignore <PATTERN>   Additional ignore patterns
    -p, --pattern <PATTERN>  File patterns to include
```

## Performance

- Indexes ~2000 files in under 10 seconds
- Parallel indexing with `--parallel` flag (~1.7x speedup)
- Incremental updates only reindex changed files
- Fast vector search with sqlite-vec
- Compressed source storage (~70% size reduction)
- In-memory DuckDB for fast analytical queries
- Local embeddings with fastembed (~90MB model, runs offline)

## Environment Variables

| Variable | Description |
|----------|-------------|
| `OPENAI_API_KEY` | Required for `--openai` flag with `embed` and `semantic` commands |
| `GITHUB_TOKEN` | Optional for `review` command (uses `gh` CLI auth by default) |

## Examples

### Generate context for a bug fix
```bash
# Find relevant code and generate context
ctx smart "fix authentication timeout bug" --max-tokens 10000 | pbcopy
```

### Review a pull request
```bash
# Get context for PR review
ctx review 42 --summary --include-comments
```

### Pre-commit quality check
```bash
# Add to .git/hooks/pre-commit
ctx audit --min-score 7.0 --incremental || exit 1
```

### CI/CD integration
```bash
# In your CI pipeline
ctx audit --output json > quality-report.json
ctx audit --min-score 8.0 || exit 1
```

### Explore codebase interactively
```bash
ctx shell
ctx> find handleAuth
ctx> callers handleAuth
ctx> impact handleAuth
ctx> source handleAuth
```

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on development setup, coding style, and the pull request process.

## Security

To report a security vulnerability, see [SECURITY.md](SECURITY.md).

## License

This project is licensed under either of:

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
