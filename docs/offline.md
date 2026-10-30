# Offline operation

Indexing, keyword search, tree-sitter extraction, DuckDB analytics, and local
Ollama embeddings do not send source code anywhere. Only the local embedding
model has to be staged before an air-gapped run; a DuckDB extension can be
staged as an optional speed-up.

## Stage local embeddings

The default local provider uses fastembed's `all-MiniLM-L6-v2` model. The first
local embedding run downloads about 90 MB of model files. On a connected
staging machine, choose a cache directory and run embedding once:

```bash
export FASTEMBED_CACHE_DIR="$PWD/.cache/ctx-fastembed"
ctx index
ctx embed --provider local
```

Copy that cache directory to the offline machine and set the same variable
before running `ctx embed`, `ctx semantic`, `ctx smart`, or `ctx similar`.
`FASTEMBED_CACHE_DIR` is fastembed's cache setting; it is separate from the
project's `.ctx/codebase.sqlite` index. If the model files are missing, ctx
exits with a model error; it does not produce an incomplete embedding corpus.

Ollama can also be staged: pull the model while connected, then run Ollama on
the offline machine and set `OLLAMA_HOST` and `OLLAMA_EMBED_MODEL`. The OpenAI
provider is network-backed and is not an offline option.

## DuckDB analytics

DuckDB-backed commands (`ctx sql`, `query`, `snapshot`, `smart`, `diff`) never
download anything. DuckDB reads the SQLite index through its `sqlite_scanner`
extension, which is not bundled; ctx disables DuckDB's automatic extension
installation and, when the extension is not installed, copies the index into a
private read-only DuckDB file instead. These commands therefore work offline
with no staging.

On large indexes the copy adds start-up time. To attach the index directly,
stage the extension on a connected machine with a DuckDB CLI of the same
version ctx bundles (an extension built for another DuckDB release does not
load), then point ctx at the directory:

```bash
mkdir -p "$PWD/.cache/ctx-duckdb-extensions"
duckdb -c "SET extension_directory='$PWD/.cache/ctx-duckdb-extensions'; INSTALL sqlite_scanner;"
(cd .cache/ctx-duckdb-extensions && find . -type f ! -name SHA256SUMS -exec shasum -a 256 {} + | sort > SHA256SUMS)
```

On the offline machine, verify the checksums and set the directory:

```bash
(cd .cache/ctx-duckdb-extensions && shasum -a 256 -c SHA256SUMS)
export CTX_DUCKDB_EXTENSION_DIRECTORY="$PWD/.cache/ctx-duckdb-extensions"
```

Windows and Intel macOS release artifacts are built without DuckDB. Source
builds can omit it with `--no-default-features`; DuckDB-backed commands then
exit with an actionable feature error instead of returning empty results.

## Suppress the update check

After a command, ctx may check GitHub for a newer release. Set `CTX_OFFLINE=1`
(or `CTX_NO_UPDATE_CHECK=1`) so an offline run makes no network request at all.

## Verify before disconnecting

Run the real commands on the staging machine, then repeat them with network
access disabled on the target machine:

```bash
export CTX_OFFLINE=1
ctx index
ctx embed --provider local
ctx smart --count-only --max-tokens 2000
ctx sql "SELECT count(*) FROM v1.symbols"
```

Keep the staged model cache (and extension directory, if used) with the ctx
release and target architecture it was made for. The SQLite index is portable
as a file, but rebuild it with `ctx index` when the checkout changes.
