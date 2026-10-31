# ctx hotspots

Rank files and symbols by churn × size (default) or churn × complexity (`--signal complexity`) to find refactoring candidates.

## Synopsis

```bash
ctx hotspots [OPTIONS]
```

## Description

`ctx hotspots` multiplies git churn (how often code changes) by a structural signal from the index and ranks the result. The default signal is size in lines: a symbol's span, or for a file the sum of its symbols' spans. `--signal complexity` uses the index's complexity metric instead.

Size is the default because it predicted bug fixes better. In time-split backtests on 10 repositories (Rust, Python, TypeScript, Go, C, C++), size beat complexity at predicting which functions later got bug fixes in all 10, and churn × size beat churn × complexity in 9 of 10. Treat the ranking as a list of places to look, not as proof that the code is wrong.

ctx v0.4.0 and earlier rank by churn × complexity and have no `--signal` option.

## Prerequisites

Requires a git repository and an index:

```bash
ctx index
```

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--signal <SIGNAL>` | Structural signal multiplied with churn: `lines` or `complexity` | `lines` |
| `--since <SPEC>` | Git date spec bounding the churn window (for example `"6 months ago"`, `2025-01-01`) | `6 months ago` |
| `--limit <N>` | Maximum number of results | 20 |
| `--by <SCOPE>` | Rank by `file` or `symbol`. A symbol's churn is its file's churn | `file` |
| `--min-churn <N>` | Ignore entries with fewer than N commits in the window | 2 |
| `--against <REF>` | Only rank files changed relative to REF | none |
| `--json` | Machine-readable output (global flag) | false |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success (informational command) |
| 2 | Operational error (not a git repo, missing index, bad git ref) |

## Examples

```bash
# Top refactoring candidates over the last 6 months
ctx hotspots

# Rank by churn × complexity instead of churn × size
ctx hotspots --signal complexity

# Symbol-level hotspots in the last quarter
ctx hotspots --since "3 months ago" --by symbol --limit 10

# Only files touched by the current branch
ctx hotspots --against main

# Machine-readable (standard envelope: ctx_version, command, generated_at, data)
ctx hotspots --json
```

## Caveats

- Churn counts commits per file from `git log`. Renames are not followed, so a renamed file's history restarts.
- With `--by symbol`, every symbol in a file gets the file's churn.
- Complexity is the index's coupling formula (fan-out and same-file fan-in), not control-flow complexity. ctx does not track dynamic calls or macro-generated code.

## See also

- [ctx score](./score.md)
- [ctx complexity in code intelligence](../code-intelligence.md)
- [Quality gates](../integrations/quality-gates.md)
