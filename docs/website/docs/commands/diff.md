---
id: diff
title: ctx diff
sidebar_position: 2
---

# ctx diff

Generate context for changed files and the code they affect.

## Synopsis

```bash
ctx diff [REVISION] [PATTERNS]... [OPTIONS]
```

## Description

`ctx diff` finds the files that changed relative to a git revision and adds the symbols and files those changes affect. Use it to:

- get context for reviewing a pull request or commit,
- see what your changes affect,
- check changes before you commit them.

Optional positional patterns restrict both the changed files and the graph-expanded
context. ctx ORs literal files, directories, and globs together. A rename matches
when either the old or the new path is in scope, and deletions stay visible in the
change summary. With no patterns, `.` includes all changed paths.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `REVISION` | Git revision to compare the working tree against | `HEAD~1` |
| `--max-tokens <N>` | Maximum tokens in output | 8000 |
| `--depth <N>` | Call graph depth for related symbols | 1 |
| `--changes-only` | Only include changed files, without related context | false |
| `--staged` | Compare staged changes instead of a revision | false |
| `--summary` | Print a change summary on stderr | false |
| `-f, --format <FORMAT>` | Output format (xml, markdown, md, plain, json) | xml |
| `--show-sizes` | Show file sizes in the project tree | false |
| `--no-tree` | Disable the project tree in output | false |
| `--count-only` | Count the selected, budgeted files instead of streaming contents | false |
| `--encoding <ENCODING>` | Tokenizer for budgeting and counting (`cl100k_base`, `o200k_base`, or `p50k_base`) | cl100k_base |
| `--stats` | Print count timing to stderr with `--count-only` | false |

## Examples

### Changes since the previous commit

```bash
# Compare the working tree with HEAD~1 (the default)
ctx diff
```

### Compare with a branch

```bash
# Changes compared to main branch
ctx diff main

# Changes in feature branch
ctx diff origin/main

# Review only Rust sources and the public configuration contract
ctx diff origin/main "src/**/*.rs" docs/configuration.md
```

### Staged changes

```bash
# Only the changes you have staged
ctx diff --staged
```

### Token-limited output

```bash
# Limit output for smaller context windows, or measure that selected pack
ctx diff --max-tokens 4000
ctx diff --max-tokens 4000 --count-only --encoding o200k_base
```

### Summary mode

```bash
# Count selected files on stdout while retaining the summary on stderr
ctx diff --summary --changes-only --no-tree --count-only
```

`--summary` adds diagnostics on stderr and does not suppress context by itself.
`--count-only` suppresses streamed contents and writes the count summary to
stdout. `--stats` adds timing on stderr. The same `--encoding` drives both
budget selection and the reported count.

Summary output:
```
Changes in main: 3 files changed

Changed files:
  src/auth.rs (+12 -3)
  src/cache.rs (+40 -0)
  src/legacy.rs (deleted)

Affected symbols:
  authenticate (function) in src/auth.rs
  Cache (struct) in src/cache.rs

Context: 5 files (3120 tokens)
```

### Deeper dependency analysis

```bash
# Follow the call graph further for related context
ctx diff main --depth 4
```

## How it works

1. ctx runs `git diff --name-status` to find added, modified, renamed, and deleted files.
2. It parses the hunks to find which symbols changed.
3. It follows the call graph to related symbols, up to `--depth`.
4. It writes the changed files plus related context, within `--max-tokens`.

## Output

The output includes:
- changed files with their full contents,
- related files containing symbols affected by the changes (unless you pass `--changes-only`),
- no more than `--max-tokens` in total.

## Pull request context

`ctx review` builds on `diff` to fetch a GitHub pull request and generate its
context. It requires the GitHub CLI (`gh`), installed and authenticated:

```bash
# Get context for a GitHub PR
ctx review 123
```

`ctx review` accepts `--repo <owner/name>`, `--include-comments`, `--max-tokens`,
`--depth`, `--changes-only`, and `--summary`.

## Tips

- Run with `--summary` first to see the scope of the change.
- Increase `--depth` for refactoring tasks.
- Combine with `--format markdown` for PR descriptions.

## See also

- [ctx smart](./smart.md): task-based file selection
- [Impact analysis](../code-intelligence.md#impact-analysis): what a change affects
