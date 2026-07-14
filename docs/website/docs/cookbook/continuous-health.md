---
id: continuous-health
title: Build a codebase health timeline in CI
sidebar_position: 2
---

# Build a codebase health timeline in CI

A pull-request gate answers whether one change introduced a known problem. A codebase health
timeline answers a different question: **are complexity, coupling, duplication, hotspots, and
architecture violations moving in a healthy direction across many changes?**

This recipe captures one immutable metrics partition for every default-branch commit, keeps the
history outside the product branch, and queries it with DuckDB through `ctx sql`.

## When to use this recipe

Use it when you want to:

- distinguish repository growth from degrading design;
- find metrics that worsen repeatedly across otherwise acceptable pull requests;
- identify complex code that is also frequently changed;
- compare releases or engineering periods;
- give an agent evidence for investigating a trend.

Do not use a historical trend as an automatic verdict. Trends identify questions; engineers decide
whether the underlying change is accidental debt, intentional design, or a measurement artifact.

## Prerequisites

- A git repository with enough history to compare.
- A ctx build with the default `duckdb` feature. `ctx snapshot` and `ctx sql` are unavailable in
  builds without it.
- An up-to-date index: `ctx index`.
- Optional `.ctx/rules.toml` policy. Without active rules, a violation count of zero says nothing
  about architectural conformance.

## 1. Inspect the current state

Start with point-in-time evidence before building a history:

```bash
ctx index
ctx hotspots --since "90 days ago" --limit 20
ctx duplicates --threshold 0.95 --min-tokens 50
```

`hotspots` combines normalized churn and complexity. `duplicates` shows candidate pairs for human
inspection. If `.ctx/rules.toml` exists, run `ctx check --list` to verify which architecture policy
gives meaning to violation counts; if it does not exist, record that policy coverage is absent.

## 2. Capture one snapshot

```bash
ctx snapshot --json
```

ctx writes an atomic partition beneath `.ctx/snapshots/sha=<commit>/` containing:

- `files.parquet` — file complexity, churn, and violation counts;
- `symbols.parquet` — symbol complexity, fan-in, and fan-out;
- `dup_pairs.parquet` — structural near-duplicate pairs;
- `meta.parquet` — commit and capture metadata.

Capturing the same commit again is a no-op unless you pass `--force`. A dirty-tree snapshot is
labelled with `HEAD` but reflects working-tree content, so CI should capture from a clean checkout.

## 3. Append snapshots from default-branch CI

Keep generated metric history out of the product branch. The following shape uses an orphan
`ctx-snapshots` branch and serializes writers so concurrent merges cannot overwrite each other:

```yaml
name: snapshot

on:
  push:
    branches: [main]

permissions:
  contents: write

concurrency:
  group: ctx-snapshots
  cancel-in-progress: false

jobs:
  capture:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@08c6903cd8c0fde910a37f88322edcfb5dd907a8 # v5.0.0
        with:
          fetch-depth: 0
      - uses: dtolnay/rust-toolchain@fa04a1451ff1842e2626ccb99004d0195b455a88
        with:
          toolchain: "1.91"
      - run: cargo build --locked --release
      - run: |
          target/release/ctx index
          target/release/ctx snapshot --json
      - name: Append the partition
        run: |
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"

          if git ls-remote --exit-code origin refs/heads/ctx-snapshots >/dev/null 2>&1; then
            git fetch origin +refs/heads/ctx-snapshots:refs/remotes/origin/ctx-snapshots
            git worktree add -B ctx-snapshots ../snapshots-branch origin/ctx-snapshots
          else
            git worktree add --orphan -b ctx-snapshots ../snapshots-branch
          fi

          mkdir -p ../snapshots-branch/snapshots
          cp -R .ctx/snapshots/. ../snapshots-branch/snapshots/
          cd ../snapshots-branch
          git add -A
          git diff --cached --quiet && exit 0
          git commit -m "snapshot: ${GITHUB_SHA}"
          git push origin ctx-snapshots
```

Keep third-party actions pinned to reviewed commit SHAs and restrict the workflow's write
permissions according to your repository policy. ctx uses this pattern in its own
[`snapshot.yml`](https://github.com/agentis-tools/ctx/blob/main/.github/workflows/snapshot.yml).

## 4. Query the history

Check out the data branch beside the repository:

```bash
git fetch origin ctx-snapshots
git worktree add ../ctx-snapshots ctx-snapshots
```

First confirm the coverage and writer versions:

```bash
ctx sql --snapshots=../ctx-snapshots/snapshots "
SELECT count(*) AS snapshots,
       min(committed_at) AS first_commit,
       max(committed_at) AS latest_commit,
       min(ctx_version) AS oldest_ctx,
       max(ctx_version) AS newest_ctx
FROM snap.meta;"
```

Then build a normalized series. Absolute complexity and duplicate counts naturally grow when a
repository gains code, so include denominators and averages:

```bash
ctx sql --snapshots=../ctx-snapshots/snapshots "
WITH files AS (
  SELECT commit_sha,
         count(*) AS files,
         sum(total_complexity) AS total_complexity,
         sum(violation_count) AS violations
  FROM snap.files
  GROUP BY commit_sha
), symbols AS (
  SELECT commit_sha,
         count(*) AS symbols,
         avg(complexity) AS avg_complexity,
         avg(fan_out) AS avg_fan_out
  FROM snap.symbols
  GROUP BY commit_sha
), duplicates AS (
  SELECT commit_sha, count(*) AS duplicate_pairs
  FROM snap.dup_pairs
  GROUP BY commit_sha
)
SELECT m.committed_at,
       left(m.commit_sha, 7) AS commit,
       f.files,
       s.symbols,
       f.total_complexity,
       round(s.avg_complexity, 2) AS avg_complexity,
       round(s.avg_fan_out, 2) AS avg_fan_out,
       coalesce(d.duplicate_pairs, 0) AS duplicate_pairs,
       round(1000.0 * coalesce(d.duplicate_pairs, 0) / s.symbols, 1)
         AS duplicate_pairs_per_1000_symbols,
       f.violations
FROM snap.meta m
JOIN files f USING (commit_sha)
JOIN symbols s USING (commit_sha)
LEFT JOIN duplicates d USING (commit_sha)
ORDER BY m.committed_at;"
```

## 5. Read the evidence

Use several signals together:

| Observation | Interpretation to investigate |
|---|---|
| Total complexity rises while average complexity is stable | Repository growth, not necessarily degradation |
| Duplicate count rises but pairs per 1,000 symbols falls | Duplication grew more slowly than the codebase |
| Complexity and churn rise in the same file | Possible chronic hotspot |
| Fan-out rises repeatedly around one symbol | Growing orchestration role or spreading coupling |
| Violation count stays at zero | Confirm that active rules cover the architecture before celebrating |
| One metric jumps at one commit and then stabilizes | Inspect that change before calling it a trend |

Treat a trend as stronger evidence when it persists, appears in related metrics, and concentrates in
code that changes frequently.

## 6. Investigate rather than auto-refactor

After locating a suspicious commit or file:

```bash
git show <commit> --stat
ctx map --focus <path> --budget 3000
ctx query find <symbol>
ctx query callers <symbol>
ctx query deps <symbol>
ctx query impact <symbol>
ctx source <symbol>
```

Classify the result before recommending work:

- **Intentional complexity** — the function clearly owns an inherently complex operation.
- **Accidental complexity** — responsibilities or dependencies accumulated without a coherent role.
- **Intentional similarity** — implementations are parallel but should evolve independently.
- **Missed reuse** — duplicated behavior should share an established implementation.
- **Insufficient evidence** — the metric cannot support a safe recommendation yet.

Do not split a parser, state machine, transaction boundary, or orchestration function merely to
lower a score. Do not abstract platform adapters or tests merely because their token shapes match.

## What this found in ctx itself

ctx's `ctx-snapshots` branch contained 98 partitions when this recipe was written, spanning
2025-11-25 through 2026-07-13 and writer versions 0.3.1 through 0.3.5.

Comparing release snapshots with the latest recorded 0.3.5 state produced:

| State | Symbols | Total complexity | Avg. complexity | Avg. fan-out | Duplicate pairs | Pairs / 1,000 symbols |
|---|---:|---:|---:|---:|---:|---:|
| v0.3.1 | 1,648 | 22,944 | 13.92 | 6.26 | 137 | 83.1 |
| v0.3.2 | 1,922 | 26,720 | 13.90 | 6.24 | 136 | 70.8 |
| v0.3.3 | 2,026 | 27,897 | 13.77 | 6.18 | 137 | 67.6 |
| v0.3.4 | 2,048 | 28,127 | 13.73 | 6.16 | 138 | 67.4 |
| latest 0.3.5 snapshot (`62ba248`) | 2,161 | 29,733 | 13.76 | 6.16 | 139 | 64.3 |

The simplistic conclusion would be “complexity and duplication increased.” The normalized evidence
says something different: the indexed symbol count grew by about 31%, average complexity and
fan-out slightly declined, and duplicate pairs grew by only two. Duplicate pairs per 1,000 symbols
fell by about 23%.

The current hotspot view still identified `src/db/schema.rs` as the strongest pressure point because
it combined high complexity with 11 commits in the 90-day window. That is a useful investigation
target, not an instruction to split the file blindly.

The duplicate list also contained parallel enum `as_str` methods, analogous test helpers, and
similar read-only database query methods. Some may be reusable; others are intentionally explicit.
Each pair needs source and ownership context before action.

Finally, every snapshot reported zero architecture violations because the repository had no
committed active architecture rules; the current local starter file is also empty. That is a
coverage gap, not proof of perfect architecture. This is why a health report must record the policy
and tooling configuration that gave each metric meaning.

## Give the workflow to an agent

Ask an agent to report evidence and uncertainty, not just recommendations:

```text
Analyze the ctx snapshot history between v0.3.1 and the latest commit. Normalize totals by
repository size, identify persistent changes rather than one-commit spikes, and inspect the files
or symbols behind the strongest signal. Classify each finding as intentional, likely accidental,
or insufficient evidence. Do not recommend a refactor solely because a metric is high.
```

The published ctx agent skill encodes the same safeguards so compatible agents can route trend
questions to `ctx sql --snapshots` and investigate the responsible code before proposing changes.

## Next steps

- Add a scheduled job or release workflow that turns the series into a health report.
- Introduce architecture rules gradually, then annotate when policy coverage changes.
- Compare release-to-release trends rather than gating every absolute metric.
- Follow up with [`ctx score`](../commands/score.md) for point-in-time change evaluation and
  [`ctx snapshot`](../commands/snapshot.md) for the complete snapshot schema and backfill behavior.
