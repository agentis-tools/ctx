---
id: offline
title: Offline operation
sidebar_position: 3
---

# Offline operation

Indexing, keyword search, tree-sitter extraction, and local Ollama embeddings do
not send source code to ctx. Two first-use artifacts must be staged before an
air-gapped run: the fastembed model and, for DuckDB analytics, DuckDB's
`sqlite_scanner` extension.

## Stage local embeddings

The default local provider uses fastembed's `all-MiniLM-L6-v2` model. On a
connected staging machine:

```bash
export FASTEMBED_CACHE_DIR="$PWD/.cache/ctx-fastembed"
ctx index
ctx embed --provider local
```

Copy the cache directory to the offline machine and keep the same variable set.
If model files are missing, `ctx embed` exits with a model error; it does not
produce an incomplete embedding corpus. Ollama can be staged by pulling the
selected model while connected and running Ollama locally. The OpenAI provider
is network-backed and is not an offline option.

## Stage DuckDB analytics

The bundled DuckDB engine is v1.5.4 (`duckdb-rs` 1.10504.0). On a connected
staging machine, use a DuckDB CLI reporting v1.5.4 and install the extension:

```bash
duckdb --version
mkdir -p "$PWD/.cache/ctx-duckdb-extensions"
duckdb -c "SET extension_directory='$PWD/.cache/ctx-duckdb-extensions'; INSTALL sqlite_scanner; LOAD sqlite_scanner;"
```

Copy the extension directory to the offline machine. Record and verify its
contents with the platform's SHA-256 tool (for example, `shasum -a 256` on
macOS):

```bash
(cd .cache/ctx-duckdb-extensions && find . -type f ! -name SHA256SUMS -exec shasum -a 256 {} + | sort > SHA256SUMS)
(cd .cache/ctx-duckdb-extensions && shasum -a 256 -c SHA256SUMS)
```

Run ctx with automatic extension installation disabled and passive update
checks suppressed:

```bash
export CTX_DUCKDB_EXTENSION_DIRECTORY="$PWD/.cache/ctx-duckdb-extensions"
export CTX_DUCKDB_OFFLINE=1
export CTX_OFFLINE=1
ctx sql "SELECT * FROM v1.files LIMIT 10"
```

Offline mode explicitly loads the staged extension and fails with an error that
names `sqlite_scanner` when the artifact is absent. `CTX_OFFLINE=1` also prevents
the common post-command update notice from attempting a network request.

Windows release artifacts are built without DuckDB, while Intel and Apple
Silicon macOS artifacts have separate target builds. Source builds that need a
smaller footprint can use `--no-default-features`, but DuckDB-backed commands
then exit with an actionable feature error rather than returning empty results.

## Verify before disconnecting

Run the real commands on the staging machine, then repeat them with network
access disabled:

```bash
ctx index
ctx embed --provider local
ctx smart --count-only --max-tokens 2000
ctx sql "SELECT count(*) FROM v1.symbols"
```

The SQLite index is portable as a file, but rebuild it with `ctx index` when the
checkout changes.
