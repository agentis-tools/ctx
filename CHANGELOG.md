# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-06-17

### Added
- Token count estimate in `--stats` output: `Generated context: N files, X KB, ~Yk tokens in Zms`
- Token count shown automatically by `ctx smart` and `ctx diff`/`ctx review` after context generation

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
