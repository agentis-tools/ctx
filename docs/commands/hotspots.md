# ctx hotspots

Rank files and symbols by churn x complexity to find refactoring hotspots.

## Synopsis

```bash
ctx hotspots [OPTIONS]
```

## Description

The `hotspots` command combines git history (how often code changes) with index metrics (how complex it is) to rank the places where refactoring pays off most. Code that is both complex **and** frequently touched is where bugs cluster and where extraction into smaller modules helps first.

## Prerequisites

Requires a git repository and an index:

```bash
ctx index
```

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--since <SPEC>` | Git date spec bounding the churn window (e.g. `"6 months ago"`, `2025-01-01`) | `6 months ago` |
| `--limit <N>` | Maximum number of results | 20 |
| `--by <SCOPE>` | Rank by `file` or `symbol` | `file` |
| `--min-churn <N>` | Ignore entries with fewer than N commits in the window | 2 |
| `--against <REF>` | Only rank files changed relative to REF | none |
| `--json` | Machine-readable output (global flag) | false |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success (informational command) |
| 2 | Operational error (not a git repo, missing index, bad git ref) |

## Examples

```bash
# Top refactoring candidates over the last 6 months
ctx hotspots

# Symbol-level hotspots in the last quarter
ctx hotspots --since "3 months ago" --by symbol --limit 10

# Only files touched by the current branch
ctx hotspots --against main

# Machine-readable (standard envelope: ctx_version, command, generated_at, data)
ctx hotspots --json
```

## Caveats

- Churn counts commits per file from `git log`; renames are not followed, so a renamed file's history restarts.
- Complexity uses the shared fan-in/fan-out formula from the index; dynamic calls and macro-generated code are not tracked.

## See Also

- [ctx score](./score.md)
- [ctx complexity in Code Intelligence](../code-intelligence.md)
- [Quality Gates](../integrations/quality-gates.md)
