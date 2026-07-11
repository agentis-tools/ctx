# ctx

[![Crates.io](https://img.shields.io/crates/v/agentis-ctx)](https://crates.io/crates/agentis-ctx)
[![CI](https://github.com/agentis-tools/ctx/actions/workflows/ci.yml/badge.svg)](https://github.com/agentis-tools/ctx/actions)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](#license)
[![Rust Version](https://img.shields.io/badge/rust-1.91%2B-orange)](https://www.rust-lang.org)
[![Docs](https://img.shields.io/badge/docs-docs.agentis.tools-blue)](https://docs.agentis.tools/)

**AI coding agents write fast, sloppy code.** They duplicate logic that already exists, drift the
architecture, and declare "done" without proof.

**`ctx` is the local quality authority for AI-written code.** It's a single local binary that turns
your repo into a queryable model (every symbol, call, and dependency) and uses it to **ground**
your agent and **gate** its output on every turn:

- **Ground:** hand the agent a map before it starts, and the right ~8k tokens of context instead of
  the wrong 233k.
- **Govern:** show the blast radius of every edit, and enforce your architecture rules as
  deterministic gates it can't ship past.

Rules live in your repo as code, checks run in milliseconds inside the agent's loop, and nothing
leaves your machine.

> Unlike code-graph tools, ctx governs what agents *write*, not just what they read. Unlike quality
> platforms, it does so in milliseconds, locally, with gates you own.

📖 **Documentation:** https://docs.agentis.tools/

## Install

```bash
cargo install agentis-ctx        # crate is `agentis-ctx`; it installs the `ctx` binary
```

Prebuilt binaries for Linux (x86_64), macOS (Intel + Apple Silicon), and Windows (x86_64) are
attached to each [GitHub release](https://github.com/agentis-tools/ctx/releases). Once installed,
`ctx self-update` upgrades in place (every download is checksum-verified against the release's
`SHA256SUMS` before replacing the binary).

```bash
# On Windows (MSVC), DuckDB analytics aren't available, so skip the default feature:
cargo install agentis-ctx --no-default-features
```

See the [Getting Started guide](https://docs.agentis.tools/docs/getting-started) for the full
platform matrix.

## The loop: index → ground → govern

**Build the model first.** Every intelligence and governance command reads a prebuilt index, so
`ctx index` always comes first (it writes a single `.ctx/codebase.sqlite`). On this repo it indexes
**870 symbols and 5,463 call edges in 0.36s.** Run it once, then keep it warm with `--watch`.

```bash
ctx index                        # build the world model (or `ctx index --watch` in the background)
```

**Ground:** feed the agent the right context, selected by meaning *and* call-graph relevance:

```bash
ctx smart "add rate limiting" --max-tokens 8000   # ~8.7k tokens instead of 233k, about 27× smaller
ctx diff --summary                                # context for what changed, with dependency expansion
ctx similar "retry with backoff"                  # reuse before you write: find it if it already exists
```

**Govern:** guardrail what the agent changes, with deterministic pass/fail gates:

```bash
ctx check --against origin/main                                   # enforce architecture rules
ctx score --fail-on "check_violations>0,new_duplication>0"       # one composite quality gate
```

## One model, two jobs

ctx indexes your repo into a structured, queryable model: symbols, call graphs, relationships, and
semantics. Not a bag of files, but a model an agent (or you) can ask questions of. That one model
does two jobs: it feeds the model the right context going *in*, and guardrails what it changes coming
*out*.

### Ground: the right context, in

| Command | What it does |
|---|---|
| [`ctx smart "<task>"`](https://docs.agentis.tools/docs/commands/smart) | Rank files by semantic + call-graph relevance, fit to a token budget |
| [`ctx map`](https://docs.agentis.tools/docs/commands/map) | Token-budgeted architectural overview (PageRank over the symbol graph) |
| [`ctx diff`](https://docs.agentis.tools/docs/commands/diff) | Context for git changes, with automatic dependency expansion |
| [`ctx similar "<description>"`](https://docs.agentis.tools/docs/commands/similar) | Find existing functions before writing new ones (with fan-in) |
| `ctx search` / `ctx semantic` | Keyword (FTS5) and embedding-based symbol search |
| `ctx query impact / callers / deps / graph` | Walk the call graph: blast radius, callers, dependencies |

Plain `ctx` is also a **context generator:** select files by glob and stream LLM-ready output:

```bash
ctx src/ | pbcopy                 # copy source to the clipboard (XML by default)
ctx --format markdown src/        # or Markdown / JSON / plain
ctx --count-only src/             # just count tokens against your model's window
ctx --max-tokens 8000 "src/**/*.rs"
```

Semantic search and `ctx smart`/`ctx similar` need embeddings first. Generate them with
`ctx embed` — `--provider local` (default, a ~90 MB fastembed model), `--provider ollama`
(any local Ollama model, offline and free), or `--provider openai` (needs `OPENAI_API_KEY`). See
[Index & embed first](https://docs.agentis.tools/docs/guides/indexing).

### Govern: guardrails on what changes

| Command | What it does | Gate |
|---|---|---|
| [`ctx check`](https://docs.agentis.tools/docs/commands/check) | Enforce architecture rules from `.ctx/rules.toml` over the real edge graph | Exit 1 on any violation |
| [`ctx score`](https://docs.agentis.tools/docs/commands/score) | Composite delta: check violations + new duplication + complexity/fan-out | `--fail-on "<expr>"` |
| [`ctx duplicates`](https://docs.agentis.tools/docs/commands/duplicates) | MinHash near-duplicate detection over normalized token shingles | `--fail-on-found` |
| [`ctx hotspots`](https://docs.agentis.tools/docs/commands/hotspots) | Rank refactoring targets by churn × complexity | informational |
| [`ctx sql`](https://docs.agentis.tools/docs/commands/sql) | Read-only SQL over the stable `v1.*` views for custom queries and gates | `--fail-on-rows` |

Architecture rules live in your repo as code (`.ctx/rules.toml`: layers, forbidden dependencies,
fan-in/complexity limits) and `--against <ref>` scopes any gate to only what a diff changed, so a PR
or an agent is judged on its *new* violations, not the repo's history.

**Exit codes are the integration API.** Every governance command shares one convention, so CI and
agents read the result the same way:

| Code | Meaning |
|------|---------|
| `0` | Success, nothing to report |
| `1` | Ran successfully but produced findings (rule violations, gate hit) |
| `2` | Operational error (bad arguments, missing index, git failure) |
| `3` | Version requirement not met (reserved for `ctx harness compat --require`) |

See the [Quality Gates guide](https://docs.agentis.tools/docs/integrations/quality-gates) for CI
recipes and the [`--json` contract](https://docs.agentis.tools/docs/json-output).

## Drop it into Claude Code

The point of ctx is to run *inside the agent's loop*, not beside it. One command wires the whole
suite into Claude Code as hooks, with the guardrails already set:

```bash
ctx harness init --target claude          # local hooks in .claude/ (or --mode plugin for a shareable plugin)
```

This scaffolds three hooks and a starter `.ctx/rules.toml`:

- **SessionStart** → `ctx map` primes the agent with a codebase map before it does anything.
- **PostToolUse** (on Edit/Write) → `ctx index` reindexes, then `ctx check --against HEAD` flags any
  architecture violation the edit just introduced.
- **Stop** → `ctx score --fail-on "check_violations>0,new_duplication>0"`: a quality scorecard on
  the whole change before the agent calls it done.

The generated permissions let the agent run `ctx *` but **deny** `ctx self-update` and edits to the
rules, hooks, and settings, so an agent can't weaken the policy that governs it. `ctx harness
doctor` diagnoses the integration. Details in the
[Claude integration guide](https://docs.agentis.tools/docs/integrations/claude) and
[Using ctx with agents](https://docs.agentis.tools/docs/guides/using-ctx-with-agents).

### MCP server (Claude Desktop)

ctx can also expose the world model over the Model Context Protocol. MCP is **feature-gated** and not
in the default/release binaries, so build with the `mcp` feature:

```bash
cargo install agentis-ctx --features mcp
ctx serve --mcp
```

Configure Claude Desktop and see the available tools in the
[Claude integration guide](https://docs.agentis.tools/docs/integrations/claude).

## Command reference

The full flag reference for every command lives at
[docs.agentis.tools](https://docs.agentis.tools/). At a glance:

| | Command | Purpose |
|---|---|---|
| **Ground** | `index` / `embed` | Build the index; generate embeddings (`--watch` to keep them warm) |
| | `smart` | Select files for a task by semantic + call-graph relevance |
| | `map` | Token-budgeted architectural overview |
| | `diff` / `review` | Context for git changes / a GitHub PR |
| | `similar` | Find existing functions before writing new ones |
| | `search` / `semantic` / `query` / `source` / `explain` | Search and navigate the model |
| **Govern** | `check` | Enforce architecture rules from `.ctx/rules.toml` |
| | `score` | Composite quality gate for a change vs a git ref |
| | `duplicates` | MinHash near-duplicate detection |
| | `hotspots` | Churn × complexity refactoring targets |
| | `sql` | Read-only SQL over the `v1.*` views (and SQL gates) |
| **Integrate** | `harness` | Wire ctx into Claude Code (`init` / `doctor` / `compat`) |
| | `serve --mcp` | MCP server (requires the `mcp` feature) |
| | `shell` | Interactive REPL for exploring the codebase |
| | `self-update` | Update to the latest release (checksum-verified) |

## Supported Languages

| Language | Extensions | Symbol Extraction | Edge Types |
|----------|-----------|-------------------|------------|
| Rust | `.rs` | Functions, structs, enums, traits, impls | Calls, Implements, Imports |
| TypeScript | `.ts` | Functions, classes, interfaces, types, enums | Calls, Extends, Implements, Imports |
| TSX | `.tsx` | Functions, components, interfaces | Calls, Extends, Implements, Imports |
| JavaScript | `.js`, `.mjs`, `.cjs` | Functions, classes, arrow functions | Calls, Extends, Imports |
| JSX | `.jsx` | Functions, components | Calls, Extends, Imports |
| Python | `.py`, `.pyi` | Functions, classes, methods, constants | Calls, Extends, Imports |
| Go | `.go` | Functions, structs, interfaces, methods | Calls, Imports |
| Solidity | `.sol` | Contracts, functions, events, structs | Calls |
| YAML | `.yaml`, `.yml` | File tracking (no symbols) | N/A |

See [Language Support](https://docs.agentis.tools/docs/language-support) for detail.

## Using ctx as a library

Everything the CLI does is available as a Rust library, so you can embed indexing, search, and
context generation in your own tools. The package is `agentis-ctx`; the library target is named
`ctx`:

```toml
[dependencies]
agentis-ctx = "0.3"

# On Windows (or to skip DuckDB analytics):
# agentis-ctx = { version = "0.3", default-features = false }
```

```rust
use ctx::prelude::*;
use std::path::Path;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let root = Path::new("./my-project");

    // Build (or incrementally update) the index at .ctx/codebase.sqlite
    let mut indexer = Indexer::with_config(root, false, WalkerConfig::default())?;
    let result = indexer.index()?;
    println!("{} symbols extracted", result.symbols_extracted);

    // Keyword search over the indexed symbols
    let db = open_database(root)?;
    for symbol in db.find_symbols("authenticate", 10)? {
        println!("{} ({}:{})", symbol.name, symbol.file_path, symbol.line_start);
    }
    Ok(())
}
```

The [API documentation](https://docs.rs/agentis-ctx) covers the full surface: smart context
selection, diff-aware context, semantic search (local or OpenAI embeddings), call-graph analytics,
token counting, and output formatting.

## How it works

```
.ctx/
├── codebase.sqlite    # symbols, edges, embeddings, compressed source (FTS5 + sqlite-vec)
└── rules.toml         # your architecture rules (created by `ctx harness init`)
```

- **Tree-sitter** parses every supported language into symbols and relationship edges.
- **SQLite** (with FTS5 and `sqlite-vec`) is the persistent, single-file store.
- **DuckDB** runs the recursive graph and analytical queries (default-on; not available on Windows).
- **fastembed** generates local embeddings offline (all-MiniLM-L6-v2, 384-dim); **Ollama** (any local model) and **OpenAI** are optional via `--provider`.

Indexing respects `.gitignore`, an optional `.contextignore`, and 170+ built-in patterns. See
[Configuration](https://docs.agentis.tools/docs/configuration) and
[Architecture](https://docs.agentis.tools/docs/architecture).

| Variable | Description |
|----------|-------------|
| `OPENAI_API_KEY` | Required for `--provider openai` on `embed` / `semantic` / `smart` / `similar` |
| `OLLAMA_HOST` | Ollama server URL for `--provider ollama` (default `http://localhost:11434`) |
| `OLLAMA_EMBED_MODEL` | Ollama embedding model (default `nomic-embed-text`) |
| `GITHUB_TOKEN` | Optional for `review` (uses `gh` CLI auth by default) |
| `CTX_NO_UPDATE_CHECK` | Silence the passive "new release available" notice |

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on development
setup, coding style, and the pull request process.

## Security

To report a security vulnerability, see [SECURITY.md](SECURITY.md).

## License

This project is licensed under either of:

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
