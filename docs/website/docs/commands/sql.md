---
id: sql
title: ctx sql
sidebar_position: 6
---

# ctx sql

Run read-only SQL against the code-intelligence index through DuckDB, over a stable `v1` view layer.

## Synopsis

```bash
ctx sql [QUERY] [OPTIONS]
```

## Description

`ctx sql` gives you raw SQL access to the code-intelligence index. Where the
canned `ctx query` subcommands answer fixed questions (callers, deps, impact),
`ctx sql` lets you ask arbitrary ones: aggregations, joins across symbols and
edges, and custom `WHERE` conditions. **Prefer `ctx sql` whenever you need a
grouping, a join, or a filter the canned commands do not expose.**

The query surface is the versioned **`v1`** schema — a set of stable views
(`v1.symbols`, `v1.edges`, `v1.files`, `v1.meta`). See the full
[SQL Schema (v1)](../sql-schema.md) reference for every column.

- **Query `v1.*` only.** It is the compatibility contract: columns and views may
  be added within `schema_version` 1, but nothing is renamed or removed without
  bumping `v1.meta.schema_version`.
- **Anything outside `v1.*` is internal and unstable.** The raw index is
  reachable as `code.*`, but its shape can change at any time. Do not depend on it.

The query is read from the first of: the `[QUERY]` argument, `--file`, or stdin
(when `QUERY` is `-` or omitted and stdin is piped).

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
| `[QUERY]` | SQL text. If `-` or omitted while stdin is piped, read from stdin. | — |
| `--file <PATH>` | Read the query from a file (mutually exclusive with `QUERY`). | — |
| `--output <F>` | Output format: `table`, `csv`, or `json`. | `table` |
| `--json` | Alias for `--output json`. | — |
| `--max-rows <N>` | Cap returned rows (`0` = unlimited). | 1000 |
| `--timeout <SECS>` | Abort the query after N seconds. | 10 |
| `--fail-on-rows` | Exit `1` if the query returns `>= 1` row (for gate usage). | false |
| `--schema` | Print the `v1` schema reference and exit `0`. | — |

> **Note:** the flag is `--output`, not `--format` — `ctx` already has a global
> `-f/--format` for context output. `--json` is the convenient alias for
> `--output json`.

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
selects the *violations*, and any returned row fails the check. Keep gate
queries in files under `.ctx/gates/` and run them by path:

```bash
ctx sql --fail-on-rows --file .ctx/gates/no-utils-imports.sql
```

If the query returns any row, `ctx sql` exits `1` and the gate fails; zero rows
exits `0`. For example, a `.ctx/gates/no-utils-imports.sql` that flags forbidden
imports:

```sql
-- Fail if anything imports the legacy utils module
SELECT source_file, line
FROM v1.edges
WHERE kind = 'imports' AND target_name = 'utils';
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

Access is **read-only and engine-hardened**. Filesystem access, extension
loading, and file-based `ATTACH` are disabled, and the index cannot be
modified — a query can only read the `v1` (and internal `code`) views. Because
`ctx sql` cannot write to disk, mutate the index, or reach the network, it is
safe to add `Bash(ctx sql *)` to a Claude Code (or other harness) plugin
allow-list.

## See Also

- [SQL Schema (v1)](../sql-schema.md) - Full column reference for the `v1` views
- [Code Intelligence](../code-intelligence.md) - Indexing and the canned `ctx query` commands
- [ctx audit](./audit.md) - Automated quality gates
