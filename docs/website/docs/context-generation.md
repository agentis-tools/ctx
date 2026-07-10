---
id: context-generation
title: Context Generation
sidebar_position: 3
---

# Context Generation

The primary use case for ctx is generating formatted context for LLMs. This page covers all the options for selecting files and formatting output.

## Basic Usage

```bash
# All files in current directory
ctx

# Specific directory
ctx src/

# Multiple directories
ctx src/ lib/ tests/

# Copy to clipboard (macOS)
ctx src/ | pbcopy
```

## File Selection

### Glob Patterns

ctx supports standard glob patterns:

```bash
# All TypeScript files
ctx "**/*.ts"

# All files in src/
ctx "src/**/*"

# Multiple patterns
ctx "src/**/*.rs" "tests/**/*.rs"

# Specific extensions
ctx "**/*.{ts,tsx,js,jsx}"

# Single directory level
ctx "src/*.rs"  # Only top-level .rs files in src/

# Recursive
ctx "src/**/*.rs"  # All .rs files in src/ and subdirectories
```

### Direct Paths

Specify files and directories directly:

```bash
ctx src/ lib/ Cargo.toml README.md
```

### Combining Patterns and Paths

```bash
ctx src/ "tests/**/*.test.ts" package.json
```

### Pattern Priority

When you specify patterns:
1. **Literal paths** (no wildcards) are used as starting directories
2. **Glob patterns** filter files within those directories
3. If only globs are provided, search starts from current directory

```bash
# Start from src/, filter to .ts files
ctx src/ "**/*.ts"

# Start from current dir, find all .rs files
ctx "**/*.rs"
```

## Output Formats

### XML (Default)

Best for most LLMs. Clear structure with file paths as attributes.

```bash
ctx --format xml src/
# or just
ctx src/
```

Output:
```xml
<context>
<project_tree>
project/
├── src/
│   ├── main.rs
│   └── lib.rs
└── Cargo.toml
</project_tree>
<project_files>
<file name="main.rs" path="/src/main.rs">
fn main() {
    println!("Hello!");
}
</file>
<file name="lib.rs" path="/src/lib.rs">
pub fn greet() -> String {
    "Hello".to_string()
}
</file>
</project_files>
</context>
```

**Why XML?**
- Unambiguous structure
- Clear file boundaries
- Works well with instruction-following models
- Easy to parse programmatically

### Markdown

Good for chat interfaces that render markdown.

```bash
ctx --format markdown src/
# or
ctx --format md src/
```

Output:
````markdown
# Project Context

## Project Tree

```
project/
├── src/
│   ├── main.rs
│   └── lib.rs
└── Cargo.toml
```

## /src/main.rs

```rs
fn main() {
    println!("Hello!");
}
```

## /src/lib.rs

```rs
pub fn greet() -> String {
    "Hello".to_string()
}
```
````

**Why Markdown?**
- Renders nicely in chat UIs
- Syntax highlighting in code blocks
- Familiar format for developers

### Plain Text

Simple format without markup.

```bash
ctx --format plain src/
```

Output:
```
=== PROJECT TREE ===

project/
├── src/
│   ├── main.rs
│   └── lib.rs
└── Cargo.toml

=== /src/main.rs ===

fn main() {
    println!("Hello!");
}

=== /src/lib.rs ===

pub fn greet() -> String {
    "Hello".to_string()
}
```

**Why Plain Text?**
- Minimal overhead
- Universal compatibility
- Good for piping to other tools

## Ignore System

ctx uses a three-tier ignore system to filter out irrelevant files.

### Tier 1: .gitignore (Default)

Automatically respects your `.gitignore` file:

```bash
# Include gitignored files
ctx --no-gitignore src/
```

### Tier 2: .contextignore

Create a `.contextignore` file for project-specific exclusions. Uses the same syntax as `.gitignore`:

```gitignore
# Test files
**/*.test.ts
**/*.spec.ts
__tests__/

# Fixtures and mocks
fixtures/
__mocks__/

# Generated files
*.generated.ts
*.pb.go
generated/

# Documentation (might not need for AI context)
docs/
*.md

# Large data files
*.sql
*.csv
data/
```

### Tier 3: Built-in Ignores (170+ patterns)

ctx automatically excludes common non-source patterns:

**Version Control:**
- `.git/`, `.svn/`, `.hg/`, `.bzr/`

**Dependencies:**
- `node_modules/`, `vendor/`, `Pods/`, `target/`, `.cargo/`

**Build Outputs:**
- `dist/`, `build/`, `out/`, `.next/`, `.nuxt/`, `coverage/`

**Binary Files:**
- Images: `*.png`, `*.jpg`, `*.gif`, `*.ico`, `*.webp`, `*.svg`
- Media: `*.mp3`, `*.mp4`, `*.avi`, `*.mov`
- Executables: `*.exe`, `*.dll`, `*.so`, `*.dylib`
- Archives: `*.zip`, `*.tar`, `*.gz`, `*.rar`

**Lock Files:**
- `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`
- `Cargo.lock`, `Gemfile.lock`, `poetry.lock`

**Environment & Secrets:**
- `.env`, `.env.*`, `*.pem`, `*.key`, `*.crt`

**IDE & Editor:**
- `.vscode/`, `.idea/`, `*.swp`, `*.swo`

**Cache & Temp:**
- `.cache/`, `tmp/`, `temp/`, `*.tmp`

Disable with:
```bash
ctx --no-default-ignores src/
```

### Command-Line Ignores

Add patterns on the fly:

```bash
ctx -i "*.test.ts" -i "fixtures/" src/
```

Multiple `-i` flags are combined with other ignore sources.

### Disabling Ignores

```bash
# Disable .gitignore only
ctx --no-gitignore src/

# Disable built-in patterns only
ctx --no-default-ignores src/

# Include everything (except .contextignore)
ctx --no-gitignore --no-default-ignores src/
```

Note: `.contextignore` is always respected if present.

## Additional Options

### Show File Sizes

Display file sizes in the project tree:

```bash
ctx --show-sizes src/
```

Output:
```
project/
├── src/
│   ├── main.rs (1.2 KB)
│   └── lib.rs (856 B)
└── Cargo.toml (423 B)
```

### Skip Project Tree

Omit the tree visualization to save space:

```bash
ctx --no-tree src/
```

Output (XML):
```xml
<context>
<project_files>
<file name="main.rs" path="/src/main.rs">
fn main() {
    println!("Hello!");
}
</file>
</project_files>
</context>
```

### Buffered vs Streaming Output

By default, ctx streams output as files are processed. This is efficient for piping.

To buffer everything first (e.g., for error checking before output):

```bash
ctx --no-stream src/
```

### Statistics

Print stats to stderr after completion:

```bash
ctx --stats src/
```

Output:
```
Generated context: 42 files, 156.3 KB, ~38.9k tokens in 23ms
```

Stats go to stderr, so they don't interfere with piping:
```bash
ctx --stats src/ | pbcopy  # Stats shown, only content copied
```

The statistics line includes a token estimate alongside the file count, size, and elapsed time.

### Token Counting

Count the tokens a selection would produce without printing any file contents:

```bash
ctx --count-only src/
```

This is useful for checking whether a selection fits your model's context window before generating the full output.

### Token Budgeting

Fit context to a token budget. When `--max-tokens` is set, ctx omits whole files that would push the output over the budget. Files are dropped as a unit; they are never truncated:

```bash
# Keep the output under 8000 tokens
ctx --max-tokens 8000 src/
```

### Tokenizer Encoding

Token counts are computed with a tiktoken-compatible encoding. The default is `cl100k_base`; override it with `--encoding`:

```bash
# Use the o200k_base encoding (GPT-4o family)
ctx --encoding o200k_base --count-only src/
```

Available encodings: `cl100k_base` (default), `o200k_base`, `p50k_base`.

## Common Workflows

### Quick Context for LLM

```bash
# macOS
ctx src/ | pbcopy

# Linux
ctx src/ | xclip -selection clipboard

# WSL
ctx src/ | clip.exe
```

### Save to File

```bash
ctx src/ > context.xml
ctx --format md src/ > CONTEXT.md
```

### Targeted Context for a Feature

```bash
# Just auth-related files
ctx "src/auth/**/*.ts" "src/middleware/auth.ts" | pbcopy

# API routes and their types
ctx "src/routes/**/*.ts" "src/types/**/*.ts" | pbcopy
```

### Full Project Context

```bash
# Everything except tests
ctx -i "**/*.test.ts" -i "__tests__/" src/

# Core source only
ctx src/lib src/core --no-tree
```

### Minimal Context (Save Tokens)

```bash
# No tree, specific files only
ctx --no-tree src/api.ts src/types.ts src/utils.ts
```

### Compare Contexts

```bash
ctx src/ > before.xml
# make changes
ctx src/ > after.xml
diff before.xml after.xml
```

### Count Tokens

```bash
# Exact token count (no file contents printed)
ctx --count-only src/

# Rough proxies without the tokenizer
ctx src/ | wc -w   # word count
ctx src/ | wc -c   # character count
```

### Pipe to Other Tools

```bash
# Syntax highlighting
ctx src/main.rs | bat --language=xml

# Search within context
ctx src/ | grep -A5 "handleAuth"
```

## Output Format Summary

| Format | Flag | Use Case |
|--------|------|----------|
| XML | `--format xml` (default) | Most LLMs, clear structure |
| Markdown | `--format md` / `--format markdown` | Chat UIs, readable output |
| Plain | `--format plain` | Simple tools, minimal overhead |

## CLI Reference

```
ctx [OPTIONS] [PATTERNS]...

Arguments:
  [PATTERNS]...  File patterns or paths to include (glob syntax supported)
                 Examples: "src/**/*.rs", "*.ts", "src/"
                 Default: "." (current directory)

Options:
  -f, --format <FORMAT>     Output format [default: xml]
                            Possible values: xml, markdown, md, plain, json
      --no-gitignore        Disable .gitignore pattern matching
  -i, --ignore <PATTERN>    Additional ignore patterns (can be repeated)
      --no-default-ignores  Disable built-in ignore patterns
      --show-sizes          Show file sizes in project tree
      --no-tree             Disable project tree in output
      --no-stream           Buffer all output before printing
      --stats               Print stats (file count, size, time, token estimate)
      --count-only          Count tokens only; do not print file contents
      --max-tokens <N>      Omit whole files to fit a token budget (never truncates a file)
      --encoding <ENCODING> Tokenizer encoding [default: cl100k_base]
                            Possible values: cl100k_base, o200k_base, p50k_base
  -h, --help                Print help
  -V, --version             Print version
```
