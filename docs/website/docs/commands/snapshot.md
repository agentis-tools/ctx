---
id: snapshot
title: ctx snapshot
sidebar_position: 14
---

# ctx snapshot

Capture per-commit Parquet metric snapshots for longitudinal quality analysis.

## Synopsis

```bash
ctx snapshot [--force] [--churn-window <SPEC>] [--json]
ctx snapshot backfill --since <REF> [--every <N>] [--churn-window <SPEC>] [--json]
```

## Description

Point-in-time commands like [`ctx score`](./score.md) answer "did *this change* make the code better or worse?". `ctx snapshot` answers the longitudinal version: **is the codebase trending better or worse over weeks and months?** Each run exports the current commit's per-file and per-symbol metrics, near-duplicate pairs, and capture metadata as one Parquet partition:

```text
.ctx/snapshots/sha=<sha>/symbols.parquet     per-symbol metrics
.ctx/snapshots/sha=<sha>/files.parquet       per-file metrics + churn + violations
.ctx/snapshots/sha=<sha>/dup_pairs.parquet   near-duplicate function pairs
.ctx/snapshots/sha=<sha>/meta.parquet        capture metadata (1 row)
```

Accumulate partitions over time (one per commit), then query them with [`ctx sql --snapshots`](#querying-snapshots-with-ctx-sql) to plot duplication, violation, and hotspot trends across the history.

A bare `ctx snapshot` captures HEAD: the index is refreshed incrementally first (same as `ctx score`), then the four Parquet files are written to a staging directory and moved into place with an atomic rename — readers never observe a half-written partition. If a partition for HEAD's sha already exists the command is a no-op (exit 0); `--force` rewrites it. When the working tree is dirty, a stderr warning notes that the snapshot is labeled with HEAD's sha but reflects the working tree.

Snapshot capture requires the `duckdb` feature (on by default); builds without it exit 2.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--force` | Overwrite an existing partition for HEAD | false |
| `--churn-window <SPEC>` | How far back to count per-file churn (a `git log --since` date spec) | `"90 days ago"` |
| `--json` | Machine-readable output (global flag) | false |

`backfill` adds:

| Option | Description | Default |
|--------|-------------|---------|
| `--since <REF>` | Starting commit/ref — the walk covers `REF..HEAD` first-parent, plus `REF` itself when it names a commit | required |
| `--every <N>` | Sample every Nth commit; sampling counts backwards from HEAD so the newest commit is always included | 1 |

## Partition contents

Every row of every table is denormalized with the partition stamp, so partitions union cleanly across commits with a single `read_parquet('.ctx/snapshots/*/*.parquet')` glob per table:

| Column | Type | Description |
|--------|------|-------------|
| `commit_sha` | VARCHAR | Full sha of the snapshotted commit |
| `committed_at` | TIMESTAMP | Committer date of that commit, normalized to UTC |

### `symbols.parquet` — one row per symbol

Stamp columns plus `id`, `name`, `qualified_name`, `kind`, `file`, `line_start`, `line_end`, `is_public`, `complexity`, `fan_in`, `fan_out` — the same columns (and types) as the `v1.symbols` SQL view, minus `doc`.

### `files.parquet` — one row per file

Stamp columns plus:

| Column | Type | Description |
|--------|------|-------------|
| `path` | VARCHAR | File path |
| `language` | VARCHAR | Detected language |
| `symbol_count` | BIGINT | Symbols defined in the file |
| `total_complexity` | DOUBLE | Sum of symbol complexity for the file |
| `max_complexity` | BIGINT | Highest single-symbol complexity in the file |
| `churn_commits` | INTEGER | Commits touching the file within the churn window |
| `violation_count` | INTEGER | Architecture-rule violations in the file (0 when `.ctx/rules.toml` is absent) |

### `dup_pairs.parquet` — one row per near-duplicate pair

Stamp columns plus:

| Column | Type | Description |
|--------|------|-------------|
| `file_a` / `file_b` | VARCHAR | Files of the two symbols |
| `symbol_a` / `symbol_b` | VARCHAR | Names of the two symbols |
| `similarity` | DOUBLE | Verified token similarity (0–1) |
| `token_count_a` / `token_count_b` | BIGINT | Normalized token counts |

Pairs use the same detector and thresholds as [`ctx score`](./score.md)'s `new_duplication` (Jaccard >= 0.85, >= 50 tokens).

### `meta.parquet` — one row per partition

Stamp columns plus:

| Column | Type | Description |
|--------|------|-------------|
| `captured_at` | VARCHAR | RFC 3339 time the snapshot was captured |
| `ctx_version` | VARCHAR | ctx version that wrote the partition |
| `snapshot_schema_version` | INTEGER | Snapshot Parquet schema version (currently 1) |
| `capture_mode` | VARCHAR | `live` (bare `ctx snapshot`) or `backfill` |

## Backfilling history

`ctx snapshot backfill --since <REF>` captures partitions for historical commits so trend queries have a past to look at. It walks the **first-parent** range `REF..HEAD` oldest-first (including `REF` itself when it resolves to a commit), checks each missing commit out into a temporary `git worktree`, snapshots it into *this* repository's `.ctx/snapshots/`, and removes the worktree again — your working tree is never touched.

```bash
ctx snapshot backfill --since v0.1.0             # every first-parent commit since v0.1.0
ctx snapshot backfill --since main~200 --every 5 # sample every 5th commit
```

Caveats:

- **First-parent only.** Commits that live on merged side branches are not walked; a squash/merge-based history is covered completely, a fast-forward-heavy one is not.
- **The churn window's lower bound is relative to now.** `--churn-window` is a `git log --since` date spec, and git resolves relative specs like `"90 days ago"` against the wall clock — even in backfill mode, where only the *upper* bound is anchored to each commit's date. For old commits, a relative window therefore spans more than 90 days of their history; pass an absolute date if that matters to your analysis.
- **Per-commit failures are skipped, not fatal.** A commit that fails to index or export is logged to stderr and the walk continues; the final report covers the captured and already-existing partitions only. Existing partitions are always skipped (no `--force` in backfill mode).

## JSON output

`ctx snapshot --json` emits the standard envelope with command `snapshot.capture`:

```json
{
  "command": "snapshot.capture",
  "ctx_version": "0.3.0",
  "data": {
    "commit_sha": "86258796aa7f19c06d310f6abce6c5f56465e316",
    "committed_at": "2026-07-10T21:25:27+02:00",
    "dup_pairs": 0,
    "files": 2,
    "partition_dir": ".ctx/snapshots/sha=86258796aa7f19c06d310f6abce6c5f56465e316",
    "skipped_existing": false,
    "symbols": 3,
    "violations": 0
  },
  "generated_at": "2026-07-10T19:25:28.09532Z"
}
```

`ctx snapshot backfill --json` emits `snapshot.backfill` with per-partition reports:

```json
{
  "command": "snapshot.backfill",
  "ctx_version": "0.3.0",
  "data": {
    "captured": 1,
    "since": "3019df548fc417c7b6b06bef7defb74a0c01ba78",
    "skipped_existing": 1,
    "snapshots": [
      {
        "commit_sha": "3019df548fc417c7b6b06bef7defb74a0c01ba78",
        "committed_at": "2026-07-10T21:25:27+02:00",
        "dup_pairs": 0,
        "files": 1,
        "partition_dir": ".ctx/snapshots/sha=3019df548fc417c7b6b06bef7defb74a0c01ba78",
        "skipped_existing": false,
        "symbols": 2,
        "violations": 0
      },
      {
        "commit_sha": "86258796aa7f19c06d310f6abce6c5f56465e316",
        "committed_at": "2026-07-10T21:25:27+02:00",
        "dup_pairs": 0,
        "files": 0,
        "partition_dir": ".ctx/snapshots/sha=86258796aa7f19c06d310f6abce6c5f56465e316",
        "skipped_existing": true,
        "symbols": 0,
        "violations": 0
      }
    ]
  },
  "generated_at": "2026-07-10T19:25:28.670878Z"
}
```

When `skipped_existing` is true the partition was left untouched and its row counts are reported as zero — they are not re-read from the existing Parquet files. See [JSON Output](../json-output.md).

## Querying snapshots with `ctx sql`

`ctx sql --snapshots[=DIR]` (default `DIR` is `.ctx/snapshots`) loads the partitions as `snap.files`, `snap.symbols`, `snap.dup_pairs`, and `snap.meta` tables alongside the usual `v1` views. For example, the violation trend across all snapshotted commits:

```bash
ctx sql --snapshots "SELECT commit_sha, min(committed_at) AS committed_at,
       sum(violation_count) AS violations
FROM snap.files
GROUP BY commit_sha
ORDER BY committed_at;"
```

The `snap.*` column reference and more canned trend queries (duplication trend, hotspot mass) live in the SQL schema reference — run `ctx sql --schema`, or see the [SQL Schema (v1)](../sql-schema.md) reference.

## Snapshots in CI (data branch)

To build the trend history automatically, capture one partition per merge to your default branch and append it to an orphan *data branch*, keeping metric history out of your main history. ctx's own repository does this in `.github/workflows/snapshot.yml`: on every push to `main` it runs `ctx index && ctx snapshot --json`, checks the `ctx-snapshots` orphan branch out into a linked worktree, copies the new partition in, and pushes (runs are serialized via a concurrency group, and re-runs on the same sha are no-ops). Analyze the accumulated history from any machine:

```bash
git fetch origin ctx-snapshots
git worktree add ../snapshots ctx-snapshots
ctx sql --snapshots=../snapshots/snapshots "SELECT ..."
```

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Snapshot written, or the partition already existed |
| 2 | Operational error (not a git repo, build without the `duckdb` feature, IO failure) |

## Examples

```bash
ctx snapshot                                 # snapshot HEAD (skip if it exists)
ctx snapshot --force                         # rewrite the HEAD partition
ctx snapshot --json                          # machine-readable report
ctx snapshot --churn-window "180 days ago"   # wider churn window
ctx snapshot backfill --since v0.1.0         # snapshot v0.1.0..HEAD (first-parent)
ctx snapshot backfill --since main~20 --every 5
ctx sql --snapshots "SELECT commit_sha, count(*) FROM snap.dup_pairs GROUP BY commit_sha"
```

## See Also

- [ctx score](./score.md) — the point-in-time quality delta the snapshots accumulate over history
- [ctx duplicates](./duplicates.md) — the near-duplicate detector behind `dup_pairs.parquet`
- [ctx check](./check.md) — the architecture rules behind `violation_count`
- [JSON Output](../json-output.md)
