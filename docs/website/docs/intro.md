---
id: intro
title: Introduction
sidebar_position: 1
slug: /
---

# ctx

ctx is a queryable model of your codebase for the language models that change it. A local CLI
indexes your repository into symbols, call graphs, relationships, and embeddings, then uses that
index for two jobs:

- Ground the model's input. ctx selects whole files by meaning and call-graph relevance and fits
  them to a token budget, so the model reads the code its change depends on.
- Govern the model's output. Impact analysis lists what an edit can affect, and quality gates
  (architecture rules, duplication, a composite score) can block a change in CI or in an agent's
  stop hook.

```bash
ctx index                                          # build the index once
ctx smart "add rate limiting" --max-tokens 8000    # ground: selected files within 8,000 tokens
ctx query impact validateToken                     # govern: what does a change here affect?
ctx serve --mcp                                    # serve the index to an agent (mcp build)
```

`ctx serve` needs a build with the `mcp` Cargo feature; release binaries do not include it. See
[Getting started](getting-started.md#from-source).

## Where to next

- [Why ctx?](why-ctx.md): the problem it solves and how grounding and governing work.
- [Getting started](getting-started.md): install ctx and build your first index.
- [Using ctx with agents](guides/using-ctx-with-agents.md): the recommended agent loop and MCP setup.
- [Cookbook](cookbook/index.md): workflows for agent-assisted engineering, change governance, and
  codebase health over time.
- [Comparison](comparison.md): how ctx differs from file packers and IDE indexers.

By topic: [Context generation](context-generation.md), [Code intelligence](code-intelligence.md),
and [Configuration](configuration.md).

## Install

```bash
cargo install agentis-ctx
cargo binstall agentis-ctx
brew install agentis-tools/tap/ctx
yay -S ctx-bin
```

On Windows, install from the [Scoop bucket](https://github.com/agentis-tools/scoop-bucket). Each
[GitHub release](https://github.com/agentis-tools/ctx/releases) also carries Debian and RPM
packages. [Getting started](getting-started.md) covers every platform and how to upgrade.

ctx has built-in parsers for Rust, TypeScript, JavaScript, JSX/TSX, Python, Go, Solidity, C, C++,
Zig, and YAML, and can index other languages through a language server
([Add a language via LSP](lsp-languages.md)). It runs on macOS, Linux, and Windows.
