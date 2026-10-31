# ctx sql

Run read-only SQL against the code intelligence index through DuckDB, over a stable `v1` view layer.

## Synopsis

```bash
ctx sql [QUERY] [OPTIONS]
```

## Description

`ctx sql` gives you raw SQL access to the code intelligence index. The
`ctx query` subcommands answer fixed questions (callers, deps, impact).
`ctx sql` answers any question you can write as SQL: aggregations, joins across
symbols and edges, and custom `WHERE` conditions. Use it when you need a
grouping, a join, or a filter that `ctx query` does not offer.

Queries run against the versioned `v1` schema, a set of stable views
(`v1.symbols`, `v1.edges`, `v1.files`, `v1.meta`). See the
[SQL schema (v1)](https://docs.agentis.tools/docs/sql-schema) reference for every column.

- Query `v1.*` only. It is the compatibility contract: new columns and views may
  appear within `schema_version` 1, but nothing is renamed or removed without
  a new `v1.meta.schema_version`.
- Anything outside `v1.*` is internal and can change at any time. The raw index is
  visible as `code.*`, but do not depend on it.

Pass the query as the `[QUERY]` argument, with `--file`, or on stdin (when `QUERY` is `-` or
omitted and stdin is piped). Passing both `QUERY` and `--file` is an error.

Imports are stored as per-file module metadata, not as `v1.edges` rows, so an
`imports` filter on `v1.edges` returns nothing. Use [`ctx check`](check.md) rules for import
boundaries.

### Programmatic use

For agents and scripts, pass `--json` for machine-readable rows and
`--max-rows` to bound the result set:

```bash
ctx sql --json --max-rows 50 "SELECT name, complexity FROM v1.symbols ORDER BY complexity DESC"
```

An index must exist first (`ctx index`); querying without one exits with code 2.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `[QUERY]` | SQL text. If `-` or omitted while stdin is piped, read from stdin. | none |
| `--file <PATH>` | Read the query from a file (mutually exclusive with `QUERY`). | none |
| `--output <F>` | Output format: `table`, `csv`, or `json`. | `table` |
| `--json` | Alias for `--output json`. | false |
| `--max-rows <N>` | Cap returned rows (`0` = unlimited). | 1000 |
| `--timeout <SECS>` | Abort the query after N seconds. | 10 |
| `--fail-on-rows` | Exit `1` if the query returns `>= 1` row (for gate usage). | false |
| `--schema` | Print the `v1` schema reference and exit `0`. | false |
| `--snapshots[=DIR]` | Load snapshot partitions from DIR as tables `snap.files`, `snap.symbols`, `snap.dup_pairs`, and `snap.meta` for trend queries (see [ctx snapshot](./snapshot.md)) | `.ctx/snapshots` |

The flag is `--output`, not `--format`, because `ctx` already has a global
`-f/--format` for context output. `--json` is a shorter alias for
`--output json`.

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | Query ran successfully (with `--fail-on-rows`: zero rows returned). |
| `1` | `--fail-on-rows` was set and the query returned `>= 1` row. |
| `2` | Any error: SQL error, timeout, missing index, invalid flags, or a build without the `duckdb` feature. |

## Examples

### Ten most complex symbols

```bash
ctx sql "SELECT name, file, complexity
FROM v1.symbols
ORDER BY complexity DESC
LIMIT 10;"
```

### Symbol counts by kind

```bash
ctx sql "SELECT kind, COUNT(*) AS n
FROM v1.symbols
GROUP BY kind
ORDER BY n DESC;"
```

### Public functions that nothing calls (dead-code candidates)

```bash
ctx sql "SELECT name, file
FROM v1.symbols
WHERE kind IN ('function', 'method') AND is_public AND fan_in = 0
ORDER BY file, name;"
```

### Print the schema

```bash
ctx sql --schema
```

## Gates

`--fail-on-rows` turns a query into a CI or pre-commit gate: write SQL that
selects the violations, and any returned row fails the check. Keep gate
queries in files under `.ctx/gates/` and run them by path:

```bash
ctx sql --fail-on-rows --file .ctx/gates/no-legacy-calls.sql
```

If the query returns any row, `ctx sql` exits `1` and the gate fails. Zero rows
exit `0`. For example, a `.ctx/gates/no-legacy-calls.sql` that flags new code
calling into a legacy module:

```sql
-- Fail if code outside src/legacy/ calls into it
SELECT source_file, source_name, target_name, line
FROM v1.edges
WHERE kind = 'calls'
  AND target_file LIKE 'src/legacy/%'
  AND source_file NOT LIKE 'src/legacy/%';
```

Wire it into a pre-commit hook:

```bash
#!/usr/bin/env bash
# .git/hooks/pre-commit
ctx index >/dev/null
for gate in .ctx/gates/*.sql; do
  ctx sql --fail-on-rows --file "$gate" || {
    echo "Gate failed: $gate" >&2
    exit 1
  }
done
```

## Security

Access is read-only. ctx disables filesystem access, extension loading, and
file-based `ATTACH` in the DuckDB engine, and the index cannot be modified: a
query can only read the `v1` (and internal `code`) views. ctx never downloads
DuckDB extensions. It attaches the index through a locally installed
`sqlite_scanner` extension when one exists and otherwise reads a private copy,
so `ctx sql` works offline. Because it cannot write to disk, change the index,
or reach the network, you can add `Bash(ctx sql *)` to a Claude Code (or other
harness) plugin allow-list.

## See also

- [SQL schema (v1)](https://docs.agentis.tools/docs/sql-schema): column reference for the `v1` views
- [Code intelligence](../code-intelligence.md): indexing and the `ctx query` commands
- [ctx snapshot](./snapshot.md): per-commit metrics for trend queries
- [ctx audit](./audit.md): automated quality gates
