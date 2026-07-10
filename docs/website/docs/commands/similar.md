---
id: similar
title: ctx similar
sidebar_position: 10
---

# ctx similar

Find existing functions similar to a description before writing new ones.

## Synopsis

```bash
ctx similar "<query>" [OPTIONS]
```

## Description

The `similar` command searches the index for functions that already do what you are about to write. Run it before adding a new function: reusing (or extending) an existing implementation is the cheapest way to avoid duplication that `ctx duplicates` and `ctx score` would flag later.

By default the search is semantic (embedding-based, requires `ctx embed`); `--keyword` falls back to FTS5 keyword search over names, signatures, and doc comments.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--limit <N>` | Maximum number of results | 10 |
| `--keyword` | Use keyword (FTS5) search instead of embeddings | false |
| `--openai` | Use OpenAI embeddings instead of the local model (requires `OPENAI_API_KEY`) | false |
| `--json` | Machine-readable output (global flag) | false |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success (informational command) |
| 2 | Operational error (missing index, missing embeddings for semantic mode) |

## Examples

```bash
# Before writing a retry helper, see what already exists
ctx similar "retry an operation with exponential backoff"

# Keyword mode (no embeddings needed)
ctx similar "parse config file" --keyword

# More candidates, OpenAI embeddings
ctx similar "validate user input" --limit 20 --openai

# Machine-readable (standard envelope: ctx_version, command, generated_at, data)
ctx similar "token counting" --json
```

## Caveats

- Semantic mode requires embeddings: run `ctx embed` (local model) or `ctx embed --openai` first; without them, use `--keyword`.
- Results rank by meaning, not correctness - always read the candidate before reusing it.

## See Also

- [ctx duplicates](./duplicates.md) - structural (after-the-fact) duplicate detection
- [Semantic Search in Code Intelligence](../code-intelligence.md)
- [Quality Gates](../integrations/quality-gates.md)
