# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Documentation
- Added symptom-first cookbook routing, per-recipe quick paths, canonical cross-cutting concepts,
  pinned worked-example provenance, and an authoring contract that preserves verified limitations.
- Added verified recipes for fork-safe ctx analysis and recovery from disputed findings,
  operational gate failures, and stale indexes without weakening policy.
- Expanded the downloadable ctx skill and `llms.txt` so agents can route directly from engineering
  symptoms to the complete cookbook workflow.
- Started Cookbook v2 with a source-verified unfamiliar-codebase orientation workflow that distinguishes ranked structural leads from real entry points and documents static-analysis uncertainty.
- Added a verified smallest-useful-context recipe that tests smart selection at multiple budgets, audits omitted consumers and contracts, and separates compact symbol investigation from complete implementation context.
- Added a reuse-discovery recipe that compares semantic, signature-like, and keyword retrieval, traces matching primitives into their composed workflow, and verifies ownership semantics from tests before recommending reuse.
- Added a blast-radius recipe that bounds graph traversal, verifies transitive relationships from source, supplements incomplete edges with exact contract search, and turns persisted, generated, public, and behavioral impact into a validation plan.
- Added an evidence-backed implementation recipe that baselines the owning boundary, tests the compiled interface, refreshes contracts and code intelligence after editing, audits context omissions, and interprets structural deltas before completion.
- Added a focused failing-test debugging recipe that reproduces the symptom, traces test dependencies into production code, compares neighboring behavior, and proves one causal correction with widening validation.
- Added a large-branch review recipe that inventories the complete change, separates review streams, routes attention with scoped metrics, rejects false graph expansion, and verifies policy intent against enforcement and CI wiring.
- Added an outcome-driven cookbook with a real ctx-on-ctx case study for capturing codebase-health snapshots in CI, normalizing longitudinal metrics, and investigating trends without treating high complexity or duplication as automatic defects.
- Added a delta-focused pull-request governance recipe that separates informational metrics, human-review signals, explicit blocking policy, and safe analysis of fork contributions.
- Added an architectural-drift recipe that combines reviewed dependency rules, pull-request scoping, longitudinal coupling signals, and policy-history interpretation.
- Added a chronic-hotspots recipe that combines current churn-complexity rankings, normalized historical evidence, change history, and ownership-focused investigation.
- Added an intentional-complexity recipe that separates fan-in from fan-out, tests responsibility coherence, and records why complex shared primitives, parsers, and dispatchers may remain intact.
- Added a duplication-trajectory recipe that distinguishes current, changed-file, and newly introduced pairs while normalizing history and requiring ownership analysis before reuse.
- Completed the first cookbook set with a release-health reporting workflow that combines immutable comparisons, provenance, normalized metrics, focused investigations, uncertainty, and owned actions.

### Internal
- Made the local CI and canonical plugin lockstep checks honor Cargo's configured target directory while validating the standalone downloadable ctx skill against its harness template.

## [0.3.5] - 2026-07-13

### Added
- Homebrew, Scoop, and AUR package definitions, plus Debian and RPM artifacts in the release workflow with native-package validation and upgrade guidance (#38)
- Canonical Claude Code and Codex plugin trees that are regenerated and checked against the harness templates, then shipped as versioned release ZIPs (#40)
- Pull-request analysis workflows that install a checksum-verified ctx binary, run repository checks in an unprivileged workflow, and publish a single updatable report comment from the trusted default branch (#39)

### Changed
- Standardized release archive construction, checksums, metadata generation, and package-manager-safe self-update behavior so package-owned installations defer upgrades to their package manager (#38)
- Updated GitHub Actions JavaScript runtimes to Node 24 (#39)

### Fixed
- Removed unusable `-f` aliases from `--file` filters that collided with the global `-f`/`--format` option and caused affected command help to abort (#41)

### Security
- Refreshed compatible locked dependencies to remediate actionable RustSec advisories and documented narrowly scoped, time-bounded transitive exceptions (#41)

### Documentation
- Expanded installation, package upgrade, project configuration, and canonical plugin setup guidance (#38, #40)

### Internal
- Added deterministic versioning, release governance, compatibility checks, supply-chain policy, and artifact provenance guardrails (#41)

## [0.3.4] - 2026-07-12

### Added
- Codex harness support: `ctx harness init --target codex` installs trusted project hooks under `.codex/`, prints durable `AGENTS.md` guidance, and supports the same session map, post-edit architecture checks, and opt-in blocking stop-time quality gate as the Claude integration (#36)
- Distributable Codex plugin scaffolding via `ctx harness init --target codex --mode plugin`, including lifecycle hooks, the ctx skill, local marketplace metadata, optional MCP wiring, release packaging, JSON output, doctor diagnostics, and integration documentation (#36)

### Fixed
- Removed a generated ctx index accidentally committed beneath the documentation site and ignored nested `.ctx` directories (#34)

### Documentation
- Refreshed the whole-repository token estimate after the recent feature and documentation growth (#35)

## [0.3.3] - 2026-07-11

### Added
- Ollama embedding provider: `--provider ollama` on `ctx embed`/`semantic`/`smart`/`similar` (and the MCP `smart_context` tool) talks to a local or remote Ollama server via `/api/embed` -- host from `OLLAMA_HOST` (default `http://localhost:11434`, bare `host:port` normalized), model from `OLLAMA_EMBED_MODEL` (default `nomic-embed-text`), optional `OLLAMA_API_KEY` bearer for remote hosts -- with the embedding dimension probed from the model rather than hardcoded. Provider selection is unified behind a single `--provider <local|openai|ollama>` flag and factory (the old `--openai` stays as a deprecated alias), and a provider/dimension mismatch against the existing index is detected and warned. Fully offline and free (#28)
- Project configuration `.ctx/config.toml`: an optional, committed file that sets per-project defaults under `[embedding]` (`provider`, `model`, `host`); resolution is always CLI flag > environment variable > this file > built-in default, so it never overrides an explicit request. `.ctx/` stays git-ignored while `config.toml` is kept tracked so the default is shared with the team (#28)

### Changed
- `ctx index` and `ctx embed` now run in parallel by default; pass `--serial` for single-threaded execution. `ctx embed` parallelizes embedding computation across rayon threads (chunked provider calls, preserving order and the provider's retry/backoff) while storing serially. The `-j`/`--parallel` flag on `ctx index` is now a no-op (kept for backward compatibility) (#31)
- `ctx smart` file ranking: a lexical path boost promotes candidates whose path contains the task's own tokens (surfacing on-topic files the embedding ranked low, e.g. `embeddings/openai.rs` for "...openai"), and the single most-relevant file is always included even when it alone exceeds the `--max-tokens` budget rather than being dropped in favour of smaller, less-relevant files. Selection stays deterministic (#25)

### Fixed
- Solidity modifier invocations (`function f() onlyOwner`) now emit `calls` edges, so `ctx query callers`/`impact` and `v1.edges` can answer access-control questions; also covers constructor base-contract invocations (#29)
- Solidity qualified library calls `Lib.fn()` now resolve to the library function instead of remaining unresolved in the call graph (#32)
- `ctx duplicates` no longer silently skips Solidity: functions are tokenized via the solang-parser lexer (identifiers -> ID, string/hex/address/number literals -> LIT, keywords/punctuation verbatim) and participate in MinHash near-duplicate detection, matching the normalization used for tree-sitter languages (#30)
- MCP `smart_context` failed to compile under `--all-features` because a `.ctx/config.toml` variable shadowed the `SmartConfig`; renamed so the `mcp`/`--all-features` build works (#25)

## [0.3.2] - 2026-07-11

### Added
- `ctx snapshot`: per-commit Parquet metric snapshots for longitudinal quality analysis -- exports HEAD's per-symbol metrics, per-file metrics (complexity, churn within `--churn-window`, default "90 days ago", and rule violations), near-duplicate pairs, and capture metadata as one partition `.ctx/snapshots/sha=<sha>/{symbols,files,dup_pairs,meta}.parquet`, every row denormalized with `commit_sha`/`committed_at` so partitions union with a single `read_parquet` glob; partitions are staged and atomically renamed into place, existing partitions are skipped unless `--force`, a dirty working tree warns on stderr, and `--json` emits a `snapshot.capture` envelope. `ctx snapshot backfill --since <REF> [--every N]` walks the first-parent range `REF..HEAD` (REF inclusive) oldest-first through temporary git worktrees to build the history -- newest commit always sampled, per-commit failures logged to stderr and skipped, `--json` emits `snapshot.backfill`. Requires the `duckdb` feature (exit 2 otherwise); see `docs/commands/snapshot.md`
- `ctx sql --snapshots[=DIR]` (default `.ctx/snapshots`): materializes the snapshot partitions as `snap.files`, `snap.symbols`, `snap.dup_pairs`, and `snap.meta` tables (loaded before the sandbox hardening, so the query connection stays read-only) for trend queries across commits; the `snap.*` column reference and canned duplication/violation/hotspot-mass trend queries are in the SQL schema reference (`ctx sql --schema`)
- Gate-evaluation logging via `CTX_GATE_LOG`: when set, `ctx score` appends one JSONL record per gate evaluation to a local log (`1`/`true` = `.ctx/gate-log.jsonl` under the repo root, any other value = custom path, unset/empty/`0` = off) with `schema_version`, `ts`, `ctx_version`, `source`, `against`, `fail_on`, the seven scorecard `metrics`, `failed_conditions`, `outcome` (`pass`/`fail`), `blocking`, and `session_id`; best-effort (IO failures warn on stderr) and never changes the command's exit code. Opt-in and local-only -- ctx ships no telemetry
- Opt-in blocking quality gate in the Claude Code Stop hook: `CTX_GATE_BLOCKING=1` makes the generated `stop.sh` exit 2 (Claude Code's blocking-stop mechanism, so the session keeps working until the failed conditions are addressed) when `ctx score --fail-on` gates fail; the default stays non-blocking, and operational errors (compat mismatch, score errors) always fail open with a stderr note
- Study scripts for longitudinal gate experiments: `scripts/rework-rate.sh` (fraction of each commit's added lines modified or deleted again within a window, via `git blame` survival at the window boundary) and `scripts/revert-rate.sh` (revert and fix-commit rates per 100 first-parent commits), plus a versioned benchmark run-record schema (`docs/benchmark/run-record.schema.json`, JSON Schema draft 2020-12, documented in `docs/benchmark/run-record.md`) for recording agent benchmark runs as JSONL
- Operational performance harness (`perf/`, a non-published companion crate): spawns a prebuilt `ctx` binary against deterministic synthetic fixtures and enforces latency budgets on the hook-path commands (incremental index 300 ms, score 2 s, check 1 s, map 500 ms, sql 500 ms on a 2,000-file fixture; cold index of a ~150k-LOC fixture 60 s; 300 MB RSS ceiling) plus a 1.20x regression gate against committed per-runner-class baselines (`perf/baselines/`; `CTX_PERF_BUDGET_SCALE` relaxes budgets to 1.5x in CI), with criterion microbenches for library hot paths; wired into CI as an advisory `perf` job and a new `[profile.perf]` build profile
- Snapshot CI workflow (`.github/workflows/snapshot.yml`): every push to `main` captures a `ctx snapshot` partition and appends it to the `ctx-snapshots` orphan data branch (serialized via a concurrency group, idempotent per sha), keeping the metric history out of the main branch while staying queryable with `ctx sql --snapshots`
- Deterministic seeded fixture generator (`ctx::fixture`, `#[doc(hidden)]`): fully parameterized synthetic repos (`FixtureSpec { seed, files, avg_loc, modules, fan_in_skew, history_commits }`) shared by the perf harness and reusable by external benchmark suites; any change to the generated bytes bumps `FIXTURE_FORMAT_VERSION`, invalidating committed perf baselines automatically

- Run-record schema v1: additive optional fields for the ctx-bench pilot runner -- `agent` (session id, cost, tokens, turns), `acceptance` (the ctx-independent functional endpoint), `normalization` (changed-line denominators), plus `study_id`, `task_seed`, `generator_version`, `scorer_ctx_version`, `retry_attempt`, and `metrics.gate_blocks`/`gate_block_recovered`
- First committed `perf/baselines/ubuntu-latest.json` (captured from CI run 29126075011); the perf job's regression gate is now active against it

### Changed
- The `stop.sh` harness template was revised for `CTX_GATE_BLOCKING` and the gate log; re-run `ctx harness init` after updating ctx to regenerate the hook scripts (`ctx harness doctor`'s `templates_stale` check reports when this is due)

### Fixed
- Exact-name symbol lookup silently failed on snake_case names (underscores were treated as `LIKE` wildcards); `ctx query find`/`explain` now escape the pattern
- `ctx smart` is deterministic and semantic-first: stable ordering for equal-relevance results and semantic ranking applied before structural fallbacks

## [0.3.1] - 2026-07-10

### Changed
- `ctx harness init` (local mode) now **automatically wires the hooks into `.claude/settings.json`** instead of only printing a snippet for you to paste. The merge is additive and idempotent: a missing settings file is created; an existing one is deep-merged (`permissions.allow`/`deny` are unioned with de-duplication, and a ctx hook group is appended to each `SessionStart`/`PostToolUse`/`Stop` event only when none already references `.claude/hooks/ctx/`), leaving unrelated keys untouched and re-running `init` a no-op. A settings file that is present but not valid JSON is never modified -- ctx falls back to printing the snippet with a warning. `--json` output gains a `settings_action` field (`created`/`merged`/`already_wired`/`skipped_invalid`)

### Internal
- Parser refactors with no behavior change: extracted a `push_symbol` helper in the Solidity parser and symbol-construction/import helpers in the Go and TypeScript parsers, reducing per-function fan-out complexity in the hottest parser functions

## [0.3.0] - 2026-07-10

### Added
- `ctx self-update [--version X.Y.Z]`: update the binary from GitHub releases -- picks the artifact for the current platform (mirroring the release build matrix), verifies its sha256 against the release's aggregated `SHA256SUMS` file (mismatch aborts with exit 2, binary untouched), and atomically replaces the running executable (on Windows the previous binary is renamed aside as `ctx.exe.old` and cleaned up on the next run); `--version` pins an exact release (downgrades allowed), unwritable install locations are refused with guidance before any network work, and `--json` emits a `self_update` envelope (`old_version`, `new_version`, `outcome`)
- Passive update notice: interactive invocations check GitHub for a newer release at most once per 24h (timestamp cache under the user cache dir, 1-second timeout, silent failure) and print a single stderr line pointing at `ctx self-update`. **ctx never updates itself automatically** -- the check only prints a notice. It is skipped entirely (no network call) with `CTX_NO_UPDATE_CHECK=1`, when stderr is not a terminal, in `--json` mode, and inside Claude Code hooks/sessions (`CLAUDECODE`, `CLAUDE_PROJECT_DIR`, `CLAUDE_PLUGIN_ROOT`)
- `ctx --version --check`: explicit release comparison on stdout (always allowed -- exempt from the suppression rules and the 24h cache; exits 0 whether or not an update exists; `--json` emits a `version.check` envelope). Plain `ctx --version` / `-V` output is byte-identical to before
- Release workflow: releases now attach an aggregated `SHA256SUMS` file over all artifacts (consumed by `ctx self-update`; per-artifact `.sha256` files remain) and a `ctx-claude-plugin-<version>.zip` plugin scaffold; a new lockstep gate (`scripts/release-plugin-check.sh`) fails the release when the generated `plugin.json` version does not match the tag. The plugin updates through Claude Code via `/plugin update ctx`; sha-pin the marketplace entry for frozen installs
- `ctx harness init|compat|doctor`: package ctx as a Claude Code integration -- `init` scaffolds version-guarded hook scripts (`SessionStart` codebase map, `PostToolUse` architecture check, `Stop` quality scorecard), a commented starter `.ctx/rules.toml`, and prints the `.claude/settings.json` snippet (allow `Bash(ctx *)`, deny self-update and policy-file edits) plus a `CLAUDE.md` guidance block; `--mode plugin` generates a full Claude Code plugin (`.claude-plugin/plugin.json` with the crate version, `hooks/`, `skills/ctx/SKILL.md`, `marketplace.json`, permissions `settings.json`, and `.mcp.json` when the binary has the `mcp` feature). All files come from templates embedded in the binary and carry a `generated by ctx` header plus checksum (JSON files are tracked in the new `.ctx/harness.lock` manifest); re-running `init` regenerates unmodified files in place but never overwrites user-modified or foreign files without `--force`, and `.ctx/rules.toml` is never overwritten at all. `compat --require <SEMVER>` is the fail-open version guard baked into every generated hook (one stderr line + exit 3 on mismatch); `doctor` diagnoses the integration (binary/template versions, index existence/schema/freshness, rules validity, hook wiring and checksums, MCP availability) and exits 1 on problems. `init` and `doctor` support the global `--json` envelope (`harness.init`, `harness.doctor`; see `docs/json-output.md`)
- `ctx score`: quality scorecard for the changes between a git reference and the working tree -- reports `complexity_delta` and `fan_out_delta` (baseline parsed in memory at the reference with the same parser; fan-in approximated as same-file callers on both sides), `new_duplication` (near-duplicate pairs at Jaccard >= 0.85 / >= 50 tokens with at least one endpoint in a changed file that did not exist at the baseline), `check_violations` (via the `ctx check` engine, scoped to the same reference; 0 with a note when `.ctx/rules.toml` is missing), `symbols_added` / `symbols_removed`, and `files_changed`; `--against <REF>` (default `HEAD` scores uncommitted changes, use `main`/`master` to score a branch or PR), `--fail-on "metric OP value,..."` CI gates (`>=`, `<=`, `>`, `<`) that exit 1 when any condition is met, and the global `--json` envelope with flat `metrics` plus a `per_file` breakdown; refreshes the index incrementally before scoring
- `ctx map`: token-budgeted repository map for priming AI assistants (e.g. from SessionStart hooks). Ranks symbols with PageRank over the resolved symbol graph (calls, imports, extends, implements), spends ~10% of the budget on a compact project tree, and emits symbols grouped by file until the budget (tokens estimated as `ceil(chars / 4)`) is exhausted. Supports `--budget`, `--focus <path-glob|symbol>` (10x rank boost for the focused symbols and their direct neighbors), and `--format text|markdown|json` (the global `--json` flag forces JSON); output is byte-identical for identical index state. Ranks are cached in a new `symbol_rank` table that is invalidated on reindex and recomputed lazily, so existing indexes self-heal without a rebuild
- `ctx similar <description>`: find existing functions/methods similar to a natural-language description before writing a new one; reports similarity score, fan-in, and a one-line doc per hit, supports `--keyword` (FTS5 fallback that needs no embeddings), `--openai`, and `--json`, and exits with code 2 when embeddings are missing
- `ctx hotspots`: rank files (or symbols with `--by symbol`) by combined git churn and code complexity (`score = normalized_churn * normalized_complexity`), with `--since`, `--limit`, `--min-churn`, and `--against REF` filters and `--json` output including each file's top 3 most complex symbols; see `docs/json-output.md`
- `ctx check`: architecture rules engine driven by `.ctx/rules.toml` -- declare layers as glob patterns over indexed files, then enforce `forbidden` layer dependencies, `allowed_dependents` whitelists, `limit` metric thresholds (fan-in / fan-out / complexity / file symbols), and `no_new_dependents` frozen paths; supports `--against REF` to scope violations to changed files, `--list` to inspect parsed rules, and `--json`; exits 1 when violations are found (see `ctx check --help` for a full example)
- Global `--json` flag: `search`, `semantic`, `query find/callers/deps/graph/impact/stats/files`, and `explain` emit a single machine-readable JSON document wrapped in a stable envelope (`ctx_version`, `command`, `generated_at`, `data`); see `docs/json-output.md`
- Index schema versioning via SQLite `PRAGMA user_version`; opening an index built with an incompatible schema now fails with a clear "run `ctx index --force`" message
- Shared complexity metrics (fan-in / fan-out / complexity) available directly from the SQLite index, mirroring the DuckDB formula
- `ctx index --force` now also removes stale SQLite `-wal`/`-shm` sidecar files
- MinHash structural fingerprints: `ctx index` now fingerprints every function/method (normalized token shingles: identifiers -> `ID`, string/number literals -> `LIT`, comments dropped) into the new `symbol_fingerprints` table, incrementally per changed file
- `ctx duplicates --against <REF>` limits results to pairs where at least one function is in a file changed relative to a git reference
- `ctx duplicates --fail-on-found` exits with code 1 when any near-duplicate pair is reported (default remains informational, exit 0)
- `ctx duplicates` supports the global `--json` envelope (`data.pairs` with SymbolRefs, similarity, token counts, plus `skipped_languages`)
- Library API documentation: crate-level rustdoc with integration examples, module docs, and a "Using ctx as a Library" README section
- docs.rs builds with all features enabled

### Changed
- Repository moved to [agentis-tools/ctx](https://github.com/agentis-tools/ctx); documentation now lives at [docs.agentis.tools](https://docs.agentis.tools) (the old saldestechnology.github.io Pages URL no longer works, GitHub repo URLs redirect)
- Exit code 3 is now reserved exclusively for `ctx harness compat` (version requirement not met); all other commands keep the 0 = clean / 1 = findings / 2 = error convention
- **Breaking:** exit codes now follow a three-way convention: 0 = clean, 1 = findings, 2 = operational error (errors previously exited with code 1)
- **Breaking:** `search --output json` and `semantic --output json` now emit the new envelope instead of the old ad-hoc JSON arrays; `query graph --output json` is an alias for `--json` (`complexity`/`graph`/`audit` keep their legacy shapes for now)
- **Breaking:** `ctx duplicates` is now a MinHash-based structural near-duplicate detector. The old line-based `--similarity <PERCENT>`, `--min-lines <N>`, and `--output` flags are removed. The new `--threshold <F>` (default 0.85) is a Jaccard similarity from 0.0 to 1.0 over normalized 5-token shingles -- 0.85 means 85% of shingles are shared, not that 85% of lines match -- and `--min-tokens <N>` (default 50) filters short functions. Renamed variables and changed literals no longer hide duplicates; idiomatic boilerplate may appear (raise `--min-tokens` to filter it). Solidity functions are skipped (no tree-sitter grammar). Existing indexes lack fingerprints and must be rebuilt: run `ctx index --force` (index schema is now v2)

### Fixed
- `mcp` feature failed to compile the binary (`use crate::mcp` resolved against the binary crate instead of the library); CI now builds `--all-features` on Linux to prevent regressions
- Stack overflow in the compiled binary on Windows (`~1 MiB` default thread stack) under normal parsing/graph-walking call depth; `main()` and rayon's global pool now run with an explicit 16 MiB stack
- CI's `test` job matrix silently collapsed to a single `windows-latest --no-default-features` job instead of the intended 3 (ubuntu `--all-features`, macos default, windows `--no-default-features`), because the redundant, unused `rust: [stable]` matrix axis caused later `include` entries to overwrite earlier ones; `ubuntu-latest`/`macos-latest` had never actually run in CI
- Added `.gitattributes` (`* text=auto eol=lf`); without it, Windows checkouts (git `core.autocrlf=true` by default) rewrote `include_str!`'d harness templates from LF to CRLF, breaking `ctx harness init`'s generated file content on Windows

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

[Unreleased]: https://github.com/agentis-tools/ctx/compare/v0.3.5...HEAD
[0.3.5]: https://github.com/agentis-tools/ctx/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/agentis-tools/ctx/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/agentis-tools/ctx/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/agentis-tools/ctx/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/agentis-tools/ctx/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/agentis-tools/ctx/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/agentis-tools/ctx/releases/tag/v0.2.1
