---
id: diff
title: ctx diff
sidebar_position: 2
---

# ctx diff

Generate context for changed files with their related dependencies.

## Synopsis

```bash
ctx diff [REVISION] [PATTERNS]... [OPTIONS]
```

## Description

The `diff` command identifies changed files compared to a git reference and includes context about the symbols and dependencies affected by those changes. This is useful for:

- **Code review** - Get context for reviewing a PR or commit
- **Change impact** - Understand what's affected by your changes
- **Pre-commit** - Verify changes before committing

Optional positional patterns restrict both changed files and graph-expanded
context. Literal files, directories, and globs are ORed together. Renames match
when either the old or new path is in scope; deletions remain visible in the
change summary. With no patterns, `.` includes all changed paths.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `REVISION` | Git reference to compare against | `HEAD~1` |
| `--max-tokens <N>` | Maximum tokens in output | 8000 |
| `--depth <N>` | Depth for finding related symbols | 1 |
| `--changes-only` | Only include changed files, not related context | false |
| `--staged` | Compare staged changes instead of a revision | false |
| `--summary` | Include a change summary on stderr | false |
| `-f, --format <FORMAT>` | Output format (xml, markdown, md, plain, json) | xml |
| `--show-sizes` | Show file sizes in the project tree | false |
| `--no-tree` | Disable the project tree in output | false |
| `--count-only` | Count selected, budgeted files instead of streaming contents | false |
| `--encoding <ENCODING>` | Tokenizer used for budgeting and counting (`cl100k_base`, `o200k_base`, or `p50k_base`) | cl100k_base |
| `--stats` | Print count timing to stderr with `--count-only` | false |

## Examples

### Changes Since Last Commit

```bash
# See what changed since the previous commit (default: HEAD~1)
ctx diff
```

### Compare with Branch

```bash
# Changes compared to main branch
ctx diff main

# Changes in feature branch
ctx diff origin/main

# Review only Rust sources and the public configuration contract
ctx diff origin/main "src/**/*.rs" docs/configuration.md
```

### Review a Specific Commit

```bash
# Changes in the last commit
ctx diff HEAD~1
```

### Staged Changes

```bash
# Only the changes you have staged
ctx diff --staged
```

### Token-Limited Output

```bash
# Limit output for smaller context windows, or measure that selected pack
ctx diff --max-tokens 4000
ctx diff --max-tokens 4000 --count-only --encoding o200k_base
```

### Summary Mode

```bash
# Count selected files on stdout while retaining the summary on stderr
ctx diff --summary --changes-only --no-tree --count-only
```

`--summary` adds diagnostics on stderr and does not itself suppress context.
`--count-only` suppresses streamed contents and writes the count summary to
stdout; `--stats` adds timing on stderr. The same `--encoding` drives both
budget selection and the reported count.

Output:
```
Changed files (5):
  M src/auth.rs       - 3 symbols affected
  M src/handlers.rs   - 2 symbols affected
  A src/cache.rs      - new file
  D src/legacy.rs     - removed

Related symbols:
  - authenticate (src/auth.rs:42) - modified
  - handleLogin (src/handlers.rs:15) - calls authenticate
  - UserSession (src/models.rs:8) - used by authenticate
```

### Deep Dependency Analysis

```bash
# Find more related context
ctx diff main --depth 4
```

## How It Works

1. **Detect Changes**: Uses `git diff` to find modified, added, and deleted files
2. **Parse Hunks**: Identifies which lines/symbols changed
3. **Find Related**: Follows call graph to find dependent symbols
4. **Generate Context**: Outputs changed files plus relevant context

## Output

The output includes:
- Changed files with their full contents
- Related files containing symbols affected by the changes (unless `--changes-only` is set)
- Context limited to `--max-tokens`

## Integration with Code Review

The `ctx review` command builds on `diff` to fetch and review a GitHub pull
request. It requires the GitHub CLI (`gh`) to be installed and authenticated:

```bash
# Get context for a GitHub PR
ctx review 123
```

## Tips

- Use `--summary` first to understand scope of changes
- Increase `--depth` for refactoring tasks
- Combine with `--format markdown` for PR descriptions
- Use in CI to generate change documentation

## See Also

- [ctx smart](./smart.md) - Task-based file selection
- [Impact Analysis](../code-intelligence.md#impact-analysis) - Understand what a change affects
