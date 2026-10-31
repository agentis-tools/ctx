---
id: why-ctx
title: Why ctx?
sidebar_position: 2
---

# Why ctx?

AI coding agents duplicate logic that already exists, break architectural boundaries, and report a
task as done without evidence. ctx gives the agent a model of the codebase before it edits and
checks the result afterwards.

ctx is a single local binary. It indexes your codebase (symbols, calls, and dependencies) into
a queryable model and uses it in two ways:

- Guide the agent. ctx gives it a codebase map before it starts and lists what an edit can affect.
- Govern the agent. ctx evaluates your architecture rules and quality thresholds as deterministic
  gates. The rules live in your repository, the checks run locally inside the agent's loop, and
  ctx sends no code anywhere unless you turn on a network provider (OpenAI or remote Ollama
  embeddings, or `ctx judge`).

## The problem: agents edit code without a model of it, and nothing checks them

An agent dropped into a repository has no model of it, and nothing verifies what it produces. Two
failures follow:

- It reads too much. To be safe, an agent pastes whole directories into the prompt. That costs
  tokens, fills the context window with irrelevant code, and makes the model slower and less
  accurate. `ctx --count-only --encoding cl100k_base` measures the file contents of the current
  checkout; format wrappers and the optional tree are not part of that count. This is a grounding
  failure.
- It reads too little, and nothing checks the change. An agent greps a couple of files, edits them,
  and breaks a caller three calls away, and no check reported that caller before the change
  merged. This is both a grounding and a governance failure.

grep and file packers do not parse code. Embeddings alone find similar code but not the calls
between functions. Few tools check a model's change against the structure of the codebase before it
merges.

## The approach: index once, then ground and govern

`ctx index` builds the model. The other commands query it.

### Ground: what goes into the prompt

- `ctx smart "<task>"` combines semantic search with call-graph expansion to select the files a
  task touches, and fits whole files into a token budget that covers the complete rendered output.
- `ctx map` condenses the index into a token-budgeted overview of modules, key symbols, and their
  relationships for the start of a session.
- `--count-only`, `--max-tokens`, and `--encoding` measure selected file contents for a model's
  context window.

### Govern: checks on what changes

- `ctx query impact <symbol>` lists what a change can affect across several calls, before the edit.
- `ctx check` evaluates architecture rules from `.ctx/rules.toml`, and `ctx score --fail-on` gates
  a change on rule violations, new duplication, and coupling deltas. Both exit 1 when a gate
  fails, so CI and agent hooks can block on them.
- `ctx duplicates`, `ctx hotspots`, `ctx complexity`, and `ctx audit` report evidence to
  investigate.

### Where the agent works

`ctx harness init` installs Claude Code or Codex hooks that run these checks during a session.
`ctx serve --mcp` (in builds with the `mcp` feature) exposes the index as MCP tools, and most
commands accept `--json` for a structured result. On the v0.4.0 tag
(`0f4eea866055baa4e3e0aa4740a4b7e179a5779f`), `ctx index` extracted 2,701 symbols and 18,730 edges
from this repository. Language servers and embedding providers you configure can add their own
local or network dependencies.

## Who it's for

- Developers using AI coding agents (Claude Code, Codex, Cursor, and others) who want the agent to
  work from the real structure of their codebase and to be stopped when it breaks it.
- Builders of AI developer tools who need code retrieval and static analysis over MCP or JSON
  instead of writing their own.

## How it differs

ctx is not a file packer (repomix, gitingest, files-to-prompt) and not an IDE indexer (ctags,
LSP). The [comparison](comparison.md) has the details.

## Next steps

- [Getting started](getting-started.md): install ctx and build your first index.
- [Using ctx with agents](guides/using-ctx-with-agents.md): the recommended loop and MCP setup.
- [Comparison](comparison.md): how ctx differs from packers and IDE indexers.
