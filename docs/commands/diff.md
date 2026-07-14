# ctx diff

Generate context for changed files with their related dependencies.

## Synopsis

```bash
ctx diff [REF] [PATTERNS]... [OPTIONS]
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
| `REF` | Git reference to compare against | HEAD |
| `--max-tokens <N>` | Maximum tokens in output | 8000 |
| `--depth <N>` | Depth for finding related symbols | 2 |
| `--summary` | Show only summary, not file contents | false |

## Examples

### Changes Since Last Commit

```bash
# See what changed since HEAD
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

### Token-Limited Output

```bash
# Limit output for smaller context windows
ctx diff --max-tokens 4000
```

### Summary Mode

```bash
# Just see what changed without file contents
ctx diff --summary
```

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
- Related files containing symbols affected by the changes
- Context limited to `--max-tokens`

## Integration with Code Review

```bash
# Get context for a GitHub PR
ctx review 123

# This is equivalent to:
ctx diff $(gh pr view 123 --json baseRefName -q .baseRefName)
```

## Tips

- Use `--summary` first to understand scope of changes
- Increase `--depth` for refactoring tasks
- Combine with `--format markdown` for PR descriptions
- Use in CI to generate change documentation

## See Also

- [ctx review](./review.md) - GitHub PR context helper
- [ctx smart](./smart.md) - Task-based file selection
- [ctx query impact](../code-intelligence.md#impact-analysis) - Impact analysis
