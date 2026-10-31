---
id: using-ctx-with-agents
title: Using ctx with agents
sidebar_position: 1
---

# Using ctx with AI agents

ctx is built to be an agent's retrieval and checking layer. There are three ways to connect it:

1. Hooks. `ctx harness init` installs Claude Code or Codex hooks that give the agent a codebase map
   at session start, check architecture rules after each edit, and run a quality gate at stop.
2. MCP. `ctx serve --mcp` exposes ctx as tools the agent calls directly. It needs a build with the
   `mcp` feature.
3. CLI. The agent or a script runs `ctx` and reads the result. Most commands accept the global
   `--json` flag and return one structured envelope.

## The recommended loop

```bash
# 1. Index the repository (later runs only reprocess changed files)
ctx index

# 2. Generate embeddings for semantic and smart retrieval (first run downloads about 90 MB)
ctx embed

# 3. Retrieve for a task with the command that fits:
ctx smart "implement rate limiting on the API" --max-tokens 8000   # task-scoped context
ctx semantic "where is auth handled" --json                        # search by meaning
ctx query impact validateToken                                     # what a change can affect
ctx source authenticate                                            # source of one symbol
```

Reindex as the code changes: run `ctx index` again, or keep `ctx index --watch` running.

## Govern the change

Grounding gives the agent the right context; governance checks what it produces. After an edit,
gate the change with commands whose exit code is the verdict (`0` clean, `1` findings, `2` error;
see [exit codes](../reference/exit-codes.md)):

```bash
# Architecture rules from .ctx/rules.toml, limited to the change
ctx check --against origin/main --json

# One composite gate: exit 1 tells the agent its work is not done
ctx score --against origin/main --fail-on "check_violations>0"
```

The generated hooks block only on `check_violations` by default. Duplication still appears in the
scorecard; add `new_duplication>0` to `--fail-on` once you have checked its findings against your
repository.

You do not have to wire this by hand. `ctx harness init` (Claude Code, the default) or
`ctx harness init --target codex` installs hooks that run [`ctx map`](../commands/map.md) at session
start, [`ctx check`](../commands/check.md) after every edit, and [`ctx score`](../commands/score.md)
at stop. See [Quality gates](../integrations/quality-gates.md),
[Claude integration](../integrations/claude.md), and [Codex integration](../integrations/codex.md).

## Over MCP

`ctx serve --mcp` runs an MCP server over stdio. Release binaries do not include it; build ctx with
the `mcp` feature:

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

## As a CLI (JSON for scripts)

When an agent runs ctx as a subprocess, pass `--json`. stdout then holds one envelope with the
command's result, and diagnostics stay on stderr. The [JSON output reference](../json-output.md)
lists the commands that support it and their payloads. Some older commands also have
`--output json`, which prints their own shape without the envelope.

```bash
ctx search "handleRequest" --json
ctx query impact validateToken --json
ctx score --against origin/main --json
ctx audit --output json --min-score 7.0    # exit 1 if the score is below 7.0
```

The context commands (`ctx`, `ctx smart`, `ctx diff`, `ctx review`) print the context document
itself, ready to pipe into a prompt.

## Budget the context to your model

Context windows are limited and tokens cost money. An agent can measure and cap context before
sending it:

```bash
# Count tokens without printing the files
ctx --count-only src/

# Fit whole files into a budget (leaves out the least relevant files; never truncates a file)
ctx smart "<task>" --max-tokens 8000

# Match the tokenizer to the model family
ctx --encoding o200k_base --count-only    # o200k_base | cl100k_base | p50k_base
```

Measure the file contents of the whole checkout with `ctx --count-only --encoding cl100k_base`,
then compare `ctx smart "..." --max-tokens 8000` for the task. For `ctx smart`, the budget covers
the complete rendered output, including the tree and format markup. In v0.4.0 it covered selected
file contents only.

## Next steps

- [Claude integration](../integrations/claude.md): MCP setup for Claude Desktop.
- [Smart context](../commands/smart.md): how task-based selection works.
- [Why ctx?](../why-ctx.md): the reasoning behind agent-first retrieval.
