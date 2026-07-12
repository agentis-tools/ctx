---
id: using-ctx-with-agents
title: Using ctx with agents
sidebar_position: 1
---

# Using ctx with AI agents

ctx is designed to be an agent's retrieval layer. There are two ways to wire it in:

1. **Over MCP** (recommended) — `ctx serve --mcp` exposes ctx's capabilities as tools your agent can
   call directly. Best for Claude Desktop and other MCP-aware agents.
2. **As a CLI** — an agent (or a script) shells out to `ctx` and reads structured output. Every
   query command supports `--output json`.

## The recommended loop

```bash
# 1. Index the repo once (fast; reindex only what changed on later runs)
ctx index

# 2. Generate embeddings so semantic + smart retrieval work (one-time model download ~90 MB)
ctx embed

# 3. Retrieve for a task — pick the tool that fits:
ctx smart "implement rate limiting on the API" --max-tokens 8000   # task-scoped context bundle
ctx semantic "where is auth handled" --output json                 # meaning-based symbol search
ctx query impact validateToken                                     # what a change would break
ctx source authenticate                                            # exact source for one symbol
```

Reindex incrementally as the codebase changes (`ctx index`, or `ctx index --watch` to keep it live).

## Govern the change

Grounding gets the agent the right context; governance checks what it produces. After an edit, gate
the change with commands whose **exit code is the verdict** (`0` clean, `1` findings, `2` error — see
[exit codes](../reference/exit-codes.md)):

```bash
# Architecture rules over your .ctx/rules.toml, scoped to the change
ctx check --against origin/main --json

# One composite gate: exit 1 tells the agent its work isn't done
ctx score --against origin/main --fail-on "check_violations>0,new_duplication>0"
```

You don't have to wire this by hand. `ctx harness init --target claude` installs Claude Code hooks
that run [`ctx map`](../commands/map.md) at session start, [`ctx check`](../commands/check.md) after
every edit, and [`ctx score`](../commands/score.md) as a Stop-gate — see
[Quality gates](../integrations/quality-gates.md) and [Claude integration](../integrations/claude.md).

## Over MCP

`ctx serve --mcp` runs an MCP server over stdio. It requires a build with the `mcp` feature:

```bash
cargo install agentis-ctx --features mcp
ctx serve --mcp
```

It exposes eight tools:

| Tool | Purpose |
|------|---------|
| `search_symbols` | Search for symbols by name pattern |
| `get_definition` | Get the source for a symbol |
| `find_references` | Find references to a symbol |
| `get_file` | Read a file's contents |
| `get_file_tree` | List the project's files |
| `get_callers` | Functions that call a given function |
| `get_callees` | Functions a given function calls |
| `smart_context` | Select the files relevant to a task |

For the Claude Desktop configuration, see [Claude integration](../integrations/claude.md).

## As a CLI (JSON for scripting)

When an agent drives ctx as a subprocess, use `--output json` for machine-readable results. It's
available on `search`, `semantic`, `query graph`, `complexity`, `duplicates`, `graph`, and `audit`
(which also takes `--output json`). For example:

```bash
ctx search "handleRequest" --output json
ctx query impact validateToken
ctx audit --output json --min-score 7.0    # non-zero exit if the score is below the gate
```

Context-generating commands (`ctx`, `ctx smart`, `ctx diff`, `ctx review`) emit the formatted
context bundle itself — pipe it straight into a prompt.

## Budget the context to your model

Context windows are finite and tokens cost money. ctx lets an agent measure and cap context before
sending it:

```bash
# Count tokens without emitting the files
ctx --count-only src/

# Fit whole files into a budget (drops least-relevant files; never truncates a file)
ctx smart "<task>" --max-tokens 8000

# Match the tokenizer to the model family
ctx --encoding o200k_base --count-only    # o200k_base | cl100k_base | p50k_base
```

On a real repo the difference is large: the ctx codebase is **502,856 tokens** as a full dump, but a
task-scoped `ctx smart "..." --max-tokens 8000` returns about **8,700 tokens** — the same answer,
~58× less to read and pay for.

## Next steps

- [Claude integration](../integrations/claude.md) — full MCP setup for Claude Desktop.
- [Smart context](../commands/smart.md) — how task-based selection works.
- [Why ctx?](../why-ctx.md) — the reasoning behind agent-first retrieval.
