---
id: intro
title: Introduction
sidebar_position: 1
slug: /
---

# ctx Documentation

Welcome to the ctx documentation. **ctx** is a fast CLI tool that generates AI-ready context from codebases, with built-in code intelligence for understanding symbol relationships, call graphs, and codebase structure.

## Why ctx?

### The Problem

When working with LLMs for coding assistance, you face two challenges:

**1. Context Sharing**
- Manually copying files is tedious and error-prone
- Easy to include irrelevant files (node_modules, build artifacts, binaries)
- Hard to maintain a consistent format for the LLM

**2. Understanding Large Codebases**
- What functions call what?
- What would break if you change something?
- Where is a particular pattern used?
- How do different modules relate to each other?

### The Solution

ctx solves both problems with two complementary tools:

**Context Generation** - Select files with glob patterns, automatically filter out noise, and format output for LLMs:
```bash
ctx src/ | pbcopy  # Copy formatted source to clipboard
```

**Code Intelligence** - Build a searchable index with call graphs, impact analysis, and semantic search:
```bash
ctx index                    # Build the index
ctx search "auth"            # Find symbols
ctx query callers handleLogin # Who calls this function?
ctx query impact validateToken # What breaks if I change this?
```

## Quick Start

### Generate Context for an LLM

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

### Build and Query the Code Intelligence Index

```bash
# Build the index (creates .ctx/codebase.sqlite)
ctx index

# Search for symbols (keyword matching)
ctx search "handleRequest"

# Semantic search (natural language, local or OpenAI)
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

# Code quality analysis
ctx complexity --warnings-only   # Find high fan-out functions
ctx duplicates                   # Find duplicate code
ctx graph --by-file              # Visualize file dependencies

# Watch for changes and auto-reindex
ctx index --watch
```

## Complete Command Reference

### Context Generation (Default)

```
ctx [OPTIONS] [PATTERNS]...

Arguments:
  [PATTERNS]...  File patterns or paths (glob syntax supported)
                 Examples: "src/**/*.rs", "*.ts", "src/"
                 Default: "." (current directory)

Options:
  -f, --format <FORMAT>     Output format [default: xml]
                            Values: xml, markdown, md, plain, json
      --no-gitignore        Disable .gitignore pattern matching
  -i, --ignore <PATTERN>    Additional ignore patterns (repeatable)
      --no-default-ignores  Disable built-in ignore patterns (170+)
      --show-sizes          Show file sizes in project tree
      --no-tree             Disable project tree in output
      --no-stream           Buffer output instead of streaming
      --stats               Print statistics (file count, size, time, token estimate)
      --count-only          Count tokens only; do not print file contents
      --max-tokens <N>      Omit whole files to fit a token budget (never truncates a file)
      --encoding <ENCODING> Tokenizer encoding [default: cl100k_base]
                            Values: cl100k_base, o200k_base, p50k_base
  -h, --help                Print help
  -V, --version             Print version
```

### Subcommands

| Command | Description |
|---------|-------------|
| `ctx index` | Build or update the code intelligence index |
| `ctx query` | Query the code intelligence database |
| `ctx search` | Search for symbols using keyword matching |
| `ctx semantic` | Semantic search using embeddings |
| `ctx embed` | Generate embeddings for semantic search |
| `ctx source` | Get the source code for a symbol |
| `ctx explain` | Explain a symbol with its relationships |
| `ctx complexity` | Analyze code complexity and fan-out |
| `ctx duplicates` | Detect duplicate or similar code |
| `ctx graph` | Generate dependency graph visualization |

### Index Options

```
ctx index [OPTIONS]

Options:
  -w, --watch                Watch for changes and reindex automatically
  -v, --verbose              Show verbose output (files being indexed)
      --force                Force full reindex (clears existing database)
  -j, --parallel <N>         Number of parallel indexing threads
  -p, --pattern <PATTERN>    Only index files matching these patterns (repeatable)
  -i, --ignore <PATTERN>     Additional ignore patterns (repeatable)
      --no-gitignore         Disable .gitignore pattern matching
      --no-default-ignores   Disable built-in ignore patterns
```

### Query Subcommands

```
ctx query find <PATTERN>      Find symbols by name pattern
ctx query callers <FUNCTION>  Show functions that call a given function
ctx query deps <SYMBOL>       Show what a function depends on
ctx query graph <START>       Show the call graph from a starting point
ctx query impact <SYMBOL>     Analyze impact of changing a symbol
ctx query stats               Show codebase statistics
ctx query files               List all indexed files
```

### Search Options

```
ctx search <QUERY> [OPTIONS]

Options:
  -l, --limit <N>     Maximum results [default: 20]
      --output <FMT>  Output format: table, json [default: table]
```

### Semantic Search Options

```
ctx semantic <QUERY> [OPTIONS]

Options:
  -l, --limit <N>     Maximum results [default: 10]
      --output <FMT>  Output format: table, json [default: table]
      --openai        Use OpenAI instead of local model
```

### Embedding Options

```
ctx embed [OPTIONS]

Options:
  -f, --force           Re-embed all symbols
  -v, --verbose         Show progress
      --batch-size <N>  Batch size [default: 50]
      --openai          Use OpenAI API instead of local model
  -w, --watch           Watch for index changes and auto-embed
```

### Complexity Analysis Options

```
ctx complexity [OPTIONS]

Options:
      --threshold <N>   Fan-out threshold [default: 10]
  -w, --warnings-only   Only show functions exceeding threshold
      --output <FMT>    Output format: table, json [default: table]
```

### Duplicate Detection Options

```
ctx duplicates [OPTIONS]

Options:
      --similarity <N>  Minimum similarity percentage [default: 80]
      --min-lines <N>   Minimum lines for comparison [default: 5]
      --output <FMT>    Output format: table, json [default: table]
```

### Graph Visualization Options

```
ctx graph [OPTIONS]

Options:
      --output <FMT>    Output format: dot, mermaid, json [default: dot]
      --by-file         Group by file instead of symbols
      --filter <FILES>  Only include these files (comma-separated)
      --depth <N>       Maximum traversal depth [default: 3]
```

## Key Features

- **Fast** - Written in Rust, indexes thousands of files in seconds
- **Smart filtering** - Respects .gitignore, excludes binaries and 170+ patterns
- **Multi-language** - Rust, TypeScript, JavaScript, JSX/TSX, Python, Go, Solidity, YAML
- **Single file database** - Everything in one portable SQLite file
- **Incremental updates** - Only reindex what changed
- **Watch mode** - Auto-reindex on file changes
- **Semantic search** - Natural language queries with local or OpenAI embeddings
- **Call graphs** - Understand function relationships and dependencies
- **Impact analysis** - Know what breaks before you change code
- **Code quality** - Complexity scoring and duplicate detection
- **Graph visualization** - DOT, Mermaid, and JSON output formats

## Use Cases

### For AI/LLM Interactions

```bash
# Quick context for a bug fix
ctx "src/auth/**/*.ts" | pbcopy

# Full project context
ctx src/ lib/ --format markdown > CONTEXT.md

# Minimal context (no tree)
ctx --no-tree src/api.ts src/types.ts
```

### For Code Understanding

```bash
# "I need to modify authenticate() - what calls it?"
ctx query callers authenticate

# "What would break if I change validateToken?"
ctx query impact validateToken --depth 5

# "Show me the call flow from main"
ctx query graph main --depth 4 --output dot | dot -Tpng -o flow.png

# "Find all authentication-related code"
ctx semantic "user authentication and login"
```

### For Code Quality

```bash
# "Find functions that do too much"
ctx complexity --warnings-only

# "Find copy-pasted code"
ctx duplicates --similarity 75

# "Visualize module dependencies"
ctx graph --by-file --output mermaid
```

### For Navigation

```bash
# "Where is handleRequest defined?"
ctx search "handleRequest"

# "Show me the source of that function"
ctx source handleRequest

# "Tell me everything about this function"
ctx explain handleRequest
```

## Getting Help

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
