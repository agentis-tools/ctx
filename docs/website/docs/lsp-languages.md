---
id: lsp-languages
title: Add a Language via LSP
sidebar_position: 7
---

# Add a Language via LSP

ctx ships tree-sitter grammars for a fixed set of languages ([Language Support](language-support.md)). For everything else — Kotlin, Scala, Ruby, Zig, anything with a stdio language server — you can register the server declaratively in `.ctx/config.toml` and `ctx index` extracts symbols, call edges, and cross-file references through it. No plugin, no rebuild.

## Quick start

For languages the community registry covers, one command sets everything up:

```bash
ctx lsp add python        # shows the curated entry, asks for confirmation
ctx lsp doctor            # verify: binary found, handshake ok, capabilities ok
ctx index                 # reindex through the server
```

`ctx lsp add` writes a `[lsp.<language>]` block to `.ctx/config.toml` and nothing more — it never installs or executes the server itself. If the server binary is missing it prints the install command for you to run. See [`ctx lsp`](commands/lsp.md) for the full command reference and trust model.

## Manual registration

For languages not in the registry, write the block yourself:

```toml
# .ctx/config.toml
[lsp.kotlin]
command = "kotlin-language-server"
extensions = ["kt", "kts"]        # required: kotlin has no built-in grammar
backend = "lsp"

[lsp.python]
command = "pyright-langserver"
args = ["--stdio"]
# extensions default to the built-in set ("py", "pyi") for built-in language names
# backend defaults to "hybrid"
```

The table key (`kotlin`, `python`, …) is the language name stored on every indexed file and symbol; it flows through `ctx query`, `ctx sql`, and JSON output unchanged.

### Key reference

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `command` | string | — (required) | Executable to spawn. Resolved via `PATH` unless it contains a path separator. |
| `args` | array of strings | `[]` | Arguments passed to the server (e.g. `["--stdio"]`). |
| `extensions` | array of strings | built-in set for built-in language names | File extensions (without the dot) this server claims. **Required** when the table key is not a built-in language name. Normalized to lowercase, leading dots stripped. |
| `root_markers` | array of strings | `[]` | Marker files/dirs identifying a workspace root. Informational; `ctx lsp doctor` reports which ones exist under the project root. |
| `capabilities` | array of strings | `[]` | Capabilities you expect the server to provide (e.g. `["documentSymbol", "definition"]`). `ctx lsp doctor` warns when the server does not advertise them. |
| `backend` | `"tree-sitter"` \| `"lsp"` \| `"hybrid"` | `"hybrid"` | Extraction backend for files claimed by this block (see below). |
| `initialization_options` | any TOML value | unset | Passed through verbatim as LSP `initializationOptions`. |
| `env` | table of strings | `{}` | Extra environment variables for the server process. |
| `timeout_ms` | integer | `10000` | Per-request timeout in milliseconds. |
| `source`, `source_server` | string | unset | Provenance keys written by `ctx lsp add`; accepted and ignored by the indexer. Do not set them by hand — `source = "registry"` marks an entry as managed by `ctx lsp update`. |

Built-in language names with default extension sets: `rust` (`rs`), `typescript` (`ts`), `tsx` (`tsx`), `javascript` (`js`, `mjs`, `cjs`), `jsx` (`jsx`), `python` (`py`, `pyi`), `go` (`go`), `solidity` (`sol`), `yaml` (`yaml`, `yml`).

When two blocks claim the same extension, the first block (in table-key order) wins and a warning names both blocks.

## Backend modes

| `backend` | Symbols and edges from | Cross-file resolution | Use when |
|-----------|------------------------|-----------------------|----------|
| `tree-sitter` | Built-in grammar only | SQL name resolution only | You want to keep a registration around without using it. On a language with no built-in grammar this leaves the files unindexed. |
| `lsp` | The language server (`textDocument/documentSymbol`, call hierarchy) | `textDocument/definition` | The language has no built-in grammar, or you want the server's semantic view (e.g. resolved call hierarchies) even for a built-in language. |
| `hybrid` (default) | Built-in tree-sitter grammar | `textDocument/definition` for references tree-sitter left unresolved | The language has a built-in grammar. You keep tree-sitter's fast, offline extraction and add LSP precision only where static name resolution was ambiguous (e.g. the same function name defined in several files). |

Two details worth knowing:

- **`hybrid` on a dynamic language degrades to `lsp`.** With no built-in grammar there is nothing for tree-sitter to extract, so hybrid-configured blocks for non-built-in languages use full LSP extraction automatically. In practice: the default backend does the right thing for both kinds of language.
- **Call edges via LSP need call hierarchy.** In `lsp` mode, `Calls` edges are collected through `callHierarchy/outgoingCalls` and are only gathered when the server advertises `callHierarchyProvider`. Symbols still come through without it.

## Fallback and degradation

LSP failures never fail an indexing run, and the exit code is unaffected:

- **Server missing or crashing:** a warning is printed to stderr once per language per run, then ctx falls back — built-in languages are re-parsed with tree-sitter (full symbols), dynamic languages get a file record with zero symbols (so incremental indexing and deletion cleanup keep working).
- **Invalid config blocks** (empty `command`, missing `extensions` for a non-built-in language) are skipped with a warning; the remaining blocks and built-in grammars keep working. Unknown keys are tolerated, so configs written by newer ctx versions still load.
- **Slow servers:** each request gets `timeout_ms` (default 10 s); the first request after `initialize` gets a longer warmup grace period, because many servers index the workspace before answering. After 3 consecutive timeouts the server is declared failed for the rest of the run and the fallback above kicks in.

## Performance

- **Zero cost when unconfigured.** Without any `[lsp.*]` block, the subsystem is completely inert — nothing is spawned and indexing behaves exactly as before.
- **Per-language opt-in.** Only files whose extension is claimed by a block go through a server; everything else stays on tree-sitter.
- **Lazy spawn.** A server is started on the first file that needs it, not at startup. An incremental `ctx index` run that finds no changed files in that language never spawns the server at all.
- **Warm reuse in watch mode.** `ctx index --watch` keeps servers running across file events; a one-shot `ctx index` shuts them down at the end of the run.
- **First-request warmup.** Expect the first indexed file per server to be slower while the server warms up; subsequent files reuse the running process.

`hybrid` is the cheaper mode for built-in languages: tree-sitter does the bulk extraction and the server is only consulted for edges that stayed unresolved.

## Troubleshooting

Start with the health probe:

```bash
ctx lsp doctor
```

It checks, per configured block: the binary resolves (explicit path or `PATH`), the server completes the `initialize` handshake, and every capability listed in `capabilities` was advertised — with the server's recent stderr attached to failed handshakes. Exit code 1 means at least one server failed. See [`ctx lsp doctor`](commands/lsp.md#doctor).

Every `ctx index` run with LSP configured also writes a sidecar, `.ctx/lsp_status.json`, recording what actually happened:

```json
{
  "generated_at": 1752580800,
  "servers": [
    {
      "language": "kotlin",
      "command": "kotlin-language-server",
      "backend": "lsp",
      "state": "healthy",
      "server_name": "kotlin-language-server",
      "server_version": "1.3.13",
      "capabilities": ["documentSymbolProvider", "definitionProvider"]
    }
  ]
}
```

`state` is `healthy`, `failed` (with a `reason`), or `idle` (configured, but no file needed the server this run). The sidecar lives in `.ctx/`, which is never indexed, so it never shows up in query results.

Common cases:

| Symptom | Explanation |
|---------|-------------|
| `Warning: LSP server '…' for … …; falling back to tree-sitter` during `ctx index` | The server failed to spawn, crashed, or timed out repeatedly. Indexing continued on the fallback path; run `ctx lsp doctor` for details. |
| `Warning: ignoring [lsp.<lang>] …: 'extensions' is required for non-builtin languages` | The block's table key is not a built-in language name, so ctx cannot infer which files it claims. Add `extensions = [...]`. |
| Doctor says `WARN … missing capabilities: …` | The server negotiated fewer capabilities than the config requests; extraction may be degraded (e.g. no call edges without `callHierarchy`). |
| Dynamic-language files indexed but have no symbols | The server was unavailable, so only file records were stored. Fix the server (see doctor) and re-run `ctx index` after touching the files, or `ctx index --force`. |

## Security note

`ctx index` spawns the servers the current repository's `.ctx/config.toml` declares — command, args, and `env` — with the repository as working directory. A checkout you didn't write can therefore name any executable. Before indexing an untrusted repository, review its `.ctx/config.toml` like a build script; see the [trust model](commands/lsp.md#trust-model) for details.

## Contributing a registry entry

The curated entries behind `ctx lsp add` live in [`agentis-tools/ctx-lsp-registry`](https://github.com/agentis-tools/ctx-lsp-registry): one TOML file per language, each with one or more `[[servers]]` blocks (exactly one marked `recommended = true`) plus per-OS install hints. If you have a working manual `[lsp.<language>]` block for a language the registry does not cover yet, consider contributing it upstream — see that repository's `CONTRIBUTING.md`.

## See Also

- [`ctx lsp`](commands/lsp.md) — command reference (`add`, `list`, `update`, `doctor`)
- [Language Support](language-support.md) — the built-in tree-sitter languages
- [Configuration](configuration.md) — everything else in `.ctx/config.toml`
