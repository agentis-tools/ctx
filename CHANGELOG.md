# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Global `--json` flag: `search`, `semantic`, `query find/callers/deps/graph/impact/stats/files`, and `explain` emit a single machine-readable JSON document wrapped in a stable envelope (`ctx_version`, `command`, `generated_at`, `data`); see `docs/json-output.md`
- Index schema versioning via SQLite `PRAGMA user_version`; opening an index built with an incompatible schema now fails with a clear "run `ctx index --force`" message (pre-existing indexes are stamped silently)
- Shared complexity metrics (fan-in / fan-out / complexity) available directly from the SQLite index, mirroring the DuckDB formula
- `ctx index --force` now also removes stale SQLite `-wal`/`-shm` sidecar files
- Library API documentation: crate-level rustdoc with integration examples, module docs, and a "Using ctx as a Library" README section
- docs.rs builds with all features enabled

### Changed
- **Breaking:** exit codes now follow a three-way convention: 0 = clean, 1 = findings, 2 = operational error (errors previously exited with code 1)
- **Breaking:** `search --output json` and `semantic --output json` now emit the new envelope instead of the old ad-hoc JSON arrays; `query graph --output json` is an alias for `--json` (`complexity`/`graph`/`audit` keep their legacy shapes for now)

### Fixed
- `mcp` feature failed to compile the binary (`use crate::mcp` resolved against the binary crate instead of the library); CI now builds `--all-features` on Linux to prevent regressions

## [0.2.1] - 2026-06-17

### Added
- Token count estimate in `--stats` output: `Generated context: N files, X KB, ~Yk tokens in Zms`
- Token count shown automatically by `ctx smart` and `ctx diff`/`ctx review` after context generation

### Changed
- Published on crates.io as `agentis-ctx` (the `ctx` name is taken); the installed binary is still `ctx`

## [0.2.0] - 2026-06-06

### Added
- **Code Intelligence Foundation** -- SQLite-based symbol database with FTS5 full-text search
- **Multi-language Parsing** -- Rust, TypeScript, JavaScript, Python, Go, Solidity, YAML via Tree-sitter
- **Symbol Extraction** -- Functions, structs, enums, traits, classes, interfaces, contracts, and more
- **Relationship Tracking** -- Call graphs, inheritance, implementations, and import edges
- **Semantic Search** -- Embedding-based search via local fastembed (`all-MiniLM-L6-v2`) or OpenAI (`text-embedding-3-small`)
- **Vector Search** -- Fast similarity search powered by sqlite-vec
- **Call Graph Analysis** -- Query callers, callees, and visualize dependency graphs
- **Impact Analysis** -- See what would be affected by changing a symbol
- **Smart Context Selection** -- AI-powered file relevance scoring based on task descriptions
- **Diff-Aware Context** -- Generate context focused on git changes with dependency expansion
- **PR Review Context** -- GitHub CLI integration for pull request analysis
- **Code Quality Audit** -- Automated complexity, duplication, coverage, and modularity analysis
- **Duplicate Detection** -- Find similar or identical code blocks across the codebase
- **Complexity Analysis** -- Fan-out/fan-in analysis with configurable thresholds
- **Interactive Shell** -- REPL powered by rustyline for live codebase exploration
- **MCP Server Mode** -- Model Context Protocol integration for Claude Desktop
- **Parallel Indexing** -- Multi-core build support via rayon (~1.7x speedup)
- **File Watching** -- Automatic reindexing on file changes (notify)
- **Source Compression** -- Gzip-compressed source storage in SQLite (~70% reduction)
- **Incremental Updates** -- Only reindex changed files on subsequent runs
- **ASCII Project Tree** -- Visual file structure in context output
- **Streaming Output** -- Real-time context generation, pipeable to clipboard
- **Token Counting** -- tiktoken-rs integration for LLM context window management
- **Multiple Output Formats** -- XML (default), Markdown, JSON, and plain text
- **Built-in Ignore System** -- 170+ patterns plus `.gitignore` and `.contextignore` support
- **Pre-commit Hooks** -- Incremental audit integration for quality gates
- **Comprehensive Documentation** -- Per-command docs and integration guides

### Changed
- Restructured project from single-file CLI to modular library + binary architecture
- README rewritten with full feature overview and quick-start guide

## [0.1.0] - 2025-01-25

### Added
- Initial release
- XML, Markdown, and plain text output formats
- Glob pattern support for file selection
- `.gitignore` integration (enabled by default)
- `.contextignore` support for project-specific ignores
- Built-in ignore patterns for 170+ common non-source files
- ASCII project tree visualization
- File size display option (`--show-sizes`)
- Binary file detection and exclusion

[Unreleased]: https://github.com/saldestechnology/ctx/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/saldestechnology/ctx/releases/tag/v0.2.1
