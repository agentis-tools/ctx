---
id: comparison
title: Comparison
sidebar_position: 3
---

# How ctx compares

Tools that work on agent-written code fall into three groups, and most products belong to one. ctx
covers parts of all three: it selects what an agent reads and checks what it writes. Its gates run
locally, inside the agent's loop, from rules you write and commit with the code.

## Context packers (repomix, gitingest, files-to-prompt)

Packers concatenate files, optionally filtered, into one document you paste into an LLM. They
format and filter well, but they do not parse your code: you choose the files, and the output is
only as good as your globs.

| | Context packers | ctx |
|---|---|---|
| LLM-ready output | ✅ | ✅ (XML, Markdown, JSON, plain) |
| Ignore rules | ✅ | ✅ (`.gitignore`, `.contextignore`, 164 built-in patterns) |
| Token counting and budgets | Sometimes | ✅ `--count-only`, `--max-tokens`, `--encoding` |
| Selects files by relevance to a task | ❌ (you pick globs) | ✅ `ctx smart` (semantic search plus call graph) |
| Parses symbols and call graphs | ❌ | ✅ tree-sitter index |
| Checks what the agent writes | ❌ | ✅ rules, scorecards, gates |

## Code-graph tools for agents (CodeGraph, GitNexus, Serena)

These build a structural graph of a codebase and expose it to agents over MCP: callers,
dependencies, symbol navigation. They help an agent read. They do not check, score, or gate what
the agent writes.

| | Code-graph and MCP tools | ctx |
|---|---|---|
| Structural graph over MCP | ✅ | ✅ `ctx serve --mcp` (in builds with the `mcp` feature) |
| Semantic and keyword search | ✅ | ✅ local, Ollama, or OpenAI embeddings; FTS5 keywords |
| Token-budgeted repository map | Some | ✅ `ctx map --budget` |
| Architecture rules checked on a change | ❌ | ✅ `ctx check` over `.ctx/rules.toml` |
| Quality scorecard and CI gate | ❌ | ✅ `ctx score --fail-on` |
| Gate inside the agent loop | ❌ | ✅ Claude Code and Codex hooks from `ctx harness init` |

These tools help the agent read. ctx uses the same kind of index to also check what it writes.

## AI code quality platforms (SonarQube, CodeScene)

These do govern code quality, but as platforms: server or SaaS deployments, metric catalogs you
configure rather than gates you write, and checks that run on the pull request or in the IDE after
the agent has finished.

| | Quality platforms | ctx |
|---|---|---|
| Governs code quality | ✅ | ✅ |
| Runs locally as a single binary | ❌ (server or cloud) | ✅ |
| Runs inside the agent's loop | ❌ (pull request or IDE, afterwards) | ✅ hooks after each edit and at stop |
| Gates written and versioned in the repository | ❌ (dashboard configuration) | ✅ `.ctx/rules.toml`, hook and CI files |
| No account or infrastructure | ❌ | ✅ local index in one SQLite file |

Platforms gate at the pull request. ctx can gate while the agent is still editing, on your machine.

## Related: IDE indexers (ctags, LSP) and do-it-yourself code RAG

ctags and language servers index symbols for human editors. They do not produce LLM-ready context,
rank by meaning, or check changes. You can also build a code RAG pipeline yourself (chunk, embed,
store vectors). ctx is such a pipeline in one local binary: tree-sitter call graphs, `sqlite-vec`
vector search, and the checks on top, with no service to run.

## Summary

Code-graph tools help agents read your code. Quality platforms review it afterwards, from a
server. ctx checks what agents write while they write it, locally, with gates you own.

See [Why ctx?](why-ctx.md) for the reasoning, or [get started](getting-started.md) to try it.
