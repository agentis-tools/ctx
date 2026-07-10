# Getting Started

This guide walks you through installing ctx and using it to generate context for LLMs and build a searchable code intelligence index.

## Installation

### From Source (Recommended)

```bash
git clone https://github.com/yourusername/ctx
cd ctx
cargo build --release

# Add to your PATH (choose one):
cp target/release/ctx /usr/local/bin/
# or
export PATH="$PATH:$(pwd)/target/release"
```

### Using Cargo Install

```bash
# From crates.io (installs the ctx binary)
cargo install agentis-ctx

# Or from a local checkout
cargo install --path .
```

### Verify Installation

```bash
ctx --version
# ctx 0.2.0
```

## Part 1: Context Generation

The primary use case for ctx is generating formatted context for LLMs.

### Your First Context

Navigate to any project and generate context:

```bash
cd my-project
ctx
```

This outputs all source files in XML format (the default), ready to paste into an LLM.

### Copy to Clipboard

**macOS:**
```bash
ctx src/ | pbcopy
```

**Linux:**
```bash
ctx src/ | xclip -selection clipboard
```

**WSL:**
```bash
ctx src/ | clip.exe
```

### Select Specific Files

```bash
# Just Rust files
ctx "**/*.rs"

# Multiple patterns
ctx "src/**/*.ts" "lib/**/*.ts"

# Specific directories
ctx src/ lib/ tests/

# Mix of patterns and paths
ctx src/ "tests/**/*.test.ts" package.json
```

### Choose Output Format

```bash
# XML (default) - best for most LLMs
ctx src/

# Markdown - good for chat interfaces
ctx --format markdown src/
ctx --format md src/       # alias

# Plain text - minimal formatting
ctx --format plain src/
```

### View Statistics

```bash
ctx --stats src/
# Generated context: 42 files, 156.3 KB in 23ms
```

## Part 2: Code Intelligence

ctx includes a powerful code intelligence system for understanding your codebase.

### Build the Index

```bash
ctx index
```

This creates `.ctx/codebase.sqlite` containing:
- **Symbols** - Functions, classes, interfaces, structs, enums, traits
- **Edges** - Call relationships, imports, extends, implements
- **Files** - Metadata and compressed source code
- **FTS Index** - Full-text search across symbol names and documentation

Example output:
```
Indexing codebase...
Indexed 20 files (46 skipped, 0 failed)
Extracted 548 symbols, 2664 edges in 890ms

Codebase statistics:
  Files:     20
  Symbols:   548
  Functions: 446
  Structs:   35
  Enums:     11
  Traits:    3
  Edges:     2664
```

### Search Your Code

**Keyword search:**
```bash
ctx search "handleRequest"
ctx search "auth"
ctx search "parse config"
```

**Semantic search (requires embeddings):**
```bash
# Generate embeddings first (one-time, uses local model ~90MB)
ctx embed

# Then search with natural language
ctx semantic "error handling and recovery"
ctx semantic "functions that validate user input"
ctx semantic "database connection management"
```

### Explore Relationships

**Who calls this function?**
```bash
ctx query callers processPayment
```

Output:
```
Functions that call 'processPayment':
------------------------------------------------------------
  handleOrder (src/orders/handler.ts:45)
    > await processPayment(order.total)
  retryTransaction (src/payments/retry.ts:23)
    > return processPayment(amount)
```

**What does this function call?**
```bash
ctx query deps validateInput
```

Output:
```
Dependencies of 'validateInput':
------------------------------------------------------------
  calls checkRequired (line 12)
  calls sanitize (line 15)
  calls validateSchema (line 18)
```

**What would break if I change this?**
```bash
ctx query impact authenticate
```

Output:
```
Impact analysis for 'authenticate' (depth=5):
The following would be affected by changes:
----------------------------------------------------------------------

Distance 1:
  handleLogin (src/auth/login.ts) [function]
  protectedRoute (src/middleware/auth.ts) [function]

Distance 2:
  UserController (src/controllers/user.ts) [class]
  AdminController (src/controllers/admin.ts) [class]

Total: 4 symbols affected
```

### Visualize Call Graphs

```bash
# Text format (default)
ctx query graph main --depth 3

# GraphViz DOT format (for visualization)
ctx query graph main --depth 3 --output dot > graph.dot
dot -Tpng graph.dot -o graph.png

# JSON format (for programmatic use)
ctx query graph main --depth 3 --output json
```

### Get Detailed Symbol Information

```bash
ctx explain handleAuth
```

Output:
```
Symbol: handleAuth
============================================================
Kind:       function
File:       src/auth/handler.ts:45
Visibility: public

Signature:
  async function handleAuth(req: Request): Promise<Response>

Description:
  Handles authentication requests and returns JWT tokens.

Called by (3):
  loginRoute (src/routes/auth.ts:12)
  refreshRoute (src/routes/auth.ts:34)
  apiMiddleware (src/middleware/api.ts:8)

Calls (5):
  validateCredentials [function]
  generateToken [function]
  hashPassword [function]
  ...
```

### Retrieve Source Code

```bash
ctx source handleAuth
```

Output:
```typescript
// Source: src/auth/handler.ts::handleAuth::45
async function handleAuth(req: Request): Promise<Response> {
  const { username, password } = req.body;
  
  const user = await validateCredentials(username, password);
  if (!user) {
    return new Response("Unauthorized", { status: 401 });
  }
  
  const token = generateToken(user);
  return Response.json({ token });
}
```

## Part 3: Code Analysis

ctx includes tools for analyzing code quality.

### Complexity Analysis

Find functions that call too many other functions (high fan-out):

```bash
ctx complexity --warnings-only
```

Output:
```
Code Complexity Analysis (threshold: 10)
==========================================================================================
FUNCTION                             FAN-OUT   FAN-IN    SCORE SEVERITY   FILE
------------------------------------------------------------------------------------------
extract_symbols                           48        4      100 HIGH     src/parser/typescript.rs:310
discover_files                            46        2       94 HIGH     src/walker.rs:67
------------------------------------------------------------------------------------------
Total: 94 functions analyzed
  0 critical, 2 high complexity functions need attention
```

### Duplicate Detection

Find copy-pasted code, even when variables were renamed or literals changed
(functions are compared structurally with MinHash fingerprints built during
`ctx index`):

```bash
ctx duplicates
```

Output:
```
Near-duplicate functions (Jaccard similarity of 5-token shingles >= 0.85, >= 50 tokens)
====================================================================================================

1. similarity 0.938
   src/parser/python.rs:318 extract_edges (74 tokens)
   src/parser/typescript.rs:430 extract_edges (76 tokens)
----------------------------------------------------------------------------------------------------
Found 1 near-duplicate pair(s).
```

Tune it with `--threshold <0.0-1.0>` (Jaccard similarity, default 0.85) and
`--min-tokens <N>` (default 50, raise it to filter idiomatic boilerplate).
Use `--against main` to only check functions in changed files, and
`--fail-on-found` to exit 1 in CI when a pair is reported.

### Dependency Graph

Visualize how files depend on each other:

```bash
# DOT format (GraphViz)
ctx graph --by-file > deps.dot
dot -Tpng deps.dot -o deps.png

# Mermaid format (for markdown)
ctx graph --by-file --output mermaid
```

## Part 4: Watch Mode

Keep the index fresh during development:

**Terminal 1 - Watch for file changes:**
```bash
ctx index --watch
```

**Terminal 2 - Auto-embed new symbols:**
```bash
ctx embed --watch
```

Now any file changes are automatically indexed and embedded.

### Filtered Watch Mode

Watch only specific files or exclude patterns:

```bash
# Only watch TypeScript files in src/
ctx index --watch -p "src/**/*.ts"

# Watch everything except test files
ctx index --watch -i "**/*.test.ts" -i "**/*.spec.ts"

# Combine include and ignore patterns
ctx index --watch -p "src/" -i "src/generated/"
```

Watch mode respects all the same filtering as initial indexing, including `.gitignore`, `.contextignore`, and built-in ignores.

## Part 5: Project Configuration

### Create .contextignore

Create a `.contextignore` file for project-specific exclusions:

```gitignore
# Test fixtures
fixtures/
__snapshots__/

# Generated code
*.generated.ts
dist/

# Large data files
*.sql
*.csv
```

### What's Automatically Ignored

ctx automatically excludes:
- Everything in `.gitignore`
- 170+ built-in patterns (binaries, node_modules, build outputs, etc.)
- Files matching `.contextignore`

### Custom Ignores on Command Line

```bash
ctx -i "*.test.ts" -i "fixtures/" src/
```

## Part 6: Smart Context Selection

Let ctx intelligently select files based on your task:

```bash
# Describe what you're working on
ctx smart "add user authentication" --max-tokens 8000

# Preview what would be selected
ctx smart "fix login bug" --dry-run

# See why each file was selected
ctx smart "refactor parser" --explain
```

## Part 7: Diff-Aware Context

Generate context focused on code changes:

```bash
# Changes since last commit
ctx diff

# Changes vs main branch
ctx diff main

# Only staged changes
ctx diff --staged

# Include change summary
ctx diff --summary
```

## Part 8: PR Review Context

Generate context for GitHub pull request review:

```bash
# Review PR #123
ctx review 123

# Include PR comments
ctx review 123 --include-comments

# With change summary
ctx review 123 --summary
```

**Note:** Requires GitHub CLI (`gh`) to be installed and authenticated.

## Part 9: Code Quality Audit

Run automated quality analysis:

```bash
# Full quality report
ctx audit

# Quality gate for CI
ctx audit --min-score 7.0

# JSON output for automation
ctx audit --output json
```

## Part 10: Interactive Shell

Explore your codebase interactively:

```bash
ctx shell
```

Shell commands:
- `find <pattern>` - Find symbols
- `search <query>` - Hybrid search
- `callers <fn>` - Show callers
- `callees <fn>` - Show callees
- `source <symbol>` - Show source
- `explain <symbol>` - Explain symbol
- `impact <symbol>` - Impact analysis
- `stats` - Codebase statistics
- `help` - Show all commands

## Part 11: MCP Server (Claude Desktop)

Integrate ctx with Claude Desktop:

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

## Common Workflows

### Workflow 1: Quick LLM Context

```bash
# Generate context for current bug/feature
ctx "src/auth/**/*.ts" | pbcopy

# Paste into ChatGPT, Claude, etc.
```

### Workflow 2: Understanding New Codebase

```bash
# Index the codebase
ctx index

# Get an overview
ctx query stats

# Find entry points
ctx search "main"
ctx search "app"

# Trace the call flow
ctx query graph main --depth 4
```

### Workflow 3: Safe Refactoring

```bash
# Before changing a function, check impact
ctx query impact authenticate --depth 5

# See who calls it
ctx query callers authenticate

# Make changes, then reindex
ctx index
```

### Workflow 4: Code Review Prep

```bash
# Find complex functions that might need review
ctx complexity --warnings-only

# Find near-duplicate functions (structural match, renames ignored)
ctx duplicates --threshold 0.9

# Understand file dependencies
ctx graph --by-file --output mermaid
```

### Workflow 5: Smart Context for Tasks

```bash
# Let ctx select relevant files for your task
ctx smart "add caching to the database layer" --max-tokens 10000 | pbcopy

# Preview selection first
ctx smart "fix authentication bug" --dry-run --explain
```

### Workflow 6: PR Review

```bash
# Get context for reviewing a PR
ctx review 42 --summary --include-comments

# Focus on just the changed files
ctx review 42 --changes-only
```

### Workflow 7: CI/CD Integration

```bash
# Quality gate in CI pipeline
ctx audit --min-score 7.0 --output json > quality-report.json

# Pre-commit hook
ctx audit --incremental --min-score 7.0 || exit 1
```

## Next Steps

- [Context Generation](context-generation.md) - All output formats and filtering options
- [Code Intelligence](code-intelligence.md) - Deep dive into indexing and querying
- [Configuration](configuration.md) - .contextignore and environment variables
- [Language Support](language-support.md) - What's extracted from each language
- [Architecture](architecture.md) - How ctx works under the hood
