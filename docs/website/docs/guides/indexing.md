---
id: indexing
title: Index & embed first
sidebar_position: 1
---

# Index & embed first

:::tip Do this before anything else
Every intelligence and governance command — `query`, `search`, `map`, `similar`, `check`, `score`,
`hotspots`, `duplicates`, `smart` — reads a **prebuilt index**. Run `ctx index` once before you use
them. Semantic features (`semantic`, `smart`, `similar`) additionally need `ctx embed`.
:::

## 1. Build the index

```bash
ctx index
```

This parses your code with tree-sitter and writes a single SQLite file at `.ctx/codebase.sqlite` —
every symbol, call/import/inheritance edge, and complexity metric. Re-running is **incremental**: it
only re-parses files that changed.

Parsing runs **in parallel across CPU cores by default**. Pass `--serial` for single-threaded
execution.

| Flag | What it does |
|------|--------------|
| `-w`, `--watch` | Keep running and **reindex automatically** as files change |
| `--serial` | Parse single-threaded (parallel is the default) |
| `--force` | Full rebuild — clears the database and re-parses everything |
| `-p`, `--pattern <GLOB>` | Only index matching files (repeatable) |
| `-i`, `--ignore <GLOB>` | Extra ignore patterns (repeatable) |
| `-v`, `--verbose` | Print each file as it's indexed |

The `-j`/`--parallel` flag is retained as a no-op for backward compatibility.

```bash
ctx index --force                  # rebuild from scratch after big changes
ctx index --serial                 # force single-threaded parse
ctx index -p "src/**/*.rs"         # index only Rust sources
```

## 2. Generate embeddings (for semantic search)

```bash
ctx embed
```

`semantic`, `smart`, and `similar` rank code by *meaning*, which needs vector embeddings. The first
run **downloads a local model (~90 MB)** and embeds every indexed symbol; later runs only embed
what's new. It runs fully locally — no API key required. Embedding computation runs **in parallel by
default** (chunked across rayon threads, preserving order); pass `--serial` for single-threaded.

| Flag | What it does |
|------|--------------|
| `-w`, `--watch` | Auto-embed new symbols as the index changes |
| `--force` | Re-embed every symbol from scratch |
| `--serial` | Compute embeddings single-threaded (parallel is the default) |
| `--batch-size <N>` | Symbols per batch (default 50) |
| `--provider <local\|openai\|ollama>` | Embedding backend (default `local`); see [Configuration](../configuration.md#embedding-providers) |
| `--openai` | Deprecated alias for `--provider openai` (needs `OPENAI_API_KEY`) |

## Run them as background watchers

For an always-fresh model while you (or your agent) work, run the watchers in the background — the
index and embeddings stay current without you re-running anything:

```bash
ctx index --watch &     # reindex on every file change
ctx embed --watch &     # embed new symbols as they're indexed
```

Run these in a spare terminal or as background jobs. The `index --watch` process debounces rapid
edits; `embed --watch` reacts to index updates. Stop them with `kill %1 %2` (or close the terminal).

:::note With an AI agent, let the harness do it
[`ctx harness init --target claude`](../commands/harness.md) wires indexing straight into Claude
Code: it reindexes on every `Edit`/`Write` via a `PostToolUse` hook, so the agent is always working
against a current model. See [Using ctx with agents](using-ctx-with-agents.md).
:::

## What needs what

| You want to run | Requires |
|-----------------|----------|
| `query`, `search`, `map`, `check`, `score`, `hotspots`, `duplicates`, `graph` | `ctx index` |
| `semantic`, `smart`, `similar` | `ctx index` **+** `ctx embed` |
| context generation (bare `ctx`, `ctx diff`) | nothing — works without an index |

## Next steps

- [Code intelligence](../code-intelligence.md) — querying the index you just built.
- [Using ctx with agents](using-ctx-with-agents.md) — the full index → embed → govern loop.
