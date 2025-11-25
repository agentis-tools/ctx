# context

A fast CLI tool that generates AI-ready context from your codebase. Select files using glob patterns, and get formatted output perfect for pasting into ChatGPT, Claude, or any LLM. Outputs to stdout for easy piping to clipboard or other tools.

## Features

- **Glob pattern support** - Select files with patterns like `"src/**/*.rs"` or `"**/*.ts"`
- **Smart ignore system** - Respects `.gitignore` and supports `.contextignore` for project-specific exclusions
- **Built-in filtering** - Automatically excludes binary files, `node_modules`, build artifacts, and 170+ common non-source patterns
- **Multiple output formats** - XML (default), Markdown, or plain text
- **Project tree visualization** - Includes an ASCII tree showing file structure
- **Streaming output** - Files are output as they're processed (default behavior)
- **Pipeable output** - Works seamlessly with `pbcopy`, `xclip`, or any Unix tool

## Installation

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
# Binary at ./target/release/context
```

## Usage

```bash
# All files in current directory
context

# Specific files or directories
context src/ Cargo.toml README.md

# Glob patterns
context "src/**/*.rs"
context "**/*.ts" "**/*.tsx"

# Different output formats
context --format xml        # Default
context --format markdown   # Or --format md
context --format plain

# Copy to clipboard (macOS)
context src/ | pbcopy

# Copy to clipboard (Linux)
context src/ | xclip -selection clipboard

# Ignore specific patterns
context -i "*.test.ts" -i "fixtures/" src/

# Show file sizes in tree
context --show-sizes

# Include gitignored files
context --no-gitignore

# Disable built-in ignores
context --no-default-ignores

# Skip the project tree
context --no-tree

# Buffer all output before printing (instead of streaming)
context --no-stream
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
│   ├── main.rs
│   └── lib.rs
└── Cargo.toml
\`\`\`

## /src/main.rs

\`\`\`rs
fn main() {
    println!("Hello, world!");
}
\`\`\`
```

### Plain

```
=== PROJECT TREE ===

my-project/
├── src/
│   ├── main.rs
│   └── lib.rs
└── Cargo.toml

=== /src/main.rs ===

fn main() {
    println!("Hello, world!");
}
```

## Ignore System

The tool uses a three-tier ignore system:

1. **`.gitignore`** - Respected by default (disable with `--no-gitignore`)
2. **`.contextignore`** - Project-specific ignores, same syntax as `.gitignore`
3. **Built-in patterns** - Common non-source files (disable with `--no-default-ignores`)

### Built-in Ignores

Automatically excludes:
- Version control (`.git/`, `.svn/`)
- Dependencies (`node_modules/`, `vendor/`, `target/`)
- Build outputs (`dist/`, `build/`, `.next/`)
- Binary files (`*.png`, `*.jpg`, `*.exe`, `*.dll`)
- Lock files (`package-lock.json`, `yarn.lock`, `Cargo.lock`)
- Environment files (`.env`, `*.pem`, `*.key`)
- IDE directories (`.vscode/`, `.idea/`)
- And 150+ more patterns

## CLI Reference

```
Generate formatted context for AI assistants

Usage: context [OPTIONS] [PATTERNS]...

Arguments:
  [PATTERNS]...  File patterns or paths to include (glob syntax supported)
                 [default: .]

Options:
  -f, --format <FORMAT>    Output format [default: xml]
                           [possible values: xml, markdown, md, plain]
      --no-gitignore       Disable .gitignore pattern matching
  -i, --ignore <PATTERN>   Additional ignore patterns (can be repeated)
      --no-default-ignores Disable built-in ignore patterns
      --show-sizes         Show file sizes in project tree
      --no-tree            Disable project tree in output
      --no-stream          Buffer all output before printing (default: streaming)
      --stats              Print stats (file count, total size, time taken)
  -h, --help               Print help
  -V, --version            Print version
```

## License

MIT
