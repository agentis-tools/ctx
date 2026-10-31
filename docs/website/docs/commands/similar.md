---
id: similar
title: ctx similar
sidebar_position: 10
---

# ctx similar

Find existing functions similar to a description before writing new ones.

## Synopsis

```bash
ctx similar "<query>" [PATTERNS]... [OPTIONS]
```

## Description

`ctx similar` searches the index for functions that already do what you are about to write. Run it before adding a new function. Reusing or extending an existing implementation avoids duplication that `ctx duplicates` and `ctx score` would flag later.

By default the search is semantic and needs embeddings from `ctx embed`. `--keyword` uses FTS5 keyword search over names, signatures, and doc comments instead.

Optional positional patterns restrict candidates by file path. ctx ORs literal files,
directories, and globs together. With no patterns, `.` searches the
whole indexed repository.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--limit <N>` | Maximum number of results | 10 |
| `--keyword` | Use keyword (FTS5) search instead of embeddings | false |
| `--provider <local\|openai\|ollama>` | Embedding backend for the query. It must match the provider that built the embeddings. `--openai` is a deprecated alias for `--provider openai` | `local` |
| `--json` | Machine-readable output (global flag) | false |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success (informational command) |
| 2 | Operational error (missing index, missing or incompatible embeddings in semantic mode) |

## Examples

```bash
# Before writing a retry helper, see what already exists
ctx similar "retry an operation with exponential backoff"

# Keyword mode (no embeddings needed)
ctx similar "parse config file" --keyword

# Search only command sources and shared tests
ctx similar "parse config file" src/commands/ "tests/**/*_cli.rs" --keyword

# More candidates, OpenAI embeddings
ctx similar "validate user input" --limit 20 --provider openai

# Machine-readable (standard envelope: ctx_version, command, generated_at, data)
ctx similar "token counting" --json
```

## Caveats

- Semantic mode requires embeddings. Run `ctx embed` first (local model by default, or `--provider openai` or `--provider ollama`). Without embeddings, use `--keyword`.
- Semantic mode refuses to query a corpus built by a different provider, model, or dimension. Rebuild it with `ctx embed --force --provider <provider>`.
- Results rank by meaning, not correctness. Read a candidate before you reuse it.

## See also

- [ctx duplicates](./duplicates.md): structural duplicate detection after the fact
- [Semantic search in code intelligence](../code-intelligence.md)
- [Quality gates](../integrations/quality-gates.md)
