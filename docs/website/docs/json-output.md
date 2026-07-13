---
id: json-output
title: JSON Output
sidebar_position: 8
---

# JSON Output

ctx provides a machine-readable output mode for scripting and tool integration. Pass the global `--json` flag to any supported command to get a single, stable JSON document on stdout.

```bash
ctx query find parse --json | jq '.data.symbols[].file'
```

## Contract

### stdout / stderr rules

- In JSON mode, **stdout contains exactly one JSON document and nothing else**.
- Diagnostics (progress messages, warnings, hints) go to **stderr**.
- "No results" is not an error: the command still emits a full envelope with empty arrays.

### Exit codes

The whole ctx suite follows a three-way exit-code convention (like `grep` or most linters):

| Code | Meaning |
|------|---------|
| 0    | Success, nothing to report |
| 1    | Command ran successfully but produced findings (used by quality commands) |
| 2    | Operational error (bad arguments, missing index, git failure, IO error, ...) |

> **Breaking change:** operational errors previously exited with code 1; they now exit with code 2. Exit code 1 is reserved for "ran fine, found issues".

### The envelope

Every JSON document has the same top-level shape:

```json
{
  "ctx_version": "0.3.0",
  "command": "query.find",
  "generated_at": "2026-07-09T13:30:46.623878Z",
  "data": { }
}
```

| Field | Description |
|-------|-------------|
| `ctx_version` | Version of the ctx binary that produced the output |
| `command` | Dotted command identifier (e.g. `search`, `query.callers`) |
| `generated_at` | RFC3339 UTC timestamp |
| `data` | Command-specific payload (documented below) |

All field names are `snake_case`.

### SymbolRef

Wherever a symbol appears in a payload, it is a **SymbolRef object** (never a bare string):

```json
{
  "name": "parse_file",
  "qualified_name": "Indexer::parse_file",
  "kind": "function",
  "file": "src/index/mod.rs",
  "line_start": 120,
  "line_end": 158
}
```

`qualified_name` is `null` when not known. Some commands attach extra fields alongside or inside a SymbolRef (e.g. `visibility` in `query.find`); additions are backwards-compatible, removals or renames are not.

Note: `query.graph` and `query.impact` nodes come from graph traversal, which does not track line numbers; their SymbolRefs have `line_start`/`line_end` of `0` and `qualified_name` of `null`.

## Commands

### `search`

`ctx search <QUERY> --json` (also `--output json`)

```json
{
  "query": "parse",
  "limit": 20,
  "results": [
    {
      "symbol": { "name": "parse_file", "qualified_name": null, "kind": "function", "file": "src/index/mod.rs", "line_start": 120, "line_end": 158 },
      "score": 1.0,
      "match_type": "exact",
      "signature": "fn parse_file(&mut self, path: &Path) -> Result<()>",
      "brief": "Parse a single file into symbols and edges."
    }
  ]
}
```

`match_type` is `"exact"`, `"semantic"`, or `"name"`. `score` is a relevance value in `0.0..=1.0`.

### `semantic`

`ctx semantic <QUERY> --json` (also `--output json`)

```json
{
  "query": "token counting",
  "limit": 10,
  "results": [
    {
      "symbol": { "name": "count_tokens", "qualified_name": null, "kind": "function", "file": "src/tokens.rs", "line_start": 41, "line_end": 60 },
      "symbol_id": "src/tokens.rs::count_tokens",
      "score": 0.83
    }
  ]
}
```

If no embeddings have been generated yet, `results` is empty and a hint is printed to stderr.

### `query.find`

`ctx query find <PATTERN> [--kind K] [--file F] --json`

```json
{
  "pattern": "parse",
  "filters": { "kind": "function", "file": null },
  "symbols": [
    {
      "name": "parse_file",
      "qualified_name": "Indexer::parse_file",
      "kind": "function",
      "file": "src/index/mod.rs",
      "line_start": 120,
      "line_end": 158,
      "visibility": "public"
    }
  ]
}
```

### `query.callers`

`ctx query callers <FUNCTION> [--file F] --json`

```json
{
  "target": { "name": "open_database", "qualified_name": null, "kind": "function", "file": "src/index/mod.rs", "line_start": 637, "line_end": 652 },
  "callers": [
    {
      "symbol": { "name": "run_search", "qualified_name": null, "kind": "function", "file": "src/commands/search.rs", "line_start": 14, "line_end": 41 },
      "line": 16,
      "context": "index::open_database(&root)?"
    }
  ],
  "ambiguous": []
}
```

Disambiguation: when several symbols match the name and no `--file` filter is given, `target` is `null`, `callers` is empty, and `ambiguous` lists the candidate SymbolRefs. When the symbol is not found at all, all three are empty/`null`.

### `query.deps`

`ctx query deps <SYMBOL> [--file F] [--kind K] --json`

```json
{
  "target": { "name": "run_search", "qualified_name": null, "kind": "function", "file": "src/commands/search.rs", "line_start": 14, "line_end": 41 },
  "dependencies": [
    {
      "kind": "calls",
      "target_name": "open_database",
      "line": 16,
      "resolved": { "name": "open_database", "qualified_name": null, "kind": "function", "file": "src/index/mod.rs", "line_start": 637, "line_end": 652 }
    }
  ],
  "ambiguous": []
}
```

`resolved` is `null` for unresolved (e.g. external) references. Ambiguity is reported like `query.callers`.

### `query.graph`

`ctx query graph <START> [--depth N] --json` (also `--output json`; `--output dot` and text are unchanged)

```json
{
  "root": "main",
  "depth": 3,
  "nodes": [
    {
      "symbol": { "name": "run", "qualified_name": null, "kind": "function", "file": "src/main.rs", "line_start": 0, "line_end": 0 },
      "depth": 1
    }
  ]
}
```

### `query.impact`

`ctx query impact <SYMBOL> [--depth N] --json`

```json
{
  "target": "open_database",
  "depth": 5,
  "impacted": [
    {
      "symbol": { "name": "run_search", "qualified_name": null, "kind": "function", "file": "src/commands/search.rs", "line_start": 0, "line_end": 0 },
      "distance": 1
    }
  ],
  "total": 1
}
```

### `query.stats`

`ctx query stats --json`

```json
{
  "files": 42,
  "symbols": 815,
  "functions": 500,
  "structs": 90,
  "enums": 12,
  "traits": 8,
  "edges": 2300,
  "per_file": [
    { "file": "src/index/mod.rs", "symbols": 60, "functions": 41, "public": 18, "types": 6 }
  ],
  "most_connected": [
    { "name": "open_database", "file": "src/index/mod.rs", "calls_out": 3, "called_by": 12 }
  ]
}
```

`per_file` and `most_connected` are empty when the analytics engine is unavailable (e.g. builds without the `duckdb` feature).

### `query.files`

`ctx query files --json`

```json
{
  "files": ["src/cli.rs", "src/main.rs"]
}
```

### `explain`

`ctx explain <SYMBOL> [--file F] [--kind K] --json`

```json
{
  "symbol": { "name": "run", "qualified_name": "App::run", "kind": "method", "file": "src/app.rs", "line_start": 7, "line_end": 30 },
  "visibility": "public",
  "signature": "fn run(&self) -> Result<()>",
  "brief": "Run the app",
  "docstring": null,
  "callers_count": 3,
  "deps_count": 5,
  "ambiguous": []
}
```

When the symbol is not found, `symbol` is `null` and the counts are `0`; when several symbols match without filters, `ambiguous` lists the candidates.

### `check`

`ctx check [--rules PATH] [--against REF] --json`

```json
{
  "rules_path": ".ctx/rules.toml",
  "against": "main",
  "summary": { "violations": 2, "rules_violated": 2 },
  "violations": [
    {
      "rule": "forbidden",
      "rule_id": "forbidden: domain -> infrastructure",
      "reason": "Domain layer must stay persistence-agnostic",
      "message": "src/domain/order.ts:2 -> src/infra/db.ts [calls query]",
      "file": "src/domain/order.ts",
      "line": 2,
      "from": { "name": "order", "qualified_name": null, "kind": "function", "file": "src/domain/order.ts", "line_start": 2, "line_end": 2 },
      "to": { "name": "query", "qualified_name": null, "kind": "function", "file": "src/infra/db.ts", "line_start": 1, "line_end": 1 }
    },
    {
      "rule": "limit",
      "rule_id": "limit: fan_in <= 25 (symbol)",
      "reason": "fan_in 30 exceeds max 25",
      "message": "src/app/hub.ts:10 (handle): fan_in 30 exceeds max 25",
      "file": "src/app/hub.ts",
      "line": 10,
      "subject": { "name": "handle", "qualified_name": null, "kind": "function", "file": "src/app/hub.ts", "line_start": 10, "line_end": 42 },
      "metric": "fan_in",
      "scope": "symbol",
      "value": 30,
      "max": 25
    }
  ]
}
```

One entry per violation. `rule` is one of `forbidden`, `allowed_dependents`, `limit`, or `no_new_dependents`; `rule_id` identifies the specific rule instance (violations with the same `rule_id` belong to the same rule).

Dependency violations (`forbidden`, `allowed_dependents`, `no_new_dependents`) carry `from`/`to` endpoints. Symbol-level endpoints (resolved call/implements/extends/uses edges) are full SymbolRefs; file-level endpoints (resolved imports) are `{"file": ...}` objects. `limit` violations carry a `subject` endpoint plus `metric`, `scope`, `value`, and `max`. Absent optional fields (`line`, `from`, `to`, `subject`, the metric fields) are omitted rather than `null`. `against` is `null` when `--against` was not given.

Exit codes follow the suite convention: 0 = no violations, 1 = at least one violation, 2 = operational error (missing/invalid rules file, unknown or overlapping layers, missing index, bad git ref).

### `check.list`

`ctx check --list --json`

```json
{
  "rules_path": ".ctx/rules.toml",
  "version": 1,
  "layers": [
    { "name": "domain", "patterns": ["src/domain/**"], "files": 12 }
  ],
  "rules": {
    "forbidden": [ { "from": "domain", "to": "infrastructure", "reason": "Domain layer must stay persistence-agnostic" } ],
    "allowed_dependents": [ { "layer": "infrastructure", "only": ["application"], "reason": null } ],
    "limit": [ { "metric": "fan_in", "scope": "symbol", "max": 25, "exclude": ["src/core/**"] } ],
    "no_new_dependents": [ { "paths": ["src/legacy/**"], "reason": "Legacy module is frozen; do not add new callers" } ]
  }
}
```

`files` is the number of indexed files matching the layer's globs. `--list` always exits 0.

### `duplicates`

`ctx duplicates [--threshold F] [--min-tokens N] [--against REF] [--fail-on-found] --json`

```json
{
  "threshold": 0.85,
  "min_tokens": 50,
  "against": null,
  "skipped_languages": [],
  "pairs": [
    {
      "a": { "name": "process_orders", "qualified_name": null, "kind": "function", "file": "src/orders.rs", "line_start": 12, "line_end": 30 },
      "b": { "name": "sum_invoices", "qualified_name": null, "kind": "function", "file": "src/invoices.rs", "line_start": 4, "line_end": 22 },
      "similarity": 0.97,
      "token_count_a": 64,
      "token_count_b": 64
    }
  ]
}
```

`similarity` is the exact Jaccard similarity (0.0-1.0) of the two functions' normalized 5-token shingle sets. Pairs are sorted by similarity (descending), then by symbol id. `skipped_languages` lists languages that are never fingerprinted; all currently supported languages (including Solidity) are fingerprinted, so it is normally empty. With `--fail-on-found`, a non-empty `pairs` array exits with code 1.

### `score`

`ctx score [--against REF] [--fail-on EXPR] --json`

```json
{
  "against": "main",
  "files_changed": 1,
  "metrics": {
    "complexity_delta": 3,
    "fan_out_delta": 1,
    "new_duplication": 0,
    "check_violations": 0,
    "symbols_added": 1,
    "symbols_removed": 0,
    "files_changed": 1
  },
  "check_violations_note": "no rules file",
  "per_file": [
    {
      "path": "src/a.rs",
      "complexity_baseline": 3,
      "complexity_current": 6,
      "fan_out_baseline": 1,
      "fan_out_current": 2,
      "symbols_added": 1,
      "symbols_removed": 0
    }
  ],
  "failed_conditions": [],
  "notes": ["fan_in approximated as same-file callers for baseline comparability"]
}
```

`metrics` is flat and its keys are exactly the metric names accepted by `--fail-on` (`complexity_delta`, `fan_out_delta`, `new_duplication`, `check_violations`, `symbols_added`, `symbols_removed`, `files_changed`). `per_file` breaks the delta metrics down per changed file with both sides (`*_baseline` from parsing the file's content at the reference in memory, `*_current` from the index). `failed_conditions` lists every satisfied `--fail-on` condition in canonical `metric OP value` form (non-empty means exit code 1). `check_violations_note` is `"no rules file"` (and `check_violations` is `0`) when `.ctx/rules.toml` does not exist, `null` otherwise. `notes` carries caveats, always including the fan-in approximation note: baselines are parsed in isolation, so fan-in counts same-file callers on both sides.

Exit codes follow the suite convention: 0 = clean or informational, 1 = at least one `--fail-on` condition met, 2 = operational error (not a git repo, bad reference, malformed `--fail-on`, invalid rules file).

Separately from `--json`, setting the `CTX_GATE_LOG` environment variable makes every `ctx score` run append one **JSONL** record (a bare JSON object per line — *not* this envelope) describing the gate evaluation to a local log, default `.ctx/gate-log.jsonl`. See [ctx score — Gate logging](commands/score.md#gate-logging).

### `snapshot.capture`

`ctx snapshot [--force] [--churn-window SPEC] --json`

```json
{
  "commit_sha": "86258796aa7f19c06d310f6abce6c5f56465e316",
  "committed_at": "2026-07-10T21:25:27+02:00",
  "partition_dir": ".ctx/snapshots/sha=86258796aa7f19c06d310f6abce6c5f56465e316",
  "files": 2,
  "symbols": 3,
  "dup_pairs": 0,
  "violations": 0,
  "skipped_existing": false
}
```

One report per captured partition. `commit_sha` and `committed_at` (the committer date, strict ISO 8601) identify the snapshotted commit; `partition_dir` is where the four Parquet files were written. `files`, `symbols`, and `dup_pairs` are the row counts of the corresponding Parquet tables; `violations` is the total architecture-rule violation count (0 when `.ctx/rules.toml` is absent). When a partition for the commit already exists and `--force` was not given, `skipped_existing` is `true` and the counts are reported as zero — they are not re-read from the existing files.

Exit codes: 0 = snapshot written or partition already existed, 2 = operational error (not a git repo, build without the `duckdb` feature, IO failure).

### `snapshot.backfill`

`ctx snapshot backfill --since REF [--every N] [--churn-window SPEC] --json`

```json
{
  "since": "3019df548fc417c7b6b06bef7defb74a0c01ba78",
  "captured": 1,
  "skipped_existing": 1,
  "snapshots": [
    {
      "commit_sha": "3019df548fc417c7b6b06bef7defb74a0c01ba78",
      "committed_at": "2026-07-10T21:25:27+02:00",
      "partition_dir": ".ctx/snapshots/sha=3019df548fc417c7b6b06bef7defb74a0c01ba78",
      "files": 1,
      "symbols": 2,
      "dup_pairs": 0,
      "violations": 0,
      "skipped_existing": false
    },
    {
      "commit_sha": "86258796aa7f19c06d310f6abce6c5f56465e316",
      "committed_at": "2026-07-10T21:25:27+02:00",
      "partition_dir": ".ctx/snapshots/sha=86258796aa7f19c06d310f6abce6c5f56465e316",
      "files": 0,
      "symbols": 0,
      "dup_pairs": 0,
      "violations": 0,
      "skipped_existing": true
    }
  ]
}
```

`since` is the `--since` argument as given. `snapshots` carries one `snapshot.capture`-shaped report per partition, oldest first; `captured` and `skipped_existing` are the counts of new vs. already-existing partitions among them. Commits that failed to capture are logged to stderr and **do not appear** in `snapshots` — the walk continues past them, and the exit code stays 0.

### `harness.init`

`ctx harness init [--target claude|codex] [--mode local|plugin] [--force] --json`

Codex local mode uses `"target": "codex"` and returns `agents_md_block` in place
of the Claude-specific `settings_snippet` and `claude_md_block` fields.

```json
{
  "mode": "local",
  "target": "claude",
  "force": false,
  "files": [
    { "path": ".claude/hooks/ctx/session-start.sh", "action": "created" },
    { "path": ".claude/hooks/ctx/post-tool-use.sh", "action": "regenerated" },
    { "path": ".claude/hooks/ctx/stop.sh", "action": "skipped_modified" },
    { "path": ".ctx/rules.toml", "action": "skipped_policy" },
    { "path": ".ctx/harness.lock", "action": "regenerated" }
  ],
  "settings_snippet": "{ ... }",
  "claude_md_block": "## Code intelligence (ctx) ..."
}
```

One `files` entry per planned file. `action` is one of `created` (did not exist), `regenerated` (owned by ctx and unmodified), `overwritten` (`--force` replaced a modified or foreign file), `skipped_modified` (checksum no longer matches; use `--force`), `skipped_foreign` (exists but was not generated by ctx), or `skipped_policy` (`.ctx/rules.toml`, never overwritten). `settings_snippet` and `claude_md_block` are only present in local mode; they carry the exact text that non-JSON mode prints to stdout for manual inclusion.

### `harness.doctor`

`ctx harness doctor --json`

```json
{
  "healthy": false,
  "binary_version": "0.3.0",
  "mcp_compiled": false,
  "templates_version": "0.3.0",
  "summary": { "errors": 1, "warnings": 1, "info": 1 },
  "checks": [
    { "severity": "info", "code": "binary_version", "message": "ctx v0.3.0 (mcp feature: not compiled in)" },
    { "severity": "warning", "code": "index_missing", "message": "no code intelligence index (.ctx/codebase.sqlite)", "hint": "run 'ctx index' to build it" },
    { "severity": "error", "code": "rules_invalid", "message": ".ctx/rules.toml is invalid: ...", "hint": "fix the rules file; 'ctx check --help' documents the format" }
  ]
}
```

`severity` is `error`, `warning`, or `info`; `hint` is omitted when there is none. `code` is a stable machine-readable identifier: `binary_version`, `harness_not_initialized`, `templates_stale`, `index_missing`, `index_schema`, `index_stale`, `rules_missing`, `rules_invalid`, `hooks_missing`, `hooks_modified`, `settings_not_wired`, `mcp_unavailable`, `mcp_not_wired`. Checks are independent (a missing index and an invalid rules file are both reported in one run). `healthy` is `true` when no check is an error or a warning; exit codes: 0 = healthy, 1 = problems, 2 = operational error.

### `self_update`

`ctx self-update [--version X.Y.Z] --json`

```json
{
  "old_version": "0.2.1",
  "new_version": "0.3.0",
  "outcome": "updated"
}
```

`outcome` is `updated` (the binary was replaced) or `up_to_date` (nothing to do). Failures
(package-manager-owned installation, network error, checksum mismatch, install location not
writable, or unsupported platform) exit 2 without emitting an envelope. Note that `--json` also
suppresses the passive update notice — in JSON mode nothing but the envelope is ever printed to
stdout, and update notices only ever go to stderr.

### `version.check`

`ctx --version --check --json`

```json
{
  "current_version": "0.2.1",
  "latest_version": "0.3.0",
  "update_available": true
}
```

Explicit release comparison (never installs anything). Exits 0 whether or not an update exists; network failures exit 2.

## Legacy shapes

`ctx complexity --output json`, `ctx graph --output json`, and `ctx audit --output json` still emit their old, ad-hoc JSON shapes. They will be migrated to the envelope in a future release. The old ad-hoc shapes of `search --output json` and `semantic --output json` have already been **replaced** by the envelope described above.
