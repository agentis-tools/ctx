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
  "ctx_version": "0.2.1",
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

## Legacy shapes

`ctx complexity --output json`, `ctx graph --output json`, and `ctx audit --output json` still emit their old, ad-hoc JSON shapes. They will be migrated to the envelope in a future release. The old ad-hoc shapes of `search --output json` and `semantic --output json` have already been **replaced** by the envelope described above.
