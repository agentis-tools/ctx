---
id: lsp
title: ctx lsp
sidebar_position: 15
---

# ctx lsp

Manage LSP-backed language support from the community registry.

## Synopsis

```bash
ctx lsp add <LANGUAGE> [--server <NAME>] [--yes] [--json]
ctx lsp list [--available] [--json]
ctx lsp update [LANGUAGE] [--yes] [--json]
ctx lsp doctor [--json]
```

## Description

The `lsp` command manages `[lsp.<language>]` blocks in `.ctx/config.toml` — the declarative registrations that let `ctx index` extract symbols through any stdio language server (see [Add a language via LSP](../lsp-languages.md)). `add` installs a curated entry from the community LSP registry, `list` shows what is configured (or available), `update` refreshes registry-installed entries, and `doctor` health-checks every configured server.

The registry lives in a separate repository, [`agentis-tools/ctx-lsp-registry`](https://github.com/agentis-tools/ctx-lsp-registry). ctx fetches its raw TOML manifests over HTTPS from the pinned `v1` branch (10-second timeout per fetch). Entries whose `schema_version` is newer than the running binary understands are rejected with a message telling you to upgrade ctx.

## Trust model

`ctx lsp add` **never executes anything**. It writes a TOML block to `.ctx/config.toml` — nothing else:

- The command, arguments, install hint, and homepage from the registry are **shown to you first**, and nothing is written until you confirm (`--yes` skips the prompt).
- The install hint (e.g. `npm install -g pyright`) is printed for you to run yourself; ctx never installs a server.
- The server binary itself is only ever executed later, by `ctx index` (when a matching file is indexed) or by `ctx lsp doctor` (as an explicit health probe).
- Hand-written `[lsp.<language>]` entries are never touched: `add` refuses to overwrite them and `update` refuses to refresh them. Only entries carrying the `source = "registry"` provenance key are managed by these commands.
- Config writes are format-preserving and atomic: comments, formatting, and unrelated tables in `.ctx/config.toml` survive byte-for-byte.

The registry base URL can be overridden with the `CTX_LSP_REGISTRY_BASE_URL` environment variable (for mirrors or testing). It is deliberately an environment variable and not a config key: where manifests come from is a distribution concern, not a per-project setting.

### Treat `.ctx/config.toml` as executable content

The flip side of declarative registration: `ctx index` **spawns whatever `[lsp.<language>]` blocks the current repository's `.ctx/config.toml` declares** — the configured `command` with its `args` and `env`, run with the repository as working directory — as soon as a matching file is indexed. The `add` confirmation prompt protects entries *you* write; it does not apply to a config that arrives already committed in a checkout.

Before running `ctx index` on a repository you don't trust, review its `.ctx/config.toml` like you would review a build script or git hook. Check the `command` **and** the `env` table — environment variables such as `LD_PRELOAD` or `DYLD_INSERT_LIBRARIES` can make even a familiar-looking binary load repository-local code.

## `add <LANGUAGE>`

Fetches the registry entry for `LANGUAGE`, shows the curated server, and writes `[lsp.<LANGUAGE>]` to `.ctx/config.toml` after confirmation:

```text
$ ctx lsp add python
Registry entry for python (https://raw.githubusercontent.com/agentis-tools/ctx-lsp-registry/v1/registry/python.toml):
  server:   pyright (recommended)
  command:  pyright-langserver --stdio
  install:  npm install -g pyright
  homepage: https://github.com/microsoft/pyright

This writes [lsp.python] to .ctx/config.toml (backend "hybrid").
Proceed? [y/N] y
wrote [lsp.python] to .ctx/config.toml (server pyright, backend hybrid)
run 'ctx index' to reindex with the new language server
```

- `--server <NAME>` picks a specific server from the entry instead of the registry's recommended one.
- `--yes` / `-y` skips the confirmation prompt. It is **required** in non-interactive use: when stdin is not a TTY, or with `--json` (JSON mode never prompts), the command refuses with exit code 2 instead of hanging.
- The written entry always uses `backend = "hybrid"` plus `source = "registry"` / `source_server = "<name>"` provenance keys so `ctx lsp update` can manage it later.
- If the server binary is not on `PATH` yet, the entry is still written and a stderr warning repeats the install hint and points at `ctx lsp doctor`.

Outcomes for an existing `[lsp.<LANGUAGE>]` entry:

| Existing entry | Result |
|----------------|--------|
| None | Written after confirmation |
| Registry-managed, identical | "already configured", exit 0, nothing written |
| Registry-managed, differs | Error pointing at `ctx lsp update <LANGUAGE>` (exit 2) |
| Hand-written (no `source = "registry"`) | Refused before any network call (exit 2) |

An unknown language is an error (exit 2) that lists every language the registry does offer.

## `list`

Shows the configured servers from `.ctx/config.toml`:

```text
$ ctx lsp list
kotlin: `kotlin-language-server` (backend lsp, source manual)
python: `pyright-langserver --stdio` (backend hybrid, source registry (pyright))
```

`source` is `registry (<server>)` for entries installed by `ctx lsp add`, `manual` for hand-written ones. With nothing configured it suggests `ctx lsp list --available`.

`--available` fetches the registry index instead and lists every language it covers, with the recommended server and a `[configured]` marker for languages already present in your config:

```text
$ ctx lsp list --available
Available languages in the LSP registry (https://raw.githubusercontent.com/agentis-tools/ctx-lsp-registry/v1):
  kotlin  (recommended server: kotlin-language-server)
  python  (recommended server: pyright)  [configured]

install one with 'ctx lsp add <language>'
```

## `update [LANGUAGE]`

Re-fetches each registry-managed entry (marked `source = "registry"`), diffs it key by key against your config, and rewrites it after confirmation:

```text
$ ctx lsp update
python: registry entry for server pyright changed:
  args: ["--stdio", "--verbose"] -> ["--stdio"]
Update [lsp.python]? [y/N] y
updated [lsp.python] in .ctx/config.toml
```

- Without `LANGUAGE`, every registry-managed entry is checked; entries already matching the registry report `up to date`.
- With `LANGUAGE`, only that entry is refreshed. Naming a hand-written entry is an error (exit 2) — `update` only manages registry-installed entries. Naming a language with no entry at all suggests `ctx lsp add <LANGUAGE>` (exit 2).
- The server named by the entry's `source_server` provenance key is re-fetched; when that key is missing or empty the registry's recommended server is used.
- `--yes` / `-y` applies all changes without prompting (required with `--json` or non-TTY stdin).
- Nothing registry-managed in the config is a clean no-op (exit 0).

## `doctor`

Probes every valid `[lsp.<language>]` entry — including hand-written ones — end to end:

1. **Binary lookup** — the `command` resolves to an executable (explicit path, or `PATH` search).
2. **Handshake** — the server is spawned and must complete the LSP `initialize` handshake.
3. **Capability diff** — every capability listed in the entry's `capabilities` array must be advertised by the server. Short names map to their provider keys (`documentSymbol` → `documentSymbolProvider`); names already ending in `Provider`, and `textDocumentSync`, are matched as-is.

```text
$ ctx lsp doctor
PASS python: `pyright-langserver` handshake ok (pyright 1.1.400), capabilities ok
FAIL kotlin: `kotlin-language-server` not found on PATH
     hint: install the server (see 'ctx lsp list --available'), then re-run 'ctx lsp doctor'

2 servers: 1 pass, 0 warn, 1 fail
```

Per server the status is `fail` (binary missing or handshake failed), `warn` (handshake ok but requested capabilities not advertised — extraction may be degraded), or `pass`. The exit code is 1 when at least one server **fails**; warnings alone still exit 0. Failed handshakes include the server's recent stderr lines. With nothing configured, `doctor` is a clean no-op.

`doctor` probes servers on demand; the related sidecar `.ctx/lsp_status.json` records what actually happened during the last `ctx index` run (see [Add a language via LSP — Troubleshooting](../lsp-languages.md#troubleshooting)).

## JSON output

All four subcommands support the global `--json` flag and emit the standard envelope (see [JSON output](../json-output.md)). `add` and `update` additionally require `--yes` in JSON mode, because JSON mode never prompts.

```bash
ctx lsp add python --yes --json
```

```json
{
  "ctx_version": "0.3.5",
  "command": "lsp.add",
  "generated_at": "2026-07-15T12:00:00Z",
  "data": {
    "language": "python",
    "server": "pyright",
    "command": "pyright-langserver",
    "args": ["--stdio"],
    "backend": "hybrid",
    "source": "registry",
    "registry_url": "https://raw.githubusercontent.com/agentis-tools/ctx-lsp-registry/v1/registry/python.toml",
    "install_hint": "npm install -g pyright",
    "homepage": "https://github.com/microsoft/pyright",
    "binary_found": false,
    "status": "added"
  }
}
```

When the entry already matches the registry, `data` is just `{"language", "server", "status": "already_configured"}`.

```bash
ctx lsp doctor --json
```

```json
{
  "ctx_version": "0.3.5",
  "command": "lsp.doctor",
  "generated_at": "2026-07-15T12:00:00Z",
  "data": {
    "healthy": false,
    "summary": { "pass": 1, "warn": 0, "fail": 1 },
    "servers": [
      {
        "language": "kotlin",
        "command": "kotlin-language-server",
        "backend": "hybrid",
        "binary_found": false,
        "root_markers_found": [],
        "handshake_ok": false,
        "negotiated_capabilities": [],
        "missing_capabilities": ["documentSymbol"],
        "stderr": [],
        "error": "`kotlin-language-server` not found on PATH",
        "status": "fail"
      }
    ]
  }
}
```

`lsp.list` reports `{"available": false, "servers": [...]}` (configured entries) or `{"available": true, "registry": "<base url>", "languages": [...]}` (with `--available`). `lsp.update` reports `{"registry": "<base url>", "languages": [{"language", "server", "status": "up_to_date" | "updated", "changes": {"<key>": {"from", "to"}}}]}`.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success — including "already configured" (`add`), "up to date" (`update`), and warnings-only `doctor` runs |
| 1 | `doctor` found at least one failing server (missing binary or failed handshake) |
| 2 | Operational error (network failure, unknown language, refused overwrite, aborted confirmation, prompt needed but stdin is not a TTY or `--json` was given without `--yes`) |

## Examples

### Install and verify a language server

```bash
ctx lsp list --available          # what does the registry offer?
ctx lsp add python                # review the entry, confirm, write config
npm install -g pyright            # run the printed install hint yourself
ctx lsp doctor                    # binary found? handshake ok? capabilities?
ctx index                         # reindex through the new server
```

### Pick a non-default server

```bash
ctx lsp add python --server pylsp
```

### Keep registry entries fresh

```bash
ctx lsp update            # diff + confirm every registry-managed entry
ctx lsp update python -y  # refresh one entry without prompting
```

### Non-interactive / CI use

```bash
ctx lsp add python --yes --json
ctx lsp doctor --json     # exit 1 signals an unhealthy server
```

### Use a registry mirror

```bash
CTX_LSP_REGISTRY_BASE_URL=https://mirror.example.com/ctx-lsp-registry ctx lsp add python
```

## See Also

- [Add a language via LSP](../lsp-languages.md) — the `[lsp.<language>]` config reference, backend modes, and troubleshooting
- [Language Support](../language-support.md) — built-in tree-sitter languages
- [Configuration](../configuration.md) — `.ctx/config.toml`
- [ctx-lsp-registry](https://github.com/agentis-tools/ctx-lsp-registry) — contribute new language entries via that repository's `CONTRIBUTING.md`
