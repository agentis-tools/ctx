# Context generation

Running `ctx` without a subcommand prints the selected files as one formatted document for an LLM. This page covers file selection, output formats, and token budgets.

## Basic usage

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

## File selection

### Glob patterns

ctx accepts standard glob patterns:

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

### Direct paths

Name files and directories directly:

```bash
ctx src/ lib/ Cargo.toml README.md
```

### Combining patterns and paths

```bash
ctx src/ "tests/**/*.test.ts" package.json
```

### How patterns combine

ctx selects the union of all patterns:
1. A literal path (no wildcards) includes every file under it. Glob patterns do not filter it.
2. Glob patterns match paths relative to the current directory.
3. Without any pattern, ctx selects everything under the current directory.

```bash
# Every file under src/, plus every .ts file anywhere in the project
ctx src/ "**/*.ts"

# Only .ts files under src/
ctx "src/**/*.ts"
```

ctx also skips files that look binary (a null byte in the first 8 KB). If include patterns match no
files, ctx prints a warning.

## Output formats

### XML (default)

Each file is an element with its name and path as attributes.

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

XML marks where each file starts and ends without depending on the file's own content, and tools
can parse it.

### Markdown

For chat interfaces that render Markdown.

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

Each file is a heading followed by a fenced code block tagged with the file extension, so chat
interfaces highlight the syntax.

### Plain text

No markup beyond a header line per file. `--format text` is an alias.

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

Plain text adds the fewest tokens and suits piping to other tools.

### JSON

```bash
ctx --format json src/
```

With the default streaming output, ctx writes newline-delimited JSON: a tree object first, then one
object per file:

```json
{"type":"tree","content":"project/\n├── src/\n..."}
{"name":"main.rs","path":"/src/main.rs","content":"fn main() {\n    println!(\"Hello!\");\n}"}
{"name":"lib.rs","path":"/src/lib.rs","content":"pub fn greet() -> String {\n    \"Hello\".to_string()\n}"}
```

With `--no-stream`, ctx writes one object: `{"tree": "...", "files": [{"name", "path", "content"}, ...]}`.
This is the context document itself, not the `--json` result envelope that other commands use.

## Ignore rules

ctx applies three kinds of ignore rules.

### .gitignore (default)

ctx follows `.gitignore`:

```bash
# Include gitignored files
ctx --no-gitignore src/
```

### .contextignore

Put project-specific exclusions in `.contextignore`, using `.gitignore` syntax:

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

# Documentation, if the task does not need it
docs/
*.md

# Large data files
*.sql
*.csv
data/
```

### Built-in ignores (164 patterns)

ctx excludes common non-source files, including:

Version control:
- `.git/`, `.svn/`, `.hg/`, `.bzr/`

Dependencies:
- `node_modules/`, `vendor/`, `Pods/`, `target/`, `.cargo/`

Build outputs:
- `dist/`, `build/`, `out/`, `.next/`, `.nuxt/`, `coverage/`

Binary files:
- Images: `*.png`, `*.jpg`, `*.gif`, `*.ico`, `*.webp`, `*.svg`
- Media: `*.mp3`, `*.mp4`, `*.avi`, `*.mov`
- Executables: `*.exe`, `*.dll`, `*.so`, `*.dylib`
- Archives: `*.zip`, `*.tar`, `*.gz`, `*.rar`

Lock files:
- `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`
- `Cargo.lock`, `Gemfile.lock`, `poetry.lock`

Environment and secrets:
- `.env`, `.env.*`, `*.pem`, `*.key`, `*.crt`

IDE and editor:
- `.vscode/`, `.idea/`, `*.swp`, `*.swo`

Cache and temporary files:
- `.cache/`, `tmp/`, `temp/`, `*.tmp`

[Configuration](configuration.md#built-in-ignores) lists more. Turn them off with:
```bash
ctx --no-default-ignores src/
```

### Command-line ignores

Add patterns for one run:

```bash
ctx -i "*.test.ts" -i "fixtures/" src/
```

`-i` can be repeated and adds to the other ignore rules.

### Disabling ignores

```bash
# Disable .gitignore only
ctx --no-gitignore src/

# Disable built-in patterns only
ctx --no-default-ignores src/

# Include everything (except .contextignore)
ctx --no-gitignore --no-default-ignores src/
```

ctx always applies `.contextignore` when it exists.

## Other options

### Show file sizes

Show file sizes in the project tree:

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

### Skip the project tree

Leave out the tree to save tokens:

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

### Buffered or streaming output

By default, ctx writes each file as soon as it reads it. To build the whole document before
writing anything:

```bash
ctx --no-stream src/
```

### Statistics

Print statistics to stderr when done:

```bash
ctx --stats src/
```

Output:
```
Generated context: 42 files, 156.3 KB, ~38.9k tokens in 23ms
```

The token figure is an estimate (output bytes divided by 4). Statistics go to stderr, so piping
copies only the context:
```bash
ctx --stats src/ | pbcopy  # stats on screen, context on the clipboard
```

### Token counting

Count the tokens of a selection with the tokenizer, without printing file contents:

```bash
ctx --count-only src/
```

Output:
```
Files: 42
Characters (UTF-8): 155021
Tokens (cl100k_base): 38410
```

Use it to check whether a selection fits a model's context window.

### Token budgets

With `--max-tokens`, ctx keeps files in path order while their contents fit the budget and leaves
out any file that would exceed it. It never truncates a file, and it prints the number of omitted
files to stderr. The budget counts file contents only, not the tree or format markup:

```bash
# File contents under 8000 tokens
ctx --max-tokens 8000 src/
```

`ctx smart`, `ctx diff`, and `ctx review` have their own budget options. For `ctx smart`,
`--max-tokens` limits the whole rendered output; see [ctx smart](commands/smart.md). Passing
`--max-tokens` to any other subcommand is an error (exit 2); in v0.4.0 it was silently ignored.

### Tokenizer encoding

ctx counts tokens with a tiktoken-compatible encoding. The default is `cl100k_base`; change it
with `--encoding`:

```bash
# Use the o200k_base encoding (GPT-4o family)
ctx --encoding o200k_base --count-only src/
```

The encodings are `cl100k_base` (default), `o200k_base`, and `p50k_base`.

## Common workflows

### Quick context for an LLM

```bash
# macOS
ctx src/ | pbcopy

# Linux
ctx src/ | xclip -selection clipboard

# WSL
ctx src/ | clip.exe
```

### Save to a file

```bash
ctx src/ > context.xml
ctx --format md src/ > CONTEXT.md
```

### Context for one feature

```bash
# Just auth-related files
ctx "src/auth/**/*.ts" "src/middleware/auth.ts" | pbcopy

# API routes and their types
ctx "src/routes/**/*.ts" "src/types/**/*.ts" | pbcopy
```

### Most of the project

```bash
# Everything except tests
ctx -i "**/*.test.ts" -i "__tests__/" src/

# Core source only
ctx src/lib src/core --no-tree
```

### Minimal context

```bash
# No tree, specific files only
ctx --no-tree src/api.ts src/types.ts src/utils.ts
```

### Compare two contexts

```bash
ctx src/ > before.xml
# make changes
ctx src/ > after.xml
diff before.xml after.xml
```

### Count tokens

```bash
# Exact token count (no file contents printed)
ctx --count-only src/

# Rough proxies without the tokenizer
ctx src/ | wc -w   # word count
ctx src/ | wc -c   # character count
```

### Pipe to other tools

```bash
# Syntax highlighting
ctx src/main.rs | bat --language=xml

# Search within context
ctx src/ | grep -A5 "handleAuth"
```

## Output format summary

| Format | Flag | Use |
|--------|------|-----|
| XML | `--format xml` (default) | Most LLMs; explicit file boundaries |
| Markdown | `--format md` or `--format markdown` | Chat interfaces that render Markdown |
| Plain | `--format plain` or `--format text` | Piping to tools; fewest extra tokens |
| JSON | `--format json` | Programs that read the files |

## CLI reference

```
ctx [OPTIONS] [PATTERNS]...

Arguments:
  [PATTERNS]...  File patterns or paths to include (glob syntax supported)
                 Examples: "src/**/*.rs", "*.ts", "src/"
                 Default: "." (current directory)

Options:
  -f, --format <FORMAT>     Output format [default: xml]
                            Possible values: xml, markdown, md, plain, text, json
      --no-gitignore        Disable .gitignore pattern matching
  -i, --ignore <PATTERN>    Additional ignore patterns (can be repeated)
      --no-default-ignores  Disable built-in ignore patterns
      --show-sizes          Show file sizes in project tree
      --no-tree             Disable project tree in output
      --no-stream           Buffer all output before printing
      --stats               Print stats (file count, size, time, token estimate)
      --count-only          Count tokens only; do not print file contents
      --max-tokens <N>      Leave out whole files to keep file contents within N tokens
      --encoding <ENCODING> Tokenizer encoding [default: cl100k_base]
                            Possible values: cl100k_base, o200k_base, p50k_base
  -h, --help                Print help
  -V, --version             Print version
```
