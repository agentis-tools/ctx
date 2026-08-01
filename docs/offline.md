# Offline operation

The index, keyword search, tree-sitter extraction, and local Ollama embeddings
run without sending source code to ctx. Two first-use artifacts need to be
staged when a machine has no network access: the fastembed model and, when
DuckDB analytics are enabled, DuckDB's `sqlite_scanner` extension.

## Stage local embeddings

The default local provider uses fastembed's `all-MiniLM-L6-v2` model. On a
connected staging machine, choose a cache directory and run embedding once:

```bash
export FASTEMBED_CACHE_DIR="$PWD/.cache/ctx-fastembed"
ctx index
ctx embed --provider local
```

Copy that cache directory to the offline machine and set the same variable
before running `ctx embed`, `ctx semantic`, `ctx smart`, or `ctx similar`.
`FASTEMBED_CACHE_DIR` is the fastembed cache setting; it is separate from the
project's `.ctx/codebase.sqlite` index. The first local embedding run may
download about 90 MB of model files.

Ollama can also be staged locally: pull the selected model while connected,
then run Ollama on the offline machine and set `OLLAMA_HOST` and
`OLLAMA_EMBED_MODEL`. The OpenAI provider is network-backed and is not an
offline option.

## Stage DuckDB analytics

The default release build includes bundled DuckDB 1.10504 on Linux and macOS. When a
DuckDB command first attaches `.ctx/codebase.sqlite`, DuckDB may need its
`sqlite_scanner` extension. On a connected machine with a DuckDB CLI matching
the version bundled by ctx, install that extension into a directory you can
copy to the offline machine. Check `duckdb --version` before staging; an
extension built for another DuckDB release may not load:

```bash
mkdir -p "$PWD/.cache/ctx-duckdb-extensions"
duckdb -c "SET extension_directory='$PWD/.cache/ctx-duckdb-extensions'; INSTALL sqlite_scanner; LOAD sqlite_scanner;"
```

Copy the directory and run ctx with automatic extension installation disabled:

```bash
export CTX_DUCKDB_EXTENSION_DIRECTORY="$PWD/.cache/ctx-duckdb-extensions"
export CTX_DUCKDB_OFFLINE=1
ctx sql "SELECT * FROM v1.files LIMIT 10"
```

In offline mode ctx explicitly loads the staged extension and refuses network
installation. If it is missing, the command exits with an error naming
`sqlite_scanner` and the staging variables. Without `CTX_DUCKDB_OFFLINE=1`,
DuckDB keeps its normal automatic extension behavior.

Windows release artifacts are built without the optional DuckDB feature, so
they do not need this extension staging step. The official release matrix
does include both Intel (`x86_64-apple-darwin`) and Apple Silicon macOS
artifacts; use a matching DuckDB extension build for the target architecture.
Source builds that need the smallest/offline footprint can use
`--no-default-features`, which omits DuckDB analytics.

## Verify before disconnecting

Run the real commands on the staging machine, then repeat them with network
access disabled on the target machine:

```bash
ctx index
ctx embed --provider local
ctx smart --count-only --max-tokens 2000
ctx sql "SELECT count(*) FROM v1.symbols"
```

Keep the staged cache and extension directory versioned or checksummed with
the same ctx release and target architecture. The SQLite index itself is
portable as a file, but it should still be rebuilt with `ctx index` when the
checkout changes.
