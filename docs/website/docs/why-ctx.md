---
id: why-ctx
title: Why ctx?
sidebar_position: 2
---

# Why ctx?

AI coding agents write fast, sloppy code. They duplicate logic that already exists, drift the
architecture, and declare "done" without proof. **ctx fixes that.**

**ctx is the local quality authority for AI-written code** — a single local binary that builds a
queryable model of your codebase (every symbol, every dependency, every hotspot) and uses it to both
**guide** and **govern** your agent:

- **Guide** — it hands your agent a map before it starts, and shows the blast radius of every edit,
  so its changes reflect how the code actually works.
- **Govern** — it enforces your architecture rules and quality thresholds as deterministic gates the
  agent can't ship past. Rules live in your repo as code; checks run in milliseconds inside the
  agent's loop; nothing leaves your machine.

## The problem: agents modify code blind, and nothing checks them

Agents now write and change real code — but dropped into a repo, they operate without a model of it,
and nothing verifies what they produce. Two failure modes follow:

- **They read too much.** To "be safe," an agent dumps whole directories into the prompt. That burns
  tokens, fills the context window with noise, and makes the model slower and less accurate. Run
  `ctx --count-only --encoding cl100k_base` to measure the file-content baseline for the current
  checkout; formatter wrappers and the optional tree block are not part of that count. *(A
  grounding failure.)*
- **They read too little, and nothing checks the change.** An agent greps a couple of files, edits,
  and misses the caller three hops away that it just broke — and no guardrail flagged the blast
  radius before the change shipped. *(A grounding **and** a governance failure.)*

Grep and file-dumpers don't understand code. Raw embeddings find *similar* code but miss the
*relationships*. And almost nothing evaluates a model's change against the structure of the codebase
before it lands. There's no model of the world the agent edits — and no guardrails on it.

## The solution: build the model once, then ground and govern

`ctx index` builds the world model. Everything else queries it.

### Ground — the right context, in

- **Smart context** — `ctx smart "<task>"` combines semantic search with call-graph expansion to
  pull the files a task touches and fit whole files to a token budget.
- **Token control** — `--count-only`, `--max-tokens`, and `--encoding` measure selected file content
  for any model's window; rendered wrappers and the optional tree block add output tokens.

### Govern — guardrails, on what changes

- **Impact analysis** — `ctx query impact <symbol>` shows what a change would affect, across multiple
  hops, so the blast radius is known *before* an edit lands.
- **Quality gates** — `ctx audit --min-score`, `ctx complexity`, and `ctx duplicates` turn the model
  into CI-enforceable guardrails on the code an agent produces.

### Delivered where the agent lives

`ctx serve --mcp` exposes the whole world model as MCP tools to Claude Desktop and other agents, and
every command speaks `--output json`. It is local by default: the v0.4.0 tag
(`1425cfc5a5ba96802a7adf7a9271bd1f1c3c1dda`) indexed
2,701 symbols and 18,730 extracted edges from this repository; configured LSP and embedding
providers can add their own local or network dependencies.

## Who it's for

- **Developers using AI coding agents** (Claude Code, Cursor, and friends) who want the agent
  grounded in their real codebase and guardrailed against breaking it.
- **Builders of AI dev tools** who need a code world model and retrieval layer over MCP/JSON, instead
  of building code-RAG and static analysis from scratch.

## How it's different

ctx is **not** a file-packer (repomix, gitingest, files-to-prompt) and **not** an IDE indexer
(ctags, LSP). It's a world model built to ground and govern LLMs. See the
[Comparison](comparison.md) for the details.

## Next steps

- [Get started](getting-started.md) — install and build your first world model.
- [Using ctx with agents](guides/using-ctx-with-agents.md) — the recommended loop and MCP setup.
- [Comparison](comparison.md) — how ctx differs from packers and IDE indexers.
