---
id: intro
title: Introduction
sidebar_position: 1
slug: /
---

# ctx

**ctx is a queryable world model of your codebase, built to ground and govern the language models
that modify it.** It's a fast, local CLI that indexes your repo into a model you can query — symbols,
call graphs, relationships, and semantics — then uses that model to do two things:

- **Ground** the model's input — feed it accurate, token-budgeted context selected by meaning *and*
  call-graph relevance, so its changes reflect how the code actually works.
- **Govern** the model's output — put guardrails on what it changes, with impact analysis for the
  blast radius of an edit and quality gates you can enforce in CI.

```bash
ctx index                                          # build the world model once
ctx smart "add rate limiting" --max-tokens 8000    # ground: the right context, budgeted
ctx query impact validateToken                     # govern: what breaks if this changes?
ctx serve --mcp                                     # expose the whole model to your agent
```

## Where to next

- **[Why ctx?](why-ctx.md)** — the problem it solves and how grounding and governing work.
- **[Get started](getting-started.md)** — install and build your first world model in minutes.
- **[Using ctx with agents](guides/using-ctx-with-agents.md)** — the recommended agent loop and MCP setup.
- **[Cookbook](cookbook/index.md)** — outcome-driven workflows for everyday agent-assisted engineering, change governance, and continuous codebase health.
- **[Comparison](comparison.md)** — how ctx differs from file-packers and IDE indexers.

Prefer to learn by topic? Jump to [Context Generation](context-generation.md),
[Code Intelligence](code-intelligence.md), or [Configuration](configuration.md).

## Install

```bash
cargo install agentis-ctx
cargo binstall agentis-ctx
brew install agentis-tools/tap/ctx
yay -S ctx-bin
```

Windows users can install from the
[Scoop bucket](https://github.com/agentis-tools/scoop-bucket); Debian and RPM packages are attached
to each [GitHub Release](https://github.com/agentis-tools/ctx/releases). See
[Getting Started](getting-started.md) for all platforms and upgrade guidance.

Supports Rust, TypeScript, JavaScript, JSX/TSX, Python, Go, Solidity, and YAML. Runs locally on
macOS, Linux, and Windows — see [Getting Started](getting-started.md) for platform notes.
