# ctx

[![Crates.io](https://img.shields.io/crates/v/agentis-ctx)](https://crates.io/crates/agentis-ctx)
[![CI](https://github.com/agentis-tools/ctx/actions/workflows/ci.yml/badge.svg)](https://github.com/agentis-tools/ctx/actions)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](#license)
[![Rust Version](https://img.shields.io/badge/rust-1.91%2B-orange)](https://www.rust-lang.org)
[![Docs](https://img.shields.io/badge/docs-docs.agentis.tools-blue)](https://docs.agentis.tools/)

AI coding agents duplicate logic that already exists, break architecture boundaries, and report a
task as done without evidence that it works.

ctx is a local Rust binary that indexes a repository into a queryable model of its symbols, calls,
and dependencies. Agents and CI use that model for two jobs:

- **Ground the agent.** Give it a codebase map before it starts, then a selection of whole files
  that fits a token budget, instead of the whole repository. For scale: on the v0.4.0 tag
  (`0f4eea866055baa4e3e0aa4740a4b7e179a5779f`, 2026-07-24),
  `ctx --count-only --encoding cl100k_base` from the repository root reported 502,856
  file-content tokens for ctx's own repository with the default ignore rules. The count excludes formatter wrappers and
  the optional tree block, which add rendered-output tokens.
- **Govern its changes.** Show which symbols an edit affects, and fail the build or the agent
  turn when a change breaks the architecture rules in `.ctx/rules.toml`.

The rules are a file in your repository. The checks run locally against the index, and ctx sends
no code anywhere unless you opt in to a remote embedding provider or `ctx judge`.

Code-graph tools help an agent read code. Quality platforms usually run after the code is pushed.
ctx checks what the agent writes, on your machine, while the agent is still working.

Documentation: https://docs.agentis.tools/

## Install

From crates.io:

```bash
cargo install agentis-ctx
cargo binstall agentis-ctx
```

Package managers:

```bash
brew install agentis-tools/tap/ctx   # macOS (Intel or Apple Silicon), Linux x86-64
yay -S ctx-bin                       # Arch Linux x86-64
```

```powershell
scoop bucket add agentis-tools https://github.com/agentis-tools/scoop-bucket
scoop install ctx
```

Debian and RPM packages are attached to each
[GitHub release](https://github.com/agentis-tools/ctx/releases) and can be installed directly:

```bash
sudo apt install ./ctx_VERSION_amd64.deb
sudo dnf install ./ctx-VERSION-1.x86_64.rpm
```

The Homebrew tap, Scoop bucket, and AUR package are published from separate package-index
repositories. Prebuilt release binaries support Linux GNU x86-64, macOS Intel and Apple Silicon,
and Windows x86-64. Linux ARM64 and musl binaries are not currently published. The Windows and
Intel macOS binaries are built without DuckDB, so graph analytics return empty results there and
`ctx sql` and `ctx snapshot` exit 2 (`ctx sql --schema` still works).

### Upgrading

Use the same package manager that installed `ctx`:

| Installation | Upgrade command |
|---|---|
| Cargo | `cargo install agentis-ctx` |
| Homebrew | `brew upgrade ctx` |
| Arch/AUR | `yay -Syu ctx-bin` |
| Scoop | `scoop update ctx` |
| Debian | Install the newer `.deb` with `apt` |
| RPM | Install the newer `.rpm` with `dnf` |
| Direct GitHub download | `ctx self-update` |

`ctx self-update` checks for a package-manager installation before it touches the network, and
exits 2 with the right upgrade command instead of overwriting package-owned files. For direct
downloads it verifies the new executable against the release's `SHA256SUMS` before replacing the
old one.

If the DuckDB analytics backend doesn't compile on your machine (common on Windows with MSVC),
install without the default feature:

```bash
cargo install agentis-ctx --no-default-features
```

See the [Getting Started guide](https://docs.agentis.tools/docs/getting-started) for the full
platform matrix. Maintainers should use the [packaging and distribution runbook](docs/packaging.md)
for release assets, metadata generation, native-package validation, and external repository
publication.

## The loop: index, ground, govern

Every intelligence and governance command reads a prebuilt index, so run `ctx index` first. It
writes one file, `.ctx/codebase.sqlite`. On the v0.4.0 tag
(`0f4eea866055baa4e3e0aa4740a4b7e179a5779f`, 2026-07-24), running `ctx index` at the repository root with the default ignore rules
produced **2,701 symbols and 18,730 extracted edges**; counts vary with the checkout and indexing
configuration, and timing varies with the machine. After the first run, `ctx index --watch`
reindexes changed files as you save them.

```bash
ctx index                        # build the world model (or `ctx index --watch` in the background)
```

Ground the agent with context selected by embedding similarity and call-graph distance:

```bash
ctx smart "add rate limiting" --max-tokens 8000   # whole files for a task, within 8,000 rendered tokens
ctx diff --summary                                # changed files plus the symbols they depend on
ctx similar "retry with backoff"                  # look for an existing implementation before writing one
```

Govern what the agent changes with pass/fail gates:

```bash
ctx check --against origin/main                    # architecture rules, new violations only
ctx score --against main --fail-on "check_violations>0"   # composite scorecard with a gate
```

## One index, two jobs

The index holds symbols, call edges, other relationships (imports, implements, extends), and
optional embeddings. The same index selects what the agent reads and checks what it changes.

### Ground: context for the agent

| Command | What it does |
|---|---|
| [`ctx smart "<task>"`](https://docs.agentis.tools/docs/commands/smart) | Rank files by embedding similarity and call-graph distance, within a hard token budget |
| [`ctx map`](https://docs.agentis.tools/docs/commands/map) | Architectural overview within a token budget (PageRank over the symbol graph) |
| [`ctx diff`](https://docs.agentis.tools/docs/commands/diff) | Context for git changes, expanded to related symbols |
| [`ctx similar "<description>"`](https://docs.agentis.tools/docs/commands/similar) | Find existing functions before writing new ones, with their fan-in |
| `ctx search` / `ctx semantic` | Keyword (FTS5) and embedding-based symbol search |
| `ctx query impact / callers / deps / graph` | Walk the call graph: affected symbols, callers, dependencies |

Plain `ctx` also packs files for a model. Select files by path or glob and it streams them in a
prompt-ready format:

```bash
ctx src/ | pbcopy                 # copy source to the clipboard (XML by default)
ctx --format markdown src/        # or Markdown / JSON / plain
ctx --count-only src/             # just count tokens against your model's window
ctx --max-tokens 8000 "src/**/*.rs"
```

`ctx semantic`, `ctx smart`, and semantic `ctx similar` need embeddings. Generate them with
`ctx embed` and one of three providers: `--provider local` (the default, a ~90 MB fastembed
model), `--provider ollama` (any Ollama embedding model, local or remote), or `--provider openai`
(needs `OPENAI_API_KEY`). `ctx smart` exits 2 if the embeddings are missing or were built with a
different provider, model, or dimension. See
[Index and embed first](https://docs.agentis.tools/docs/guides/indexing).

### Govern: gates on what changes

| Command | What it does | Gate |
|---|---|---|
| [`ctx check`](https://docs.agentis.tools/docs/commands/check) | Enforce architecture rules from `.ctx/rules.toml` over the real edge graph | Exit 1 on any violation |
| [`ctx score`](https://docs.agentis.tools/docs/commands/score) | Change scorecard against a ref: check violations, new duplication, complexity and fan-out deltas | `--fail-on "<expr>"` |
| [`ctx duplicates`](https://docs.agentis.tools/docs/commands/duplicates) | MinHash near-duplicate detection over normalized token shingles | `--fail-on-found` |
| [`ctx hotspots`](https://docs.agentis.tools/docs/commands/hotspots) | Rank refactoring targets by churn times size (`--signal complexity` for churn times complexity) | informational |
| [`ctx sql`](https://docs.agentis.tools/docs/commands/sql) | Read-only SQL over the stable `v1.*` views for custom queries and gates | `--fail-on-rows` |

Architecture rules are a committed file, `.ctx/rules.toml`, that declares layers, forbidden
dependencies, and fan-in and complexity limits. `--against <ref>` limits a gate to what the diff
changed, so a pull request or an agent is judged on the violations it adds, not on existing ones.

CI and agents read results from the exit code. Every governance command uses the same codes:

| Code | Meaning |
|------|---------|
| `0` | Success, nothing to report |
| `1` | Ran successfully but produced findings (rule violations, gate hit) |
| `2` | Operational error (bad arguments, missing index, git failure) |
| `3` | Version requirement not met (reserved for `ctx harness compat --require`) |

See the [quality gates guide](https://docs.agentis.tools/docs/integrations/quality-gates) for CI
recipes and the [`--json` contract](https://docs.agentis.tools/docs/json-output).

## Use it in Claude Code or Codex

`ctx harness init` writes hooks that run ctx inside the agent's loop:

```bash
ctx harness init --target claude          # local hooks in .claude/ (or --mode plugin for a shareable plugin)
ctx harness init --target codex           # local hooks in .codex/ (also supports --mode plugin)
```

Released plugins are also available without generating them yourself:

```bash
# Claude Code: load the v0.4.0 release plugin for this session
claude --plugin-url https://github.com/agentis-tools/ctx/releases/download/v0.4.0/ctx-claude-plugin-0.4.0.zip

# Codex: add the canonical marketplace path and install ctx
codex plugin marketplace add agentis-tools/ctx --ref main --sparse plugins/codex/ctx
codex plugin add ctx@ctx-local
```

The canonical generated trees live at [`plugins/claude/ctx`](plugins/claude/ctx)
and [`plugins/codex/ctx`](plugins/codex/ctx). Both run locally and require the
`ctx` binary on `PATH`. MCP is optional and requires installing ctx with
`cargo install agentis-ctx --features mcp`; the default v0.4.0 plugin ZIPs do
not claim or configure MCP support.

`ctx harness init` writes three hooks and a starter `.ctx/rules.toml`:

- The SessionStart hook runs `ctx map` and gives the agent a codebase map before its first step.
- The PostToolUse hook (on Edit and Write) runs `ctx index`, then `ctx check --against HEAD`, and
  reports any architecture violation the edit introduced.
- The Stop hook runs `ctx score --against <default branch> --fail-on "check_violations>0"`.
  New duplication appears in the scorecard but doesn't fail the gate until you add it (v0.4.0
  hooks also gated on duplication; rerun `ctx harness init` after upgrading). The gate only
  reports; set `CTX_GATE_BLOCKING=1` to keep the session going until it passes.

The generated permissions let the agent run `ctx *` but deny `ctx self-update` and edits to the
rules, hooks, and settings, so the agent can't change the policy it is checked against.
`ctx harness doctor` diagnoses the integration. See the
[Claude integration guide](https://docs.agentis.tools/docs/integrations/claude) and
[Using ctx with agents](https://docs.agentis.tools/docs/guides/using-ctx-with-agents).

### MCP server (Claude Desktop)

ctx can serve the index over the Model Context Protocol. The release binaries and a default
`cargo install` don't include the MCP server, so build with the `mcp` feature:

```bash
cargo install agentis-ctx --features mcp
ctx serve --mcp
```

The [Claude integration guide](https://docs.agentis.tools/docs/integrations/claude) covers the
Claude Desktop configuration and the available tools.

## Command reference

The full flag reference for every command lives at
[docs.agentis.tools](https://docs.agentis.tools/). At a glance:

| | Command | Purpose |
|---|---|---|
| Ground | `index` / `embed` | Build the index and generate embeddings (`--watch` keeps both current) |
| | `smart` | Select files for a task by embedding similarity and call-graph distance |
| | `map` | Architectural overview within a token budget |
| | `diff` / `review` | Context for git changes or for a GitHub pull request |
| | `similar` | Find existing functions before writing new ones |
| | `search` / `semantic` / `query` / `source` / `explain` | Search and navigate the index |
| | `judge edges` | Opt-in: re-resolve ambiguous call edges with a decision model (`JEV_API_KEY`) |
| | `lsp` | Add, list, update, and health-check language servers for LSP extraction |
| Govern | `check` | Enforce architecture rules from `.ctx/rules.toml` |
| | `score` | Scorecard and gate for a change against a git ref |
| | `duplicates` | MinHash near-duplicate detection |
| | `hotspots` | Refactoring targets by churn times size (or complexity) |
| | `complexity` / `audit` / `graph` | Fan-out report, quality audit, dependency graph (DOT) |
| | `sql` | Read-only SQL over the `v1.*` views, with `--fail-on-rows` gates |
| | `snapshot` | Per-commit Parquet metrics for trends across history |
| Integrate | `harness` | Wire ctx into Claude Code or Codex (`init` / `doctor` / `compat`) |
| | `serve --mcp` | MCP server (requires the `mcp` feature) |
| | `shell` | Interactive REPL for exploring the index |
| | `self-update` | Update a direct GitHub installation, verified against `SHA256SUMS` |

## Supported languages

| Language | Extensions | Symbol extraction | Edge types |
|----------|-----------|-------------------|------------|
| Rust | `.rs` | Functions, structs, enums, traits, impls | Calls, Implements, Imports |
| TypeScript | `.ts` | Functions, classes, interfaces, types, enums | Calls, Extends, Implements, Imports |
| TSX | `.tsx` | Functions, components, interfaces | Calls, Extends, Implements, Imports |
| JavaScript | `.js`, `.mjs`, `.cjs` | Functions, classes, arrow functions | Calls, Extends, Imports |
| JSX | `.jsx` | Functions, components | Calls, Extends, Imports |
| Python | `.py`, `.pyi` | Functions, classes, methods, constants | Calls, Extends, Imports |
| Go | `.go` | Functions, structs, interfaces, methods | Calls, Imports |
| C | `.c`, `.h` when selected as C | Functions/prototypes, structs, unions, enums, typedefs, globals, macros | Calls, Includes |
| C++ | `.cc`, `.cpp`, `.cxx`, `.hh`, `.hpp`, `.hxx`, `.ipp`, `.tpp`, `.h` when selected as C++ | Functions, classes, methods, namespaces, aliases, globals, macros | Calls, Includes |
| Zig | `.zig` | Functions, container methods, structs, enums, types, constants, variables, named tests | Calls, Imports |
| Solidity | `.sol` | Contracts, functions, events, structs | Calls |
| YAML | `.yaml`, `.yml` | File tracking (no symbols) | N/A |

See [Language Support](https://docs.agentis.tools/docs/language-support) for detail.

### LSP extraction

Tree-sitter is the default extractor and runs locally. For languages or repositories
that need a language server to resolve references, configure an LSP backend with
`ctx lsp add <language>` and inspect it with `ctx lsp list` and `ctx lsp doctor`.
Each configured language can use `tree-sitter`, `lsp`, or `hybrid`; hybrid keeps
tree-sitter extraction and asks the server only to resolve references that stay
ambiguous or unresolved. For built-in languages, a failed server falls back to
tree-sitter; for languages without a built-in grammar, ctx keeps file records but
cannot extract server-backed symbols. In both cases indexing continues with a
warning. Hybrid mode waits up to `ready_timeout_ms` (default 300 s) for the server to finish
loading before it asks for definitions. `ctx lsp add` reads the registry over the network, and a
server may use the network too, so offline use needs a locally installed server and a committed
or hand-written configuration.

## Using ctx as a library

ctx is also a Rust library for indexing, search, and context generation in your own tools. The package is `agentis-ctx`; the library target is named
`ctx`:

```toml
[dependencies]
agentis-ctx = "0.4"

# On Windows (or to skip DuckDB analytics):
# agentis-ctx = { version = "0.4", default-features = false }
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
├── config.toml        # committed project defaults (optional; edit this)
├── rules.toml         # committed architecture policy (created by `ctx harness init`; edit this)
├── harness.lock       # generated harness ownership metadata (do not edit)
└── codebase.sqlite    # generated symbols, edges, embeddings, and compressed source
```

`.ctx/config.toml` holds project defaults for the embedding backend, language servers, and the
opt-in edge judge:

```toml
[embedding]
provider = "ollama"                 # local (default) | openai | ollama
model = "qwen3-embedding:8b"        # provider-specific model
# host = "http://localhost:11434"   # Ollama only

# [judge]
# edges = true                      # run `ctx judge edges` after every `ctx index` (needs JEV_API_KEY)
```

`[lsp.<language>]` sections configure language servers; `ctx lsp add` writes them for you.

Embedding settings resolve in this order: CLI flag, environment variable, `.ctx/config.toml`,
built-in default. They affect `ctx embed`, `semantic`, `smart`, and `similar`. Vectors from
different providers or models can't be compared, so after a switch run `ctx embed --force`, which
clears the stored corpus and rebuilds it.

`.ctx/rules.toml` defines the layers, forbidden dependencies, metric limits, and frozen paths used
by `ctx check` and the `check_violations` metric in `ctx score`. Create a commented starter with
`ctx harness init`, inspect it with `ctx check --list`, and run it with `ctx check`. See the
[rules-file reference](https://docs.agentis.tools/docs/commands/check#rules-file) for the complete
schema.

Commit `config.toml` and `rules.toml` so the team and CI share the same defaults and policy. Ignore
generated data such as `codebase.sqlite`; ctx can rebuild it with `ctx index`.

- Tree-sitter parses every supported language into symbols and relationship edges.
- SQLite, with FTS5 and `sqlite-vec`, stores everything in one file.
- DuckDB runs the recursive graph and analytical queries. It is a default feature, left out of
  the Windows and Intel macOS release binaries, and never downloads extensions: it attaches a locally installed `sqlite_scanner` or
  reads a private copy of the index, so these commands work offline.
- fastembed generates local embeddings offline (all-MiniLM-L6-v2, 384 dimensions). Ollama and
  OpenAI are optional through `--provider`.

Indexing respects `.gitignore`, an optional `.contextignore`, and 164 built-in ignore patterns. See
[Configuration](https://docs.agentis.tools/docs/configuration) and
[Architecture](https://docs.agentis.tools/docs/architecture).

| Variable | Description |
|----------|-------------|
| `OPENAI_API_KEY` | Required for `--provider openai` on `embed` / `semantic` / `smart` / `similar` |
| `OLLAMA_HOST` | Ollama server URL for `--provider ollama` (default `http://localhost:11434`) |
| `OLLAMA_EMBED_MODEL` | Ollama embedding model (default `nomic-embed-text`) |
| `JEV_API_KEY` | API key for `ctx judge edges` (read only from the environment) |
| `CTX_GATE_BLOCKING` | Set to `1` to make the generated Stop hook block until the gate passes |
| `CTX_GATE_LOG` | Append one JSONL record per `ctx score` gate evaluation (`1` = `.ctx/gate-log.jsonl`, or a path) |
| `GITHUB_TOKEN` | Optional for `review` (uses `gh` CLI auth by default) |
| `CTX_NO_UPDATE_CHECK` | Silence the passive "new release available" notice |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, coding style, and the pull request
process.

## Security

To report a security vulnerability, see [SECURITY.md](SECURITY.md).

## License

This project is licensed under either of:

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
