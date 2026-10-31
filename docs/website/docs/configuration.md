---
id: configuration
title: Configuration
sidebar_position: 5
---

# Configuration

ctx reads these sources:
1. `.contextignore`: project-specific ignore patterns
2. `.gitignore`: standard git ignores (respected by default)
3. 164 built-in ignore patterns (enabled by default)
4. command-line flags
5. environment variables: API keys, embedding hosts, and gate switches
6. `.ctx/config.toml`: optional committed project defaults for embeddings, model-assisted edge
   resolution, and language servers
7. `.ctx/rules.toml`: the committed architecture policy for `ctx check` and `ctx score`

## .contextignore

A `.contextignore` file in the project root excludes files from both context generation and indexing.

### Syntax

It uses `.gitignore` syntax:

```gitignore
# Comments start with #

# Ignore specific files
secret.key
config.local.json

# Ignore directories (trailing slash)
node_modules/
dist/
build/

# Glob patterns
*.test.ts
*.spec.js
**/__tests__/**

# Negation (include despite earlier rules)
!important.test.ts
```

### Pattern types

| Pattern | Matches |
|---------|---------|
| `*.ts` | All .ts files in any directory |
| `**/*.ts` | All .ts files recursively |
| `src/*.rs` | .rs files directly in src/ only |
| `src/**/*.rs` | .rs files in src/ and subdirectories |
| `tests/` | The tests directory |
| `**/tests/` | Any tests directory |
| `!important.ts` | Include important.ts (negation) |

### Common patterns by project type

#### JavaScript and TypeScript projects

```gitignore
# Dependencies (usually in .gitignore, but be explicit)
node_modules/

# Build outputs
dist/
build/
.next/
out/

# Test files
**/*.test.ts
**/*.spec.ts
**/__tests__/
**/__mocks__/
coverage/

# Generated files
*.generated.ts
*.d.ts
generated/

# Storybook
storybook-static/

# Config files (usually not useful for AI context)
*.config.js
*.config.ts
jest.config.js
tsconfig.json
```

#### Rust projects

```gitignore
# Build output
target/

# Generated files
*.generated.rs

# Benchmarks (usually not needed)
benches/

# Examples (optional)
examples/
```

#### Python projects

```gitignore
# Virtual environments
venv/
.venv/
env/

# Compiled files
__pycache__/
*.pyc

# Test outputs
.pytest_cache/
.coverage
htmlcov/

# Type stubs (optional)
*.pyi

# Jupyter
.ipynb_checkpoints/
```

#### Solidity projects

```gitignore
# Foundry
cache/
out/
broadcast/

# Vendored dependencies
lib/forge-std/
lib/openzeppelin-contracts/
lib/solmate/

# Generated
*.generated.sol
typechain-types/
```

#### Monorepos

```gitignore
# All node_modules
**/node_modules/

# All build outputs
**/dist/
**/build/
**/.next/

# Vendored libs in any package
**/lib/forge-std/
**/lib/openzeppelin-*/

# Package-specific ignores
packages/*/coverage/
apps/*/storybook-static/
```

## Built-in ignores

ctx excludes 164 common non-source patterns unless you pass `--no-default-ignores`. The categories
below show a selection.

### Categories

Version control:
```
.git/
.svn/
.hg/
.bzr/
.darcs/
.husky/
```

IDE and editor:
```
.vscode/
.idea/
.vs/
.sublime-project
.sublime-workspace
*.swp
*.swo
```

Lock files:
```
*.lock
*.lockb
package-lock.json
yarn.lock
pnpm-lock.yaml
Cargo.lock
poetry.lock
Gemfile.lock
Pipfile.lock
bun.lockb
```

Dependencies and build:
```
node_modules/
vendor/
Pods/
Carthage/
dist/
build/
out/
target/
.next/
.nuxt/
.gradle/
.m2/
.cargo/
coverage/
.nyc_output/
```

Cache and temp:
```
.cache/
cache/
tmp/
temp/
.tmp/
.temp/
*.cache
*.tsbuildinfo
.eslintcache
.parcel-cache/
.webpack/
.rollup.cache/
```

Environment and secrets:
```
.env
.env.*
.env.local
secrets/
private/
```

Binary files: images:
```
*.png
*.jpg
*.jpeg
*.gif
*.bmp
*.tiff
*.ico
*.webp
*.svg
```

Binary files: media:
```
*.mp3
*.mp4
*.avi
*.mov
*.wmv
*.webm
*.m4a
*.m4v
```

Binary files: system:
```
*.so
*.dll
*.dylib
*.lib
*.exe
*.bin
```

Archives:
```
*.tar
*.gz
*.bz2
*.tgz
*.zip
*.rar
*.7z
*.dmg
*.pkg
*.msi
*.deb
*.rpm
*.iso
*.img
```

Compiled files:
```
*.pyc
*.pyo
*.o
*.obj
*.class
*.jar
*.war
*.ear
```

Data files:
```
*.h5
*.hdf5
*.pkl
*.sqlite
*.sqlite3
*.db
*.joblib
*.mat
*.npz
*.npy
```

Font files:
```
*.woff
*.woff2
*.ttf
*.eot
*.otf
```

Security files:
```
*.crt
*.pem
*.key
```

System files:
```
.DS_Store
Thumbs.db
Desktop.ini
```

Backup and temp:
```
*.tmp
*.temp
*.bak
*.backup
*~
*.orig
*.rej
```

Documentation builds:
```
docs/_build/
site/
_site/
```

Package artifacts:
```
*.whl
*.egg-info/
*.egg
*.dist-info/
```

### The full list

See [src/default_ignores.rs](https://github.com/agentis-tools/ctx/blob/main/src/default_ignores.rs) for the complete list.

## Command-line options

### Ignore patterns

Add patterns for one run:

```bash
# Context generation
ctx -i "*.test.ts" -i "fixtures/" src/

# Indexing
ctx index -i "*.test.ts" -i "fixtures/"
```

`-i` can be repeated and adds to the other ignore sources.

### Include patterns (index only)

Limit indexing to matching files:

```bash
# Only index Rust files in src/
ctx index -p "src/**/*.rs"

# Index multiple patterns
ctx index -p "src/**/*.ts" -p "lib/**/*.ts"

# Mix globs and literal paths
ctx index -p "src/**/*.rs" -p "Cargo.toml"

# Positional paths scope the same way as -p
ctx index src
```

Include patterns accept:
- globs: `src/**/*.rs`, `*.ts`
- literal paths: `src/`, `lib/main.rs`
- absolute paths: `/home/user/project/src/**/*.rs`

If an explicit scope matches no files, `ctx index` exits non-zero and leaves the index unchanged.

### Disabling ignores

```bash
# Context generation
ctx --no-gitignore src/
ctx --no-default-ignores src/

# Indexing
ctx index --no-gitignore
ctx index --no-default-ignores

# Include everything (except .contextignore)
ctx --no-gitignore --no-default-ignores src/
ctx index --no-gitignore --no-default-ignores
```

ctx always applies `.contextignore` when it exists.

### Output format

```bash
ctx --format xml src/      # Default
ctx --format markdown src/ # Markdown
ctx --format md src/       # Alias for markdown
ctx --format plain src/    # Plain text
ctx --format json src/     # JSON
```

### Other options

```bash
ctx --show-sizes src/      # Show file sizes in tree
ctx --no-tree src/         # Omit project tree
ctx --no-stream src/       # Buffer output before printing
ctx --stats src/           # Print statistics
```

## Environment variables

| Variable | Purpose | Required |
|----------|---------|----------|
| `OPENAI_API_KEY` | OpenAI embeddings (`--provider openai`) | Only for the OpenAI provider |
| `OLLAMA_HOST` | Ollama server URL (default `http://localhost:11434`) | No |
| `OLLAMA_EMBED_MODEL` | Ollama embedding model (default `nomic-embed-text`) | No |
| `OLLAMA_API_KEY` | Bearer token for a remote Ollama host that requires one | No |
| `JEV_API_KEY` | API key for `ctx judge edges` and `[judge] edges = true`; never read from a config file | Only for model-assisted edge resolution |
| `CTX_GATE_BLOCKING` | `1` makes the generated Stop hooks block on failed gates | No |
| `CTX_GATE_LOG` | Appends one JSONL record per `ctx score` gate evaluation (`1` or `true` writes `.ctx/gate-log.jsonl`; another value is a path) | No |
| `CTX_SQL_MEMORY_LIMIT` | DuckDB memory limit for `ctx sql` (default `512MB`) | No |
| `CTX_NO_UPDATE_CHECK` | Any non-empty value turns off the passive update notice | No |
| `CTX_LSP_REGISTRY_BASE_URL` | Alternate base URL for the `ctx lsp` community registry (mirrors, testing) | No |

### Project config (`.ctx/config.toml`)

Per-project defaults live in an optional, committed `.ctx/config.toml`. It sets the embedding
backend used by `ctx embed`, `semantic`, `smart`, and `similar`, turns on model-assisted edge
resolution, and registers language servers for LSP-backed indexing:

```toml
[embedding]
provider = "ollama"                 # local (default) | openai | ollama
model = "qwen3-embedding:8b"        # Ollama/OpenAI model name
# host = "http://localhost:11434"   # Ollama only
```

Resolution order is CLI flag, then environment variable, then `.ctx/config.toml`, then the built-in
default, so the file never overrides an explicit flag. ctx ignores a malformed config file with a
warning. After changing the provider or model, rebuild the embeddings, because vectors from
different models are not interchangeable:

```bash
ctx embed --force
```

### Model-assisted edge resolution (`[judge]`)

```toml
[judge]
edges = true            # run `ctx judge edges` after every `ctx index` (default false)
min_confidence = 0.7    # answers below this leave the edge unchanged (default 0.7)
# model = "jev-latest"  # model id sent to the decision API (default jev-latest)
```

With `edges = true`, each `ctx index` asks the TypeSafe Jev model to resolve call edges whose name
matches more than one in-repo function, then rewrites the edge. Answers are cached in the index,
so a reindex of unchanged code makes no network calls. The API key comes only from `JEV_API_KEY`;
without it, `ctx index` applies cached answers and makes no calls. See
[`ctx judge`](commands/judge.md) and the [privacy policy](privacy.md) for what is sent.

### Language servers (`[lsp.<language>]`)

Each `[lsp.<language>]` table registers a stdio language server. `ctx index` uses it to extract or
refine code intelligence for the files it claims. The table key is the language name stored on
indexed files and symbols:

```toml
[lsp.kotlin]
command = "kotlin-language-server"
extensions = ["kt", "kts"]          # required: kotlin is not a built-in language
backend = "lsp"                     # tree-sitter | lsp | hybrid (default hybrid)

[lsp.python]
command = "pyright-langserver"
args = ["--stdio"]
# extensions default to the built-in set ("py", "pyi") for built-in language names
```

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `command` | string | none (required) | Server executable; resolved via `PATH` unless it contains a path separator |
| `args` | array of strings | `[]` | Arguments passed to the server |
| `extensions` | array of strings | built-in set for built-in language names | File extensions (without dots) this server claims; required for non-built-in language names |
| `root_markers` | array of strings | `[]` | Workspace-root marker files; informational, reported by `ctx lsp doctor` |
| `capabilities` | array of strings | `[]` | Capabilities the server is expected to advertise; checked by `ctx lsp doctor` |
| `backend` | string | `"hybrid"` | `"tree-sitter"`, `"lsp"`, or `"hybrid"`; see [Add a language via LSP](lsp-languages.md#backend-modes) |
| `initialization_options` | any value | unset | Passed verbatim as LSP `initializationOptions` |
| `env` | table of strings | `{}` | Extra environment variables for the server process |
| `timeout_ms` | integer | `10000` | Per-request timeout in milliseconds |
| `ready_timeout_ms` | integer | `300000` | How long cross-file resolution waits for the server to finish loading the workspace (`experimental/serverStatus` or the end of all `$/progress` reports); rust-analyzer answers `null` to definition requests until then |
| `source` | string | unset | Provenance written by `ctx lsp add` (`"registry"` marks the entry as registry-managed); accepted and ignored by the indexer |
| `source_server` | string | unset | Provenance written by `ctx lsp add`: the registry server name the entry was installed from |

Entries installed with [`ctx lsp add`](commands/lsp.md) carry the `source` and `source_server`
provenance keys, which `ctx lsp update` uses to refresh them. `ctx lsp update` never changes an
entry without `source = "registry"`.

`[lsp.*]` configuration never makes indexing fail:

- ctx skips an invalid block (empty `command`, or no `extensions` for a non-built-in language
  name) with a warning on stderr. The other blocks and the built-in grammars keep working.
- Unknown keys are allowed, so configs written by newer ctx versions still load.
- When two blocks claim the same extension, the first block in table-key order wins and a warning
  names both.
- If a configured server is missing or crashes, `ctx index` prints a warning, falls back to
  tree-sitter for built-in languages, and keeps its exit code.

Without any `[lsp.*]` block, ctx never starts a language server.

### Architecture policy (`.ctx/rules.toml`)

This committed file controls `ctx check` and the `check_violations` metric in `ctx score`. Generate
a commented starter, inspect the parsed policy, then evaluate it:

```bash
ctx harness init
ctx check --list
ctx check --against origin/main
```

`ctx harness init` never overwrites an existing `rules.toml`, even with `--force`. The
[`ctx check` rules-file reference](commands/check.md#rules-file) documents layers, forbidden
dependencies, allowed dependents, metric limits, and frozen paths.

### Embedding providers

`ctx embed`, `ctx semantic`, `ctx smart`, and `ctx similar` accept
`--provider <local|openai|ollama>` (or set a default in `.ctx/config.toml`):

- `local` (default): [fastembed](https://github.com/Anush008/fastembed-rs) `all-MiniLM-L6-v2`,
  384 dimensions. Runs offline after downloading a model of about 90 MB on first use.
- `openai`: `text-embedding-3-small`, 1536 dimensions. Requires `OPENAI_API_KEY`.
- `ollama`: any [Ollama](https://ollama.com) embedding model (`nomic-embed-text`,
  `mxbai-embed-large`, `qwen3-embedding:8b`, and others). Runs on your own Ollama server; ctx
  reads the dimension from the model.

```bash
# Ollama (start the daemon and pull a model first)
ollama pull nomic-embed-text
ctx embed --provider ollama
ctx smart --provider ollama "add a new output format"

# A different model / remote host
OLLAMA_EMBED_MODEL=qwen3-embedding:8b ctx embed --provider ollama
OLLAMA_HOST=http://gpu-box:11434 ctx embed --provider ollama
```

Vectors from different providers or models are not comparable, so switching requires
`ctx embed --provider <provider> --force`, which clears the stored corpus before rebuilding.
Semantic commands exit with code 2 when the query provider, model, or dimension does not match the
stored corpus, and `ctx smart` also exits 2 when embeddings are missing or incomplete.

`--openai` is still accepted as a deprecated alias for `--provider openai`.

### Setting OPENAI_API_KEY

For the current shell session:
```bash
export OPENAI_API_KEY=sk-...
ctx embed --provider openai
```

Permanently, in your shell profile:
```bash
# ~/.bashrc or ~/.zshrc
export OPENAI_API_KEY=sk-...
```

For one command:
```bash
OPENAI_API_KEY=sk-... ctx embed --provider openai
```

## Database location

The index lives at:

```
your-project/
└── .ctx/
    └── codebase.sqlite
```

This one file holds:
- symbol definitions
- call graph edges
- compressed source code
- the FTS5 search index
- embedding vectors, once generated
- cached `ctx judge` answers, if used

### Git

Most projects should ignore the index. Add to `.gitignore`:
```gitignore
.ctx/*
!.ctx/config.toml
!.ctx/rules.toml
```

`ctx index` rebuilds the database, it is a large binary file, and each developer can keep their
own. The project defaults and architecture policy stay shared.

Commit `.ctx/` instead if you want to share one index across the team or CI, or skip the rebuild on
new clones.

### Backing up

```bash
# Simple backup
cp -r .ctx/ .ctx-backup/

# With timestamp
cp -r .ctx/ ".ctx-backup-$(date +%Y%m%d)"
```

### Resetting

```bash
# Remove the database and reindex
# (this also deletes config.toml and rules.toml if you keep them in .ctx/)
rm -rf .ctx/
ctx index
```

`ctx index --force` clears only the database and rebuilds it:
```bash
ctx index --force
```

## Ignore precedence

ctx applies the ignore sources in this order:

1. built-in ignores (unless `--no-default-ignores`)
2. `.gitignore` (unless `--no-gitignore`)
3. `.contextignore` (always)
4. command-line `-i` patterns

A file is excluded if any of these sources excludes it.

## Tips

### Debugging ignore patterns

If a file is missing from the output:

1. Check whether a built-in pattern matches it:
   ```bash
   ctx --no-default-ignores src/
   ```

2. Check whether `.gitignore` excludes it:
   ```bash
   ctx --no-gitignore src/
   ```

3. Check your `.contextignore`:
   ```bash
   cat .contextignore
   ```

### Performance

Fewer files means faster indexing and context generation:

```gitignore
# Large vendored dependencies
lib/forge-std/
lib/openzeppelin-*/

# Generated files
*.generated.ts
typechain-types/

# Test fixtures
fixtures/
__mocks__/
```

### Team templates

Share one `.contextignore` across your projects:

```bash
# Copy it from an existing project
cp path/to/good/.contextignore .contextignore
```
