---
id: map
title: ctx map
sidebar_position: 11
---

# ctx map

Generate a token-budgeted map of the codebase to start an LLM session.

## Synopsis

```bash
ctx map [OPTIONS]
```

## Description

`ctx map` condenses the index into a short structural overview that fits a fixed token budget. About the first 10% of the budget goes to a compact project tree. The rest lists the most important symbols, grouped by file and ranked by PageRank over the resolved symbol graph. An AI assistant can read it first in a session to find its way around without whole files pasted in.

Tokens are estimated as `ceil(chars / 4)`. The output is deterministic for the same index state, so it suits SessionStart hooks.

## Prerequisites

```bash
ctx index
```

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--budget <N>` | Maximum tokens in the map | 2000 |
| `--focus <FOCUS>` | A file path, glob, or symbol name. Boosts the matching symbols and their direct neighbors in the ranking | none |
| `-f, --format <FMT>` | Output format: `text`, `markdown` (or `md`), or `json`. `xml` and `plain` are rejected | `text` |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success (informational command) |
| 2 | Operational error (missing index, unsupported format) |

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

- The map shows what the index knows. Reindex after large changes, or the map lags the code.
- Tight budgets drop the lowest-ranked symbols first. Raise `--budget` if a module you need is missing.

## See also

- [ctx smart](./smart.md): task-driven file selection (the map does not depend on a task)
- [Quality gates](../integrations/quality-gates.md): using `ctx map` in a SessionStart hook
