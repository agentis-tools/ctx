---
id: configuration
title: Configuration
sidebar_position: 5
---

# Configuration

ctx uses a minimal configuration approach based on ignore files. There are no complex config files to manage - just familiar gitignore-style patterns.

## Overview

ctx configuration consists of:
1. **`.contextignore`** - Project-specific ignore patterns
2. **`.gitignore`** - Standard git ignores (respected by default)
3. **Built-in ignores** - 170+ common patterns (enabled by default)
4. **Command-line flags** - Runtime overrides
5. **Environment variables** - API keys for embeddings

## .contextignore

Create a `.contextignore` file in your project root to exclude files from both context generation and code intelligence indexing.

### Syntax

Uses the same syntax as `.gitignore`:

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

### Pattern Types

| Pattern | Matches |
|---------|---------|
| `*.ts` | All .ts files in any directory |
| `**/*.ts` | All .ts files recursively |
| `src/*.rs` | .rs files directly in src/ only |
| `src/**/*.rs` | .rs files in src/ and subdirectories |
| `tests/` | The tests directory |
| `**/tests/` | Any tests directory |
| `!important.ts` | Include important.ts (negation) |

### Common Patterns by Project Type

#### JavaScript/TypeScript Projects

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

#### Rust Projects

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

#### Python Projects

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

#### Solidity Projects

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

#### Monorepo Projects

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

## Built-in Ignores

ctx automatically excludes 170+ common non-source patterns. These are always applied unless you use `--no-default-ignores`.

### Categories

**Version Control:**
```
.git/
.svn/
.hg/
.bzr/
.darcs/
.husky/
```

**IDE & Editor:**
```
.vscode/
.idea/
.vs/
.sublime-project
.sublime-workspace
*.swp
*.swo
```

**Lock Files:**
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

**Dependencies & Build:**
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

**Cache & Temp:**
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

**Environment & Secrets:**
```
.env
.env.*
.env.local
secrets/
private/
```

**Binary Files - Images:**
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

**Binary Files - Media:**
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

**Binary Files - System:**
```
*.so
*.dll
*.dylib
*.lib
*.exe
*.bin
```

**Archives:**
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

**Compiled Files:**
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

**Data Files:**
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

**Font Files:**
```
*.woff
*.woff2
*.ttf
*.eot
*.otf
```

**Security Files:**
```
*.crt
*.pem
*.key
```

**System Files:**
```
.DS_Store
Thumbs.db
Desktop.ini
```

**Backup & Temp:**
```
*.tmp
*.temp
*.bak
*.backup
*~
*.orig
*.rej
```

**Documentation Builds:**
```
docs/_build/
site/
_site/
```

**Package Artifacts:**
```
*.whl
*.egg-info/
*.egg
*.dist-info/
```

### Viewing the Full List

See [src/default_ignores.rs](https://github.com/agentis-tools/ctx/blob/main/src/default_ignores.rs) for the complete list.

## Command-Line Configuration

### Ignore Patterns

Add patterns on the fly:

```bash
# Context generation
ctx -i "*.test.ts" -i "fixtures/" src/

# Indexing
ctx index -i "*.test.ts" -i "fixtures/"
```

Multiple `-i` flags are combined with other ignore sources.

### Include Patterns (Index Only)

Limit indexing to specific patterns:

```bash
# Only index Rust files in src/
ctx index -p "src/**/*.rs"

# Index multiple patterns
ctx index -p "src/**/*.ts" -p "lib/**/*.ts"

# Mix globs and literal paths
ctx index -p "src/**/*.rs" -p "Cargo.toml"
```

Include patterns support:
- Glob patterns: `src/**/*.rs`, `*.ts`
- Literal paths: `src/`, `lib/main.rs`
- Absolute paths: `/home/user/project/src/**/*.rs`

### Disabling Ignores

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

Note: `.contextignore` is always respected if present.

### Output Format

```bash
ctx --format xml src/      # Default
ctx --format markdown src/ # Markdown
ctx --format md src/       # Alias for markdown
ctx --format plain src/    # Plain text
```

### Other Options

```bash
ctx --show-sizes src/      # Show file sizes in tree
ctx --no-tree src/         # Omit project tree
ctx --no-stream src/       # Buffer output before printing
ctx --stats src/           # Print statistics
```

## Environment Variables

ctx uses minimal environment variables:

| Variable | Purpose | Required |
|----------|---------|----------|
| `OPENAI_API_KEY` | OpenAI embeddings via `ctx embed --openai` | Only for OpenAI provider |

### Setting OPENAI_API_KEY

**Temporary (current session):**
```bash
export OPENAI_API_KEY=sk-...
ctx embed --openai
```

**Permanent (add to shell profile):**
```bash
# ~/.bashrc or ~/.zshrc
export OPENAI_API_KEY=sk-...
```

**Per-command:**
```bash
OPENAI_API_KEY=sk-... ctx embed --openai
```

## Database Location

The code intelligence database is stored at:

```
your-project/
└── .ctx/
    └── codebase.sqlite
```

This single file contains:
- Symbol definitions
- Call graph edges
- Compressed source code
- FTS5 search index
- Embedding vectors (if generated)

### Git Recommendations

**Ignore (Recommended for most projects):**

Add to `.gitignore`:
```gitignore
.ctx/
```

This is recommended because:
- Database can be rebuilt with `ctx index`
- Avoids large binary files in git
- Each developer can have their own index
- Embedding dimensions may differ between local/OpenAI

**Commit (For shared code intelligence):**

Don't ignore `.ctx/` if you want to:
- Share the index across the team
- Include code intelligence in CI/CD
- Avoid rebuild time for new clones

### Backing Up

```bash
# Simple backup
cp -r .ctx/ .ctx-backup/

# With timestamp
cp -r .ctx/ ".ctx-backup-$(date +%Y%m%d)"
```

### Resetting

```bash
# Remove database and re-index
rm -rf .ctx/
ctx index
```

Or use the force flag:
```bash
ctx index --force
```

## Configuration Precedence

When determining whether to include a file:

1. **Built-in ignores** - Applied first (unless `--no-default-ignores`)
2. **`.gitignore`** - Applied second (unless `--no-gitignore`)
3. **`.contextignore`** - Applied third (always respected)
4. **Command-line `-i` patterns** - Applied last

A file is excluded if **any** of these rules match.

## Tips

### Debugging Ignore Patterns

If a file isn't appearing:

1. Check if it matches built-in patterns:
   ```bash
   ctx --no-default-ignores src/
   ```

2. Check if it's in `.gitignore`:
   ```bash
   ctx --no-gitignore src/
   ```

3. Check your `.contextignore`:
   ```bash
   cat .contextignore
   ```

### Performance

Excluding more files = faster indexing/context generation:

```gitignore
# Good: exclude large vendored dependencies
lib/forge-std/
lib/openzeppelin-*/

# Good: exclude generated files
*.generated.ts
typechain-types/

# Good: exclude test fixtures
fixtures/
__mocks__/
```

### Project Templates

Create a standard `.contextignore` for your team:

```bash
# Copy from an existing project
cp path/to/good/.contextignore .contextignore
```
