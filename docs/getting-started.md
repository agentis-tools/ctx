# Getting started

This guide installs ctx, generates context for an LLM, and builds the code index that the
intelligence and governance commands read.

## Installation

### Package managers (recommended)

The crates.io package is named `agentis-ctx`. Every channel installs an executable named `ctx`:

```bash
cargo install agentis-ctx
cargo binstall agentis-ctx
brew install agentis-tools/tap/ctx
yay -S ctx-bin
```

On Windows with Scoop:

```powershell
scoop bucket add agentis-tools https://github.com/agentis-tools/scoop-bucket
scoop install ctx
```

Each GitHub release also carries Debian and RPM packages:

```bash
sudo apt install ./ctx_VERSION_amd64.deb
sudo dnf install ./ctx-VERSION-1.x86_64.rpm
```

The Homebrew tap, AUR package, and Scoop bucket are published from separate package-index
repositories. Prebuilt release binaries exist for Linux GNU x86-64, macOS Intel and Apple Silicon,
and Windows x86-64. Linux ARM64 and musl binaries are not published.

Use `ctx self-update` only for a binary you downloaded directly from GitHub Releases. Upgrade
Cargo, Homebrew, Arch, Scoop, Debian, and RPM installations with the package manager that owns
them; `ctx self-update` refuses to overwrite them.

### Without DuckDB

The default build includes the DuckDB analytics backend. The Windows and Intel macOS release
binaries are built without it. If it does not compile on your machine, install without it:

```bash
cargo install agentis-ctx --no-default-features
```

In that build, call graphs, impact analysis, and complexity return empty results, and `ctx sql`
and `ctx snapshot` exit with code 2.

### From source

```bash
git clone https://github.com/agentis-tools/ctx
cd ctx
cargo build --release

# Add to your PATH (choose one):
cp target/release/ctx /usr/local/bin/
# or
export PATH="$PATH:$(pwd)/target/release"
```

`cargo install --path .` from a checkout installs the same binary into Cargo's bin directory.

The MCP server (`ctx serve --mcp`) is behind the `mcp` Cargo feature. Release binaries and a plain
`cargo install agentis-ctx` do not include it. Build it yourself:

```bash
cargo build --release --features mcp
# or
cargo install agentis-ctx --features mcp
```

### Verify the installation

```bash
ctx --version
# ctx 0.4.0
```

## Part 1: Context generation

Running `ctx` with no subcommand prints the selected files as one formatted document for an LLM.

### Your first context

Go to a project and run:

```bash
cd my-project
ctx
```

ctx prints every source file it does not ignore, in XML (the default format).

### Copy to the clipboard

macOS:
```bash
ctx src/ | pbcopy
```

Linux:
```bash
ctx src/ | xclip -selection clipboard
```

WSL:
```bash
ctx src/ | clip.exe
```

### Select files

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

### Choose an output format

```bash
# XML (default)
ctx src/

# Markdown
ctx --format markdown src/
ctx --format md src/       # alias

# Plain text
ctx --format plain src/

# JSON, for tools
ctx --format json src/
```

### View statistics

```bash
ctx --stats src/
# Generated context: 42 files, 156.3 KB, ~38.9k tokens in 23ms
```

`--stats` writes the file count, total size, an approximate token count (output bytes divided by
4), and the elapsed time to stderr.

### Count tokens and fit a budget

```bash
# Count tokens with the tokenizer, without printing file contents
ctx --count-only src/

# Leave out whole files that would push the file contents past 8,000 tokens
# (ctx never truncates a file)
ctx --max-tokens 8000 src/

# Choose the tokenizer encoding (default: cl100k_base)
ctx --encoding o200k_base --count-only src/
```

The encodings are `cl100k_base` (default), `o200k_base`, and `p50k_base`.

## Part 2: Code intelligence

### Build the index

```bash
ctx index
```

> Run this first. Every intelligence and governance command (`query`, `search`, `map`, `check`,
> `score`, and the rest) reads the index. Later runs only reprocess changed files. To keep it
> current while you work, run `ctx index --watch`, and `ctx embed --watch` for semantic search.
> [Index and embed first](https://docs.agentis.tools/docs/guides/indexing) describes the full workflow.

`ctx index` writes `.ctx/codebase.sqlite`, which holds:
- symbols: functions, classes, interfaces, structs, enums, traits
- edges: calls, extends, implements, uses (imports are stored per file as module metadata)
- files: metadata and compressed source
- a full-text index over symbol names and documentation

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

The indexer parses files in parallel by default; `--serial` uses one thread. Include patterns
(`-p/--pattern`, or positional paths) and ignore patterns (`-i/--ignore`) limit what it indexes:

```bash
# Only index TypeScript sources
ctx index -p "src/**/*.ts"

# Positional paths scope the same way
ctx index src
```

If an explicit scope matches no files, `ctx index` exits non-zero and leaves the existing index
unchanged.

### Search your code

Keyword search:
```bash
ctx search "handleRequest"
ctx search "auth"
ctx search "parse config"
```

Semantic search needs embeddings:
```bash
# Generate embeddings once (the default local model is about 90 MB)
ctx embed

# Search with natural language
ctx semantic "error handling and recovery"
ctx semantic "functions that validate user input"
ctx semantic "database connection management"
```

### Explore relationships

Who calls this function?
```bash
ctx query callers processPayment
```

Output:
```
Functions that call 'processPayment' (src/payments/process.ts):
------------------------------------------------------------

Distance 1:
  handleOrder (src/orders/handler.ts:45)
    > await processPayment(order.total)
  retryTransaction (src/payments/retry.ts:23)
    > return processPayment(amount)
```

`ctx query callers` follows resolved call edges to a depth of 3 by default and groups results by
distance; `--depth 1` lists only direct callers. Calls that could not be resolved to this symbol
appear in a separate "Unresolved same-language call evidence" section.

What does this function call?
```bash
ctx query deps validateInput
```

Output:
```
Dependencies of 'validateInput' (src/validation/input.ts):
------------------------------------------------------------

Distance 1:
  calls checkRequired (line 12)
  calls sanitize (line 15)
  calls validateSchema (line 18)
```

What would break if I change this?
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

### Visualize call graphs

```bash
# Text format (default)
ctx query graph main --depth 3

# GraphViz DOT
ctx query graph main --depth 3 --output dot > graph.dot
dot -Tpng graph.dot -o graph.png

# JSON
ctx query graph main --depth 3 --output json
```

`ctx query graph` accepts `--output <text|json|dot>`. The default depth is 5.

### Get symbol details

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

`Calls` lists call edges only. Imports, `extends`, `implements`, and `uses` edges appear under a
separate `Relationships` section.

### Retrieve source code

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

## Part 3: Code analysis

### Complexity

List functions that call many other functions (high fan-out):

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

The score measures call coupling (fan-out and fan-in), not branches.

### Duplicate detection

`ctx duplicates` finds copied functions even when variables were renamed or literals changed. It
compares the MinHash fingerprints that `ctx index` builds for each function:

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

`--threshold <0.0-1.0>` sets the Jaccard similarity (default 0.85) and `--min-tokens <N>` the
minimum function size (default 50; raise it to skip idiomatic boilerplate). `--against main`
reports only pairs that touch changed files, and `--fail-on-found` exits 1 when any pair is
reported.

### Dependency graph

Show how files depend on each other:

```bash
# DOT format (GraphViz)
ctx graph --by-file > deps.dot
dot -Tpng deps.dot -o deps.png

# Mermaid, for Markdown
ctx graph --by-file --output mermaid
```

## Part 4: Watch mode

Keep the index current while you work.

Terminal 1, reindex on file changes:
```bash
ctx index --watch
```

Terminal 2, embed new symbols:
```bash
ctx embed --watch
```

### Filtered watch mode

Watch only some files, or exclude patterns:

```bash
# Only watch TypeScript files in src/
ctx index --watch -p "src/**/*.ts"

# Watch everything except test files
ctx index --watch -i "**/*.test.ts" -i "**/*.spec.ts"

# Combine include and ignore patterns
ctx index --watch -p "src/" -i "src/generated/"
```

Watch mode applies the same filters as a normal index run: `.gitignore`, `.contextignore`, and the
built-in ignores.

## Part 5: Project configuration

### Create .contextignore

Put project-specific exclusions in `.contextignore`:

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

### What ctx ignores by default

ctx excludes:
- everything in `.gitignore`
- 164 built-in patterns (binaries, `node_modules`, build outputs, lock files, and similar)
- files matching `.contextignore`

### Ignore patterns on the command line

```bash
ctx -i "*.test.ts" -i "fixtures/" src/
```

## Part 6: Smart context selection

`ctx smart` picks files for a task description:

```bash
# Describe the task
ctx smart "add user authentication" --max-tokens 8000

# Preview the selection
ctx smart "fix login bug" --dry-run

# Show why each file was selected
ctx smart "refactor parser" --explain
```

`ctx smart` needs an index (`ctx index`) and embeddings (`ctx embed`). If embeddings are missing
or incomplete, it exits with code 2 and tells you to run `ctx embed`. If they come from another
provider, model, or dimension, it exits 2 and tells you to run
`ctx embed --force --provider <provider>`. `--max-tokens` (default 8000) is a hard limit on the
whole rendered output, including the tree and format wrappers. A top-ranked file larger than the
budget is left out unless you pass `--include-oversized-top`. In v0.4.0, `--max-tokens` counted
file contents only and the top file was always included.

## Part 7: Diff-aware context

`ctx diff` builds context around code changes:

```bash
# Changes since the previous commit (default revision: HEAD~1)
ctx diff

# Changes against main
ctx diff main

# Only staged changes
ctx diff --staged

# Add a change summary (on stderr)
ctx diff --summary
```

## Part 8: Pull request review context

`ctx review` builds context for a GitHub pull request:

```bash
# Review PR #123
ctx review 123

# Include PR comments
ctx review 123 --include-comments

# Add a change summary
ctx review 123 --summary
```

`ctx review` needs the GitHub CLI (`gh`), installed and authenticated.

## Part 9: Code quality audit

```bash
# Full quality report
ctx audit

# Fail when the score is below 7.0
ctx audit --min-score 7.0

# JSON output
ctx audit --output json
```

## Part 10: Interactive shell

```bash
ctx shell
```

Shell commands:
- `find <pattern>`: find symbols
- `search <query>`: search symbols
- `callers <fn>`: show callers
- `callees <fn>`: show callees
- `source <symbol>`: show source
- `explain <symbol>`: show symbol details
- `impact <symbol>`: impact analysis
- `stats`: codebase statistics
- `help`: list all commands

## Part 11: MCP server (Claude Desktop)

The MCP server needs a build with the `mcp` feature (see [From source](#from-source)):

```bash
# Build with MCP support
cargo build --release --features mcp

# Run the MCP server
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

## Common workflows

### Quick LLM context

```bash
# Context for the code you are working on
ctx "src/auth/**/*.ts" | pbcopy

# Paste it into a chat
```

### Learn a new codebase

```bash
# Index the codebase
ctx index

# Overview
ctx query stats

# Look for entry points
ctx search "main"
ctx search "app"

# Trace the call flow
ctx query graph main --depth 4
```

### Refactor safely

```bash
# Before changing a function, check its impact
ctx query impact authenticate --depth 5

# See who calls it
ctx query callers authenticate

# After editing, reindex
ctx index
```

### Prepare a code review

```bash
# Functions with high fan-out
ctx complexity --warnings-only

# Near-duplicate functions (structural match, renames ignored)
ctx duplicates --threshold 0.9

# File dependencies
ctx graph --by-file --output mermaid
```

### Context for a task

```bash
# Let ctx select files for the task
ctx smart "add caching to the database layer" --max-tokens 10000 | pbcopy

# Preview the selection first
ctx smart "fix authentication bug" --dry-run --explain
```

### Pull request review

```bash
# Context for reviewing a PR
ctx review 42 --summary --include-comments

# Only the changed files
ctx review 42 --changes-only
```

### CI

```bash
# Quality gate in a CI pipeline
ctx audit --min-score 7.0 --output json > quality-report.json

# Pre-commit hook
ctx audit --min-score 7.0 || exit 1
```

`ctx audit --incremental` is only partly incremental. It limits the duplication, coverage, and
naming checks to staged, unstaged, and untracked files; complexity and modularity still read the
whole index. With no changed files it reports 10.0 without auditing anything.

## Next steps

- [Context generation](context-generation.md): output formats and filtering options
- [Code intelligence](code-intelligence.md): indexing and queries in depth
- [Configuration](configuration.md): `.contextignore` and environment variables
- [Language support](language-support.md): what ctx extracts from each language
- [Architecture](architecture.md): how ctx works inside
