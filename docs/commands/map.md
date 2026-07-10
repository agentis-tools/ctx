# ctx map

Generate a token-budgeted map of the codebase for LLM session bootstrap.

## Synopsis

```bash
ctx map [OPTIONS]
```

## Description

The `map` command condenses the index into a compact structural overview - modules, key symbols, and their relationships - that fits a fixed token budget. It is designed to be the first thing an AI assistant reads in a session: enough orientation to navigate the codebase without pasting whole files.

## Prerequisites

```bash
ctx index
```

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--budget <N>` | Maximum tokens in the map | 2000 |
| `--focus <PATH>` | Zoom the map on a path or module (more detail there, less elsewhere) | none |
| `--format <FMT>` | Output format: `text`, `markdown`, or `json` | `text` |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success (informational command) |
| 2 | Operational error (missing index) |

## Examples

```bash
# Compact overview for a session start hook
ctx map --budget 2000

# Zoom on one subsystem
ctx map --focus src/parser --budget 4000

# Markdown for pasting into a PR description or issue
ctx map --format markdown
```

## Caveats

- The map summarizes what the index knows: reindex after large changes, or the map lags the code.
- Tight budgets drop detail deterministically (least-connected symbols first); raise `--budget` if a module you need is missing.

## See Also

- [ctx smart](./smart.md) - task-driven file selection (map is task-agnostic orientation)
- [Quality Gates](../integrations/quality-gates.md) - using `ctx map` in a SessionStart hook
