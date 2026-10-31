---
id: indexing
title: Index and embed first
sidebar_position: 1
---

# Index and embed first

:::tip Do this before anything else
Every intelligence and governance command (`query`, `search`, `map`, `similar`, `check`, `score`,
`hotspots`, `duplicates`, `smart`) reads a prebuilt index. Run `ctx index` once before you use
them. The semantic commands (`semantic`, `smart`, and `similar` without `--keyword`) also need
`ctx embed`.
:::

## 1. Build the index

```bash
ctx index
```

`ctx index` parses your code with tree-sitter and writes one SQLite file, `.ctx/codebase.sqlite`,
with every symbol, call, import, and inheritance edge, and the metrics derived from them. Later runs
only parse files that changed.

Parsing runs in parallel by default. `--serial` uses one thread.

| Flag | What it does |
|------|--------------|
| `-w`, `--watch` | Keep running and reindex when files change |
| `--serial` | Parse on one thread (parallel is the default) |
| `--force` | Clear the database and parse everything again |
| `-p`, `--pattern <GLOB>` | Only index matching files (repeatable); positional paths work the same way |
| `-i`, `--ignore <GLOB>` | Extra ignore patterns (repeatable) |
| `-v`, `--verbose` | Print each file as it is indexed |

`-j`/`--parallel` is still accepted and does nothing.

```bash
ctx index --force                  # rebuild from scratch after large changes
ctx index --serial                 # parse on one thread
ctx index -p "src/**/*.rs"         # index only Rust sources
ctx index src                      # index only src/
```

If an explicit scope matches no files, `ctx index` exits non-zero and leaves the existing index
unchanged.

To resolve ambiguous call edges with a model after each run, set `[judge] edges = true` in
`.ctx/config.toml` (see [Configuration](../configuration.md#model-assisted-edge-resolution-judge)).

## 2. Generate embeddings (for semantic search)

```bash
ctx embed
```

`semantic`, `smart`, and `similar` rank code by meaning, which needs vector embeddings. The first
run downloads a local model (about 90 MB) and embeds every indexed symbol; later runs embed only new
symbols. The default provider runs locally and needs no API key. ctx computes embeddings in
parallel by default; `--serial` uses one thread.

| Flag | What it does |
|------|--------------|
| `-w`, `--watch` | Embed new symbols as the index changes |
| `--force` | Clear the stored embeddings and embed every symbol again (also applies when `--watch` starts) |
| `--serial` | Compute embeddings on one thread (parallel is the default) |
| `--batch-size <N>` | Symbols per batch (default 50) |
| `--provider <local\|openai\|ollama>` | Embedding backend (default `local`); see [Configuration](../configuration.md#embedding-providers) |
| `--openai` | Deprecated alias for `--provider openai` (needs `OPENAI_API_KEY`) |

The query provider must match the provider and model that built the embeddings. After switching,
run `ctx embed --force --provider <provider>`; until then, semantic commands exit with code 2.

## Run them as background watchers

To keep the index and embeddings current while you or your agent work, run both watchers in the
background:

```bash
ctx index --watch &     # reindex on every file change
ctx embed --watch &     # embed new symbols as they are indexed
```

`index --watch` waits for a burst of edits to settle before reindexing, and `embed --watch`
reacts to index updates. Stop them with `kill %1 %2` or by closing the terminal.

:::note With an AI agent, let the harness do it
[`ctx harness init`](../commands/harness.md) installs Claude Code or Codex hooks that reindex after
every edit, so the agent always works against a current index. See
[Using ctx with agents](using-ctx-with-agents.md).
:::

## What needs what

| You want to run | Requires |
|-----------------|----------|
| `query`, `search`, `map`, `check`, `score`, `hotspots`, `duplicates`, `graph`, `similar --keyword` | `ctx index` |
| `semantic`, `smart`, `similar` | `ctx index` and `ctx embed` |
| `ctx diff`, `ctx review` | Works without an index; uses one for related-file expansion when it exists |
| Context generation (`ctx` without a subcommand) | Nothing |

## Next steps

- [Code intelligence](../code-intelligence.md): querying the index you just built.
- [Using ctx with agents](using-ctx-with-agents.md): the full index, embed, and govern loop.
