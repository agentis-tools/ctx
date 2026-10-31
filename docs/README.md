# ctx documentation

ctx is a local Rust CLI that indexes a codebase into symbols, call graphs, and relationships. It
uses that index to select context for language models and to check the changes they make.

## Contents

* [Getting started](getting-started.md): installation and first steps
* [Context generation](context-generation.md): packing files for a model
* [Code intelligence](code-intelligence.md): indexing, search, and graph queries
* [Configuration](configuration.md): ignore files and `.ctx/config.toml`
* [Language support](language-support.md): supported languages and what ctx extracts
* [Add a language via LSP](lsp-languages.md): index any language through its language server
* [Architecture](architecture.md): how ctx works internally
* [JSON output](json-output.md): the `--json` envelope and per-command payloads
* [Quality gates](integrations/quality-gates.md): `check`, `score`, and exit codes in CI and agents
* [Packaging and distribution](packaging.md): release assets and package-manager maintenance
* [Privacy policy](privacy.md): what ctx and its plugins send over the network

## Why ctx

An agent working on a repository needs two things from it. Before it edits, it needs the files
that matter for the task, without `node_modules`, build output, or unrelated code. After it
edits, someone needs to check what the change affects and whether it broke the architecture.

ctx does both from one index. Context generation packs files for a model:

```bash
ctx src/ | pbcopy  # copy formatted source to the clipboard
```

Code intelligence answers questions about the code:

```bash
ctx index                      # build the index
ctx search "auth"              # find symbols by name
ctx query callers handleLogin  # who calls this function?
ctx query impact validateToken # what depends on it?
```

Governance commands check a change:

```bash
ctx check --against main                                  # new architecture rule violations
ctx score --against main --fail-on "check_violations>0"   # scorecard with a gate
```

## Quick start

### Generate context for a model

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

### Build and query the index

```bash
# Build the index (creates .ctx/codebase.sqlite)
ctx index

# Search for symbols (keyword matching)
ctx search "handleRequest"

# Semantic search (local, Ollama, or OpenAI embeddings)
ctx embed                          # Generate embeddings once
ctx semantic "authentication logic" # Search by meaning

# Find all callers of a function
ctx query callers authenticate

# See what a function depends on
ctx query deps processPayment

# Visualize call graph
ctx query graph main --depth 3

# Impact analysis: what depends on this symbol?
ctx query impact validateInput

# Code quality analysis
ctx complexity --warnings-only   # Find high fan-out functions
ctx duplicates                   # Find near-duplicate functions
ctx graph --by-file              # Visualize file dependencies

# Watch for changes and auto-reindex
ctx index --watch
```

## Command reference

`ctx <command> --help` prints the exact flags for your installed version. The pages below cover
each command in more depth.

### Context generation

```
ctx [OPTIONS] [PATTERNS]...
```

`PATTERNS` are files, directories, or globs such as `"src/**/*.rs"`; the default is the current
directory. Common options:

| Option | Effect |
|---|---|
| `-f, --format <FORMAT>` | `xml` (default), `markdown`/`md`, `plain`/`text`, or `json` |
| `--max-tokens <N>` | Stop adding files at a token budget |
| `--count-only` | Count tokens instead of printing contents |
| `--encoding <ENCODING>` | Tokenizer: `cl100k_base` (default), `o200k_base`, `p50k_base` |
| `-i, --ignore <PATTERN>` | Extra ignore pattern (repeatable) |
| `--no-gitignore` / `--no-default-ignores` | Disable `.gitignore` or the 164 built-in ignore patterns |
| `--show-sizes` / `--no-tree` | Show file sizes in the tree, or drop the tree |
| `--no-stream` / `--stats` | Buffer output; print file count, size, and time |
| `--json` | Machine-readable envelope on stdout (see [JSON output](json-output.md)) |

See [Context generation](context-generation.md).

### Subcommands

| Command | Description | Reference |
|---|---|---|
| `ctx index` | Build or update the index (`--watch`, `--force`, `--serial`, `-p` scope) | [Code intelligence](code-intelligence.md) |
| `ctx embed` | Generate embeddings (`--provider local\|openai\|ollama`, `--force` rebuilds the corpus) | [Code intelligence](code-intelligence.md#embeddings) |
| `ctx query find\|callers\|deps\|graph\|impact\|stats\|files` | Query symbols and the call graph | [Code intelligence](code-intelligence.md) |
| `ctx search` / `ctx semantic` | Keyword (FTS5) or embedding search over symbols | [Code intelligence](code-intelligence.md) |
| `ctx source` / `ctx explain` | Stored source of a symbol; a symbol with its calls and relationships | [Code intelligence](code-intelligence.md) |
| `ctx smart` | Select whole files for a task within a hard token budget | [smart](commands/smart.md) |
| `ctx map` | Architectural overview within a token budget | [map](commands/map.md) |
| `ctx diff` / `ctx review` | Context for git changes or a GitHub pull request | [diff](commands/diff.md) |
| `ctx similar` | Find existing functions before writing new ones | [similar](commands/similar.md) |
| `ctx check` | Enforce architecture rules from `.ctx/rules.toml` | [check](commands/check.md) |
| `ctx score` | Scorecard and `--fail-on` gate for a change against a ref | [score](commands/score.md) |
| `ctx duplicates` | MinHash near-duplicate detection | [duplicates](commands/duplicates.md) |
| `ctx hotspots` | Churn times size (default) or complexity | [hotspots](commands/hotspots.md) |
| `ctx complexity` | Fan-out report (`--threshold`, `--warnings-only`) | [Code intelligence](code-intelligence.md) |
| `ctx audit` | Code quality report with a `--min-score` gate | [audit](commands/audit.md) |
| `ctx graph` | Dependency graph as DOT, Mermaid, or JSON | [Code intelligence](code-intelligence.md) |
| `ctx sql` | Read-only SQL over the `v1.*` views, `--fail-on-rows` gates | [sql](commands/sql.md) |
| `ctx snapshot` | Per-commit Parquet metrics and backfill | [snapshot](commands/snapshot.md) |
| `ctx judge edges` | Opt-in: re-resolve ambiguous call edges with a decision model | [judge](commands/judge.md) |
| `ctx lsp` | Add, list, update, and check language servers | [lsp](commands/lsp.md) |
| `ctx harness` | Claude Code and Codex hooks and plugins (`init`, `doctor`, `compat`) | [harness](commands/harness.md) |
| `ctx shell` | Interactive REPL | [shell](commands/shell.md) |
| `ctx serve --mcp` | MCP server over stdio (build with `--features mcp`) | [serve](commands/serve.md) |
| `ctx self-update` | Update a direct GitHub installation | [self-update](commands/self-update.md) |

## Use cases

### Context for a model

```bash
# Quick context for a bug fix
ctx "src/auth/**/*.ts" | pbcopy

# Full project context
ctx src/ lib/ --format markdown > CONTEXT.md

# Minimal context (no tree)
ctx --no-tree src/api.ts src/types.ts
```

### Understanding code

```bash
# "I need to modify authenticate() - what calls it?"
ctx query callers authenticate

# "What depends on validateToken?"
ctx query impact validateToken --depth 5

# "Show me the call flow from main"
ctx query graph main --depth 4 --output dot | dot -Tpng -o flow.png

# "Find all authentication-related code"
ctx semantic "user authentication and login"
```

### Code quality

```bash
# "Find functions that do too much"
ctx complexity --warnings-only

# "Find copy-pasted code" (even with renamed variables)
ctx duplicates --threshold 0.9

# "Visualize module dependencies"
ctx graph --by-file --output mermaid
```

### Navigation

```bash
# "Where is handleRequest defined?"
ctx search "handleRequest"

# "Show me the source of that function"
ctx source handleRequest

# "Tell me everything about this function"
ctx explain handleRequest
```

## Getting help

```bash
ctx --help           # General help
ctx index --help     # Index command help
ctx query --help     # Query command help
ctx search --help    # Search command help
ctx embed --help     # Embedding command help
ctx complexity --help  # Complexity analysis help
ctx duplicates --help  # Duplicate detection help
ctx graph --help       # Graph visualization help
```
