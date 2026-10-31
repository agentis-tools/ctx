---
id: lsp-languages
title: Add a language via LSP
sidebar_position: 7
---

# Add a language via LSP

ctx ships tree-sitter grammars for a fixed set of languages ([Language support](language-support.md)). For other languages, such as Kotlin, Scala, or Ruby, register any stdio language server in `.ctx/config.toml`. `ctx index` then extracts symbols, call edges, and cross-file references through it, without a plugin or a rebuild.

## Quick start

For languages the community registry covers, one command writes the configuration:

```bash
ctx lsp add python        # shows the curated entry, asks for confirmation
ctx lsp doctor            # checks: binary found, handshake ok, capabilities ok
ctx index                 # reindex through the server
```

`ctx lsp add` only writes a `[lsp.<language>]` block to `.ctx/config.toml`. It never installs or runs the server. If the server binary is missing, it prints the install command for you to run. [`ctx lsp`](commands/lsp.md) has the full command reference and trust model.

## Manual registration

For languages the registry does not cover, write the block yourself:

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

The table key (`kotlin`, `python`, and so on) is the language name stored on every indexed file and symbol. `ctx query`, `ctx sql`, and JSON output report it unchanged.

### Key reference

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `command` | string | none (required) | Executable to start. ctx looks it up on `PATH` unless it contains a path separator. |
| `args` | array of strings | `[]` | Arguments passed to the server (for example `["--stdio"]`). |
| `extensions` | array of strings | built-in set for built-in language names | File extensions (without the dot) this server claims. Required when the table key is not a built-in language name. ctx lowercases them and strips leading dots. |
| `root_markers` | array of strings | `[]` | Files or directories that mark a workspace root. Informational: `ctx lsp doctor` reports which exist under the project root. |
| `capabilities` | array of strings | `[]` | Capabilities you expect the server to provide (for example `["documentSymbol", "definition"]`). `ctx lsp doctor` warns when the server does not advertise them. |
| `backend` | `"tree-sitter"` \| `"lsp"` \| `"hybrid"` | `"hybrid"` | Extraction backend for files this block claims (see below). |
| `initialization_options` | any TOML value | unset | Passed unchanged as LSP `initializationOptions`. |
| `env` | table of strings | `{}` | Extra environment variables for the server process. |
| `timeout_ms` | integer | `10000` | Per-request timeout in milliseconds. |
| `ready_timeout_ms` | integer | `300000` | How long cross-file resolution waits for the server to report that it has loaded the workspace (`experimental/serverStatus` reports quiescent, or every `$/progress` report has ended). Servers such as rust-analyzer answer `null` to definition requests until then. |
| `source`, `source_server` | string | unset | Provenance keys written by `ctx lsp add`; the indexer ignores them. Do not set them by hand: `source = "registry"` lets `ctx lsp update` manage the entry. |

Built-in language names with default extension sets: `rust` (`rs`), `typescript` (`ts`), `tsx` (`tsx`), `javascript` (`js`, `mjs`, `cjs`), `jsx` (`jsx`), `python` (`py`, `pyi`), `go` (`go`), `solidity` (`sol`), `yaml` (`yaml`, `yml`). C, C++, and Zig have built-in grammars but no default extension set here, so an `[lsp.c]`, `[lsp.cpp]`, or `[lsp.zig]` block must list `extensions`.

When two blocks claim the same extension, the first block in table-key order wins and a warning names both.

## Backend modes

| `backend` | Symbols and edges from | Cross-file resolution | Use when |
|-----------|------------------------|-----------------------|----------|
| `tree-sitter` | Built-in grammar only | SQL name resolution only | You want to keep a registration without using it. For a language without a built-in grammar, its files stay unindexed. |
| `lsp` | The language server (`textDocument/documentSymbol`, call hierarchy) | `textDocument/definition` | The language has no built-in grammar, or you want the server's semantic view (such as resolved call hierarchies) for a built-in language too. |
| `hybrid` (default) | Built-in tree-sitter grammar | `textDocument/definition` for references tree-sitter left unresolved | The language has a built-in grammar. tree-sitter does the fast, offline extraction, and the server resolves only the references that static name resolution could not (for example, a function name defined in several files). |

Two details:

- For a language without a built-in grammar, `hybrid` behaves like `lsp`, because tree-sitter has nothing to extract. The default backend therefore works for both kinds of language.
- In `lsp` mode, ctx collects call edges through `callHierarchy/outgoingCalls`, and only when the server advertises `callHierarchyProvider`. Without it, ctx still gets symbols but no call edges.

## Fallback and degradation

LSP failures never make an indexing run fail, and they do not change the exit code:

- If a server is missing or crashes, ctx prints one warning per language per run and falls back. Built-in languages are parsed again with tree-sitter (full symbols). Other languages get a file record with no symbols, so incremental indexing and deletion cleanup keep working.
- ctx skips an invalid block (empty `command`, or no `extensions` for a non-built-in language) with a warning. The other blocks and the built-in grammars keep working. Unknown keys are allowed, so configs written by newer ctx versions still load.
- Each request gets `timeout_ms` (default 10 s). The first request after `initialize` gets a longer grace period, because many servers index the workspace before they answer. After 3 timeouts in a row, ctx treats the server as failed for the rest of the run and uses the fallback above.

## Performance

- Without any `[lsp.*]` block, ctx starts no server and indexing is unchanged.
- Only files whose extension a block claims go through a server. Everything else stays on tree-sitter.
- ctx starts a server for the first file that needs it, not at startup. An incremental `ctx index` that finds no changed files in that language never starts the server.
- `ctx index --watch` keeps servers running across file events. A one-shot `ctx index` stops them at the end of the run.
- The first indexed file per server is slower while the server loads; later files reuse the running process.

`hybrid` costs less for built-in languages: tree-sitter does most of the extraction and ctx asks the server only about edges that stayed unresolved.

## Troubleshooting

Start with the health check:

```bash
ctx lsp doctor
```

For each configured block, it checks that the binary resolves (explicit path or `PATH`), that the server completes the `initialize` handshake, and that the server advertised every capability listed in `capabilities`. A failed handshake includes the server's recent stderr. Exit code 1 means at least one server failed. See [`ctx lsp doctor`](commands/lsp.md#doctor).

Every `ctx index` run with LSP configured also writes `.ctx/lsp_status.json`, which records what happened:

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

`state` is `healthy`, `failed` (with a `reason`), or `idle` (configured, but no file needed the server in this run). ctx never indexes `.ctx/`, so the file does not appear in query results.

Common cases:

| Symptom | Explanation |
|---------|-------------|
| `Warning: LSP server '…' for … …; falling back to tree-sitter` during `ctx index` | The server failed to start, crashed, or timed out repeatedly. Indexing continued on the fallback path. Run `ctx lsp doctor` for details. |
| `Warning: ignoring [lsp.<lang>] …: 'extensions' is required for non-builtin languages` | The table key is not a built-in language name, so ctx cannot tell which files the block claims. Add `extensions = [...]`. |
| Doctor reports `WARN … missing capabilities: …` | The server offers fewer capabilities than the config lists. Extraction may be incomplete (for example, no call edges without `callHierarchy`). |
| Files of a language without a grammar are indexed but have no symbols | The server was unavailable, so ctx stored only file records. Fix the server (see doctor), then touch the files and run `ctx index` again, or run `ctx index --force`. |
| Hybrid mode resolves few or no cross-file references on a large workspace | The server had not finished loading within `ready_timeout_ms`, and ctx continued without it. `ctx index -v` prints whether the server reported ready. Raise `ready_timeout_ms`. |

## Security note

`ctx index` starts the servers that the repository's `.ctx/config.toml` declares (command, args, and `env`), with the repository as the working directory. A checkout you did not write can therefore name any executable. Before indexing an untrusted repository, review its `.ctx/config.toml` as you would a build script. See the [trust model](commands/lsp.md#trust-model).

## Contributing a registry entry

The curated entries behind `ctx lsp add` live in [`agentis-tools/ctx-lsp-registry`](https://github.com/agentis-tools/ctx-lsp-registry): one TOML file per language, each with one or more `[[servers]]` blocks (exactly one marked `recommended = true`) and per-OS install hints. If you have a working `[lsp.<language>]` block for a language the registry does not cover, you can contribute it there; see that repository's `CONTRIBUTING.md`.

## See also

- [`ctx lsp`](commands/lsp.md): command reference (`add`, `list`, `update`, `doctor`)
- [Language support](language-support.md): the built-in tree-sitter languages
- [Configuration](configuration.md): the rest of `.ctx/config.toml`
