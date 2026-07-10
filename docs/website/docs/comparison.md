---
id: comparison
title: Comparison
sidebar_position: 3
---

# How ctx compares

Tools that touch agent-written code fall into three planes. Most products live in exactly one. ctx
spans them — a queryable world model that both **grounds** what an agent reads and **governs** what
it writes — and occupies a square none of them do: **local, deterministic governance inside the
agent's per-turn loop, with gates you author and commit like code.**

## Plane 1 — context packers (repomix, gitingest, files-to-prompt)

Packers concatenate files (optionally filtered) into one blob you paste into an LLM. Great at
formatting and filtering, but they don't understand your code — selection is manual, and the output
is only as good as the globs you gave it.

| | Context packers | **ctx** |
|---|---|---|
| LLM-ready output | ✅ | ✅ (XML / Markdown / JSON / plain) |
| Ignore rules, noise filtering | ✅ | ✅ (`.gitignore` + 170+ built-ins) |
| Token counting / budgeting | Sometimes | ✅ `--count-only`, `--max-tokens`, `--encoding` |
| Selects files *by relevance to a task* | ❌ (you pick globs) | ✅ `ctx smart` (semantic + call-graph) |
| Understands call graphs / symbols | ❌ | ✅ tree-sitter index |
| Governs what the agent writes | ❌ | ✅ rules, scorecards, gates |

## Plane 2 — code-graph tools for agents (CodeGraph, GitNexus, Serena)

These build a structural graph of your codebase and expose it to agents over MCP — callers,
dependencies, symbol navigation. They make an agent a better *reader*. But they stop at perception:
none of them rule-check, score, or gate what the agent *writes*.

| | Code-graph / MCP tools | **ctx** |
|---|---|---|
| Structural graph over MCP | ✅ | ✅ `ctx serve --mcp` |
| Semantic + keyword search | ✅ | ✅ local or OpenAI embeddings |
| Token-budgeted repo map | Some | ✅ `ctx map --budget` |
| Architecture rules enforced on a change | ❌ | ✅ `ctx check` over `.ctx/rules.toml` |
| Quality scorecard / CI gate | ❌ | ✅ `ctx score --fail-on` |
| Per-turn gate in the agent loop | ❌ | ✅ Claude Code hooks via `ctx harness` |

**Takeaway:** they help the agent read; ctx also constrains what it writes — same substrate, one more job.

## Plane 3 — AI-code quality platforms (SonarQube, CodeScene)

These genuinely govern AI-written code — but as platforms: server or SaaS deployments, metric
catalogs you *configure* rather than gates you *author*, and PR/IDE checks that run after the fact,
not inside the agent's per-turn loop.

| | Quality platforms | **ctx** |
|---|---|---|
| Governs code quality | ✅ | ✅ |
| Runs locally as a single binary | ❌ (server / cloud) | ✅ |
| In the agent's per-turn loop (sub-second hook) | ❌ (PR / IDE, post hoc) | ✅ |
| Gates authored + version-controlled in the repo | ❌ (dashboard config) | ✅ `.ctx/rules.toml`, gate files |
| No account / infrastructure | ❌ | ✅ offline, one file |

**Takeaway:** platforms gate at the PR; ctx gates at the edit — locally, before the change is even done.

## Adjacent: IDE indexers (ctags, LSP) and DIY code-RAG

ctags and Language Servers index symbols too, but for human editors — they don't emit LLM-ready
context, rank by meaning, or govern anything. And you can hand-roll a code-RAG pipeline (chunk, embed,
store vectors), but ctx *is* that pipeline — purpose-built and local, one binary with `sqlite-vec`
indexed vector search, real tree-sitter call graphs, and the governance layer on top — with no
infrastructure to run.

## The one-line summary

Code-graph tools help agents read your code. Quality platforms audit it after the fact, from a
server. **ctx is the only tool that governs what agents *write*, while they write it, locally.**

Unlike code-graph tools, ctx governs what agents write — not just what they read. Unlike quality
platforms, it does so in milliseconds, locally, with gates you own.

See [Why ctx?](why-ctx.md) for the reasoning, or [get started](getting-started.md) to try it.
