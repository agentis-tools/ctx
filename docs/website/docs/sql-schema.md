---
id: sql-schema
title: SQL Schema (v1)
sidebar_position: 8
---

# ctx sql — Public Schema (`v1`)

`ctx sql` runs read-only SQL against the code-intelligence index through DuckDB.
The query surface is the versioned **`v1`** schema: a set of stable views over
the physical index. Query `v1.*` — not the underlying tables.

## Stability contract

- **`v1.*` is the contract.** Columns and views may be *added* within
  `schema_version` 1. Renaming or removing anything increments
  `v1.meta.schema_version` and is noted in the changelog.
- **Everything outside `v1.*` is internal and unstable.** The raw index is
  reachable as `code.*`, but its shape can change at any time and it is excluded
  from all compatibility guarantees. Do not depend on it.
- **Access is read-only and engine-hardened.** Filesystem access, extension
  loading, and attaching other databases are disabled; the index cannot be
  modified.

## Views

### `v1.symbols` — one row per symbol

| Column           | Type    | Description                                        |
|------------------|---------|----------------------------------------------------|
| `id`             | VARCHAR | Stable symbol identifier                            |
| `name`           | VARCHAR | Symbol name                                        |
| `qualified_name` | VARCHAR | Fully-qualified name, when known                   |
| `kind`           | VARCHAR | `function`, `method`, `struct`, `enum`, `trait`, … |
| `file`           | VARCHAR | Path of the file that defines the symbol           |
| `line_start`     | BIGINT  | First line of the symbol                           |
| `line_end`       | BIGINT  | Last line of the symbol                            |
| `is_public`      | BOOLEAN | Whether the symbol is publicly visible             |
| `complexity`     | BIGINT  | `fan_out * 2 + fan_in` heuristic complexity score  |
| `fan_in`         | BIGINT  | Number of resolved incoming `calls` edges          |
| `fan_out`        | BIGINT  | Number of outgoing `calls` edges                   |
| `doc`            | VARCHAR | Docstring or brief, when present                   |

### `v1.edges` — one row per relationship

| Column        | Type    | Description                                              |
|---------------|---------|---------------------------------------------------------|
| `source_id`   | VARCHAR | `id` of the source symbol                               |
| `source_name` | VARCHAR | Name of the source symbol                               |
| `source_file` | VARCHAR | File of the source symbol                               |
| `target_id`   | VARCHAR | `id` of the target symbol; NULL when unresolved         |
| `target_name` | VARCHAR | Name of the target; retained even when unresolved       |
| `target_file` | VARCHAR | File of the target symbol; NULL when unresolved         |
| `kind`        | VARCHAR | Relationship kind, including `calls`, `uses`, `extends`, `implements`, or `imports` |
| `line`        | BIGINT  | Line of the reference in the source file                |

### `v1.files` — one row per indexed file

| Column             | Type    | Description                                  |
|--------------------|---------|----------------------------------------------|
| `path`             | VARCHAR | File path                                    |
| `language`         | VARCHAR | Detected language                            |
| `symbol_count`     | BIGINT  | Number of symbols defined in the file        |
| `total_complexity` | BIGINT  | Sum of `v1.symbols.complexity` for the file  |
| `indexed_at`       | BIGINT  | Unix time the file was last indexed          |

### `v1.meta` — single row of index metadata

| Column             | Type    | Description                                  |
|--------------------|---------|----------------------------------------------|
| `schema_version`   | INTEGER | Public schema version (starts at 1)          |
| `ctx_version`      | VARCHAR | Version of ctx that produced this output     |
| `index_created_at` | BIGINT  | Earliest file index time (Unix seconds)      |
| `index_root`       | VARCHAR | Absolute root path of the indexed project    |

## Examples

```sql
-- Ten most complex symbols
SELECT name, file, complexity
FROM v1.symbols
ORDER BY complexity DESC
LIMIT 10;
```

```sql
-- Symbol counts by kind
SELECT kind, COUNT(*) AS n
FROM v1.symbols
GROUP BY kind
ORDER BY n DESC;
```

```sql
-- Public functions that nothing calls (dead-code candidates)
SELECT name, file
FROM v1.symbols
WHERE kind IN ('function', 'method') AND is_public AND fan_in = 0
ORDER BY file, name;
```

Rust function items passed directly as callback values are exposed as `uses`
edges. They are intentionally excluded from `fan_in`, `fan_out`, call graphs,
and impact analysis:

```sql
SELECT source_name, target_name, target_file
FROM v1.edges
WHERE kind = 'uses';
```

## Snapshot tables (`snap.*`) — only with `--snapshots`

`ctx sql --snapshots[=DIR]` (default `DIR` is `.ctx/snapshots`) additionally
loads the Parquet snapshot partitions written by `ctx snapshot` as in-memory
tables in the `snap` schema. These tables exist **only** when `--snapshots`
is passed; without it, any `snap.*` reference is an error. Every row is
denormalized with the partition stamp:

| Column         | Type      | Description                                    |
|----------------|-----------|------------------------------------------------|
| `commit_sha`   | VARCHAR   | Full sha of the snapshotted commit             |
| `committed_at` | TIMESTAMP | Committer date of that commit, normalized to UTC |

### `snap.files` — one row per file per commit

Stamp columns plus:

| Column             | Type    | Description                                   |
|--------------------|---------|-----------------------------------------------|
| `path`             | VARCHAR | File path                                     |
| `language`         | VARCHAR | Detected language                             |
| `symbol_count`     | BIGINT  | Symbols defined in the file                   |
| `total_complexity` | DOUBLE  | Sum of symbol complexity for the file         |
| `max_complexity`   | BIGINT  | Highest single-symbol complexity in the file  |
| `churn_commits`    | INTEGER | Commits touching the file in the churn window |
| `violation_count`  | INTEGER | Architecture-rule violations in the file      |

### `snap.symbols` — one row per symbol per commit

Stamp columns plus the `v1.symbols` columns `id`, `name`, `qualified_name`,
`kind`, `file`, `line_start`, `line_end`, `is_public`, `complexity`,
`fan_in`, and `fan_out` (same types as in `v1.symbols`; no `doc`).

### `snap.dup_pairs` — one row per near-duplicate pair per commit

Stamp columns plus:

| Column          | Type    | Description                            |
|-----------------|---------|----------------------------------------|
| `file_a`        | VARCHAR | File of the first symbol               |
| `symbol_a`      | VARCHAR | Name of the first symbol               |
| `file_b`        | VARCHAR | File of the second symbol              |
| `symbol_b`      | VARCHAR | Name of the second symbol              |
| `similarity`    | DOUBLE  | Verified token similarity (0–1)        |
| `token_count_a` | BIGINT  | Normalized token count of the first    |
| `token_count_b` | BIGINT  | Normalized token count of the second   |

### `snap.meta` — one row per partition

Stamp columns plus:

| Column                    | Type    | Description                               |
|---------------------------|---------|-------------------------------------------|
| `captured_at`             | VARCHAR | RFC 3339 time the snapshot was captured   |
| `ctx_version`             | VARCHAR | ctx version that wrote the partition      |
| `snapshot_schema_version` | INTEGER | Snapshot Parquet schema version           |
| `capture_mode`            | VARCHAR | `live` or `backfill`                      |

### Trend queries

```sql
-- Duplication trend
SELECT commit_sha, min(committed_at) AS committed_at, count(*) AS dup_pairs FROM snap.dup_pairs GROUP BY commit_sha ORDER BY committed_at;
```

```sql
-- Violation trend
SELECT commit_sha, min(committed_at) AS committed_at, sum(violation_count) AS violations FROM snap.files GROUP BY commit_sha ORDER BY committed_at;
```

```sql
-- Hotspot mass (top-decile complexity by commit)
WITH ranked AS (SELECT commit_sha, committed_at, churn_commits * total_complexity AS mass, percent_rank() OVER (PARTITION BY commit_sha ORDER BY total_complexity) AS pr FROM snap.files) SELECT commit_sha, min(committed_at) AS committed_at, sum(mass) AS hotspot_mass FROM ranked WHERE pr >= 0.9 GROUP BY commit_sha ORDER BY committed_at;
```
