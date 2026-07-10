# ctx

[![Crates.io](https://img.shields.io/crates/v/agentis-ctx)](https://crates.io/crates/agentis-ctx)
[![CI](https://github.com/agentis-tools/ctx/actions/workflows/ci.yml/badge.svg)](https://github.com/agentis-tools/ctx/actions)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](#license)
[![Rust Version](https://img.shields.io/badge/rust-1.91%2B-orange)](https://www.rust-lang.org)
[![Docs](https://img.shields.io/badge/docs-docs.agentis.tools-blue)](https://docs.agentis.tools/)

**AI coding agents write fast, sloppy code** — they duplicate logic that already exists, drift the architecture, and declare "done" without proof. **`ctx` is the local quality authority for AI-written code**: a single local binary that builds a queryable model of your codebase and uses it to both *guide* your agent (a map before it starts, the blast radius of every edit) and *govern* it (your architecture rules and quality thresholds as deterministic gates it can't ship past). Rules live in your repo as code; checks run in milliseconds inside the agent's loop; nothing leaves your machine.

> Unlike code-graph tools, ctx governs what agents *write* — not just what they read. Unlike quality platforms, it does so in milliseconds, locally, with gates you own.

📖 **Documentation:** https://docs.agentis.tools/

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
- **Quality gates** - Architecture rules (`ctx check`) and change scoring (`ctx score`) with a 0/1/2 exit-code convention for CI and AI agents
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

### Updating

If you installed a release binary, ctx can update itself from GitHub releases:

```bash
ctx self-update                    # update to the latest release
ctx self-update --version 0.3.0    # pin an exact release
ctx --version --check              # just compare, never install
```

Every download is verified against the release's `SHA256SUMS` file before the
binary is replaced; on mismatch the update aborts (exit 2) and the installed
binary is untouched. Cargo installs should update with
`cargo install agentis-ctx` instead.

**ctx never updates itself automatically.** On interactive runs it checks for
a newer release at most once per 24h (1s timeout, silent failure) and prints a
one-line notice to stderr — nothing more. Set `CTX_NO_UPDATE_CHECK=1` to
silence the notice entirely; it is also suppressed in `--json` mode, when
stderr is not a terminal, and inside Claude Code hooks.

The Claude Code plugin generated by `ctx harness init --mode plugin` is
versioned in lockstep with the binary and updates through Claude Code itself
via `/plugin update ctx`. For frozen, reproducible installs, pin the
marketplace entry to a commit SHA when adding it
(`/plugin marketplace add agentis-tools/ctx@<commit-sha>`) — the plugin
then never moves until you re-pin, and the generated permissions deny
`Bash(ctx self-update*)` so agents cannot update the binary either. See
[`docs/commands/self-update.md`](docs/commands/self-update.md).

## Using ctx as a Library

Everything the CLI does is available as a Rust library, so you can embed
indexing, search, and context generation in your own tools. The package is
`agentis-ctx`, but the library target is named `ctx`:

```toml
[dependencies]
agentis-ctx = "0.3"

# On Windows (or to skip DuckDB analytics):
# agentis-ctx = { version = "0.3", default-features = false }
```

```rust
use ctx::prelude::*;
use std::path::Path;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let root = Path::new("./my-project");

    // Build (or incrementally update) the index at .ctx/codebase.sqlite
    let mut indexer = Indexer::with_config(root, false, WalkerConfig::default())?;
    let result = indexer.index()?;
    println!("{} symbols extracted", result.symbols_extracted);

    // Keyword search over the indexed symbols
    let db = open_database(root)?;
    for symbol in db.find_symbols("authenticate", 10)? {
        println!("{} ({}:{})", symbol.name, symbol.file_path, symbol.line_start);
    }
    Ok(())
}
```

The [API documentation](https://docs.rs/agentis-ctx) covers the full surface:
smart context selection (`smart`), diff-aware context (`diff`), semantic
search via local or OpenAI embeddings (`embeddings`), call-graph analytics
(`analytics`), token counting (`tokens`), and output formatting
(`formatter`/`output`).

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

### `ctx similar <description>`
Find existing functions similar to a description — reuse before you write. Before writing a new function, run `ctx similar` with its intended purpose; if a strong match exists (high similarity and fan-in), extend or reuse it instead.

```bash
ctx similar "parse a config file into a struct"   # Embedding search (functions/methods only)
ctx similar "retry with backoff" --keyword        # FTS5 fallback, no embeddings needed
ctx similar "count tokens" --limit 5 --json       # Machine-readable output
```

Each hit shows the symbol, similarity score, fan-in (how many callers it already has), and a one-line doc. Requires `ctx embed` first (exits with code 2 otherwise); `--keyword` works without embeddings.

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

### `ctx sql`
Run read-only SQL against the code intelligence index through DuckDB, over a
stable `v1` view layer (`v1.symbols`, `v1.edges`, `v1.files`, `v1.meta`). Reach
for `ctx sql` instead of the canned `ctx query` subcommands whenever you need an
aggregation, a join, or a custom `WHERE` condition. Query `v1.*` only — anything
else (e.g. `code.*`) is internal and unstable. For agents and scripts, use
`--json` with `--max-rows`. Run `ctx sql --schema` for the full column reference.

```bash
# Ten most complex symbols
ctx sql "SELECT name, file, complexity FROM v1.symbols ORDER BY complexity DESC LIMIT 10;"

# Symbol counts by kind
ctx sql "SELECT kind, COUNT(*) AS n FROM v1.symbols GROUP BY kind ORDER BY n DESC;"

# Public functions that nothing calls (dead-code candidates)
ctx sql "SELECT name, file FROM v1.symbols WHERE kind IN ('function', 'method') AND is_public AND fan_in = 0 ORDER BY file, name;"
```

Use `--fail-on-rows` to turn a query into a gate: any returned row is a
violation, and the command exits `1`.

```bash
# Fails (exit 1) if the query returns any row
ctx sql --fail-on-rows --file .ctx/gates/no-utils-imports.sql
```

Access is read-only and engine-hardened (filesystem access, extension loading,
and file-based `ATTACH` are disabled; the index cannot be modified), so
`Bash(ctx sql *)` is safe to add to a Claude Code allow-list.

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

## Quality Gates

A suite of quality commands designed to be composed into CI pipelines and AI
agent hooks: `ctx check` (architecture rules from `.ctx/rules.toml`),
`ctx score` (quality delta of your changes vs. a git reference),
`ctx duplicates` (MinHash near-duplicate detection), `ctx hotspots`
(churn x complexity refactoring targets), `ctx similar` (find existing
functions before writing new ones), and `ctx map` (token-budgeted codebase
overview for LLM sessions).

All of them share a three-way exit-code convention -- that convention is the
integration API:

| Code | Meaning |
|------|---------|
| 0 | Success, nothing to report |
| 1 | Ran successfully but produced findings |
| 2 | Operational error (bad arguments, missing index, git failure, ...) |

```bash
# Enforce architecture rules; --against reports only new violations
ctx check --against main

# Score your changes: complexity/fan-out deltas, new duplication,
# rule violations, symbol churn -- with CI gate conditions
ctx score --against main --fail-on "check_violations>0,new_duplication>0"

# Wire the whole suite into Claude Code (hooks, permissions, plugin scaffold)
ctx harness init
```

See the [Quality Gates guide](https://docs.agentis.tools/docs/integrations/quality-gates)
for the full suite, CI recipes, and the reference Claude Code hook
configuration, and [`docs/json-output.md`](docs/json-output.md) for the
machine-readable `--json` contract.

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

Detect structurally similar functions with MinHash fingerprints built during
`ctx index`. Functions are compared by the Jaccard similarity of their
normalized token shingles (identifiers -> `ID`, literals -> `LIT`, comments
dropped), so renamed variables and changed string literals still match.
Solidity functions are skipped (no tree-sitter grammar).

```bash
ctx duplicates                     # Default: Jaccard >= 0.85, >= 50 tokens
ctx duplicates --threshold 0.9     # Require 90% shingle overlap (0.0-1.0)
ctx duplicates --min-tokens 80     # Ignore functions under 80 tokens
ctx duplicates --against main      # Only pairs touching files changed vs main
ctx duplicates --fail-on-found     # Exit 1 when any pair is found (CI gate)
ctx duplicates --json              # Machine-readable JSON envelope
```

> **Breaking change:** the old line-based `--similarity <PERCENT>` /
> `--min-lines <N>` flags are gone. `--threshold` is a 0.0-1.0 Jaccard
> similarity over 5-token shingles, not a percentage of matching lines.
> Rebuild the index once with `ctx index --force` after upgrading.

## Change Scoring

Score the quality delta of your working tree (or branch) against a git
reference. Baselines are parsed in memory at the reference with the same
parser, so the deltas compare like with like:

```bash
ctx score                          # Score uncommitted changes (vs HEAD)
ctx score --against main           # Score the whole branch / PR
ctx score --fail-on "new_duplication>0,complexity_delta>=25"   # CI gate
ctx score --against main --json    # Machine-readable JSON envelope
```

**Metrics** (usable in `--fail-on` as `metric OP value` with `>=`, `<=`, `>`, `<`):
`complexity_delta`, `fan_out_delta`, `new_duplication`, `check_violations`,
`symbols_added`, `symbols_removed`, `files_changed`.

The index is refreshed incrementally before scoring; exit codes are 0 (clean),
1 (a `--fail-on` condition was met), 2 (operational error).

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
| Go | `.go` | Functions, structs, interfaces, methods | Calls, Imports |
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
    sql         Run read-only SQL against the index (v1 schema)
    search      Search for symbols using keyword matching
    semantic    Search using embeddings (natural language)
    embed       Generate embeddings for semantic search
    source      Get the source code for a symbol
    explain     Explain a symbol with its relationships
    smart       Intelligently select files for a task
    diff        Generate context for changed files
    review      Generate context for PR review (GitHub)
    audit       Run code quality analysis
    check       Check architecture rules from .ctx/rules.toml
    score       Score the quality delta of changes vs a git reference
    complexity  Analyze code complexity
    duplicates  Detect structurally similar functions (MinHash)
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
