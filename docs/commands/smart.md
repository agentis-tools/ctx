# ctx smart

Intelligently select files relevant to a task description using embeddings and call graph analysis.

## Synopsis

```bash
ctx smart <TASK> [PATTERNS]... [OPTIONS]
```

## Description

The `smart` command analyzes your task description and automatically selects the most relevant files from your codebase. It combines:

- **Semantic search** - Finds symbols related to your task using embeddings
- **Call graph expansion** - Includes callers and callees of matched symbols
- **Token budgeting** - Fits selection within your token limit

Optional positional patterns scope the entire selection pipeline. Literal
files, directories, and globs are ORed together; semantic matches and
call-graph expansion both stay within that scope. With no patterns, `.` selects
the whole repository.

## Prerequisites

Before using `ctx smart`, you must:

1. Index your codebase: `ctx index`
2. Generate embeddings: `ctx embed`

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--max-tokens <N>` | Maximum tokens in output | 8000 |
| `--depth <N>` | Call graph expansion depth | 2 |
| `--top <N>` | Top semantic matches to consider | 10 |
| `--explain` | Show why each file was selected | false |
| `--dry-run` | Show selection without file contents | false |

## Examples

### Basic Usage

```bash
# Select files for adding a new feature
ctx smart "add user authentication"

# Review files for fixing a bug
ctx smart "fix the login timeout issue"

# Search and expand only within Rust source and one contract file
ctx smart "change request validation" "src/**/*.rs" tests/contracts.rs
```

### With Token Budget

```bash
# Limit to 4000 tokens for smaller context
ctx smart "implement caching" --max-tokens 4000
```

### Explain Selection

```bash
# See why each file was selected
ctx smart "add logging" --explain
```

Output:
```
Selected 5 files (3,245 tokens):

src/logger.rs (1,200 tokens)
  - semantic: 0.92 - "logger" matches task
  - calls: referenced by main.rs

src/config.rs (845 tokens)
  - semantic: 0.78 - "configuration" related
  - callee: called by logger.rs
...
```

### Dry Run

```bash
# Preview selection without outputting file contents
ctx smart "refactor error handling" --dry-run
```

Output:
```
Would select 4 files (2,891 tokens):
  src/error.rs       1,245 tokens
  src/handlers.rs      892 tokens
  src/validation.rs    512 tokens
  src/response.rs      242 tokens
```

### Deep Call Graph Analysis

```bash
# Expand call graph further for complex changes
ctx smart "optimize database queries" --depth 4
```

## How It Works

1. **Semantic Search**: Embeds your task description and finds matching symbols
2. **Graph Expansion**: For each matched symbol, follows call edges up to `--depth` levels
3. **Scoring**: Ranks files by semantic similarity and call graph relevance
4. **Selection**: Picks highest-scoring files that fit within `--max-tokens`

## Output

By default, outputs in the same format as `ctx` (XML, Markdown, etc.):

```bash
# Get markdown output
ctx smart "add tests" --format markdown
```

## Tips

- Use specific task descriptions for better results
- Start with `--dry-run` to preview selection
- Use `--explain` to understand and tune selection
- Increase `--depth` for changes with wide impact
- Decrease `--max-tokens` when context window is limited

## See Also

- [ctx embed](../code-intelligence.md#embeddings) - Generate embeddings
- [ctx query callers](../code-intelligence.md#call-graph) - Manual call graph queries
- [ctx diff](./diff.md) - Select files based on git changes
