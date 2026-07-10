---
id: sql-gates
title: SQL gates (preview)
sidebar_position: 6
---

# SQL gates <span title="Preview — not yet released">🧪</span>

:::caution Preview — not yet released
`ctx sql` and repo-committed `.ctx/gates/*.sql` files are **planned**, not shipped. This page
describes the intended design so you can follow along; **the exact command name, flags, and behavior
may change** before release. Nothing here works on the current binary yet — track the
[changelog](https://github.com/saldestechnology/ctx/blob/main/CHANGELOG.md) for availability.
:::

Where [`ctx check`](check.md) enforces a fixed set of architecture rule *kinds* and
[`ctx score`](score.md) gates on a fixed set of *metrics*, **SQL gates** let a team express arbitrary
guardrails as **plain SQL committed to the repo** — the escape hatch for the questions the built-in
rules don't cover.

They build on a capability ctx already has: `ctx query --sql "<SELECT ...>"` runs read-only SQL over
the code-intelligence index (a versioned SQLite/DuckDB schema of symbols, edges, and metrics). SQL
gates turn that from an ad-hoc query into a versioned, reviewable policy.

## The idea

A gate is a `.sql` file in `.ctx/gates/`. It is a `SELECT` that returns **one row per violation**.
An empty result means the gate passes; any rows mean it fails.

```sql
-- .ctx/gates/no-todo-in-public-api.sql
-- Public functions must not ship with a TODO in their docstring.
SELECT name, file, line_start
FROM symbols
WHERE visibility = 'public'
  AND docstring LIKE '%TODO%';
```

```bash
# Provisional CLI — subject to change
ctx sql .ctx/gates/no-todo-in-public-api.sql --fail-on-rows   # exit 1 if any row returns
ctx sql --gates .ctx/gates/ --fail-on-rows                    # run every gate in the directory
```

Because gates are files in the repo, they are **versioned, diffable, and reviewed like code** — a
team's standards live next to the code they govern, not in a dashboard's config. Recurring gate
shapes become candidates for promotion into first-class [`check`](check.md) rules.

## How it will fit the suite

- Same [exit-code contract](../reference/exit-codes.md): `0` clean, `1` rows found (with
  `--fail-on-rows`), `2` operational error.
- Same [`--json` envelope](../reference/json-output.md) for tool consumers.
- Composable in the [quality-gates](overview.md) flow — CI step or Claude Code `Stop` hook, alongside
  `check` and `score`.

## Also planned: trend snapshots <span title="Preview">🧪</span>

A companion feature will snapshot scorecard metrics over time (via DuckDB/Parquet) so you can chart
code-health trends and diff a branch against a historical baseline — moving governance from
"per-change gate" to "longitudinal evidence". Design is not finalized.

## See also

- [ctx check](check.md) · [ctx score](score.md) — the shipped gates today
- [Quality gates](overview.md) — the governance suite overview
