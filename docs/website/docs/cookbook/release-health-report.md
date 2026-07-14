---
id: release-health-report
title: Produce a release health report that supports decisions
sidebar_position: 8
---

# Produce a release health report that supports decisions

A release health report should explain how the codebase changed, what deserves attention, and what
the available measurements cannot establish. It should not collapse growth, complexity, coupling,
duplication, hotspots, and architecture policy into one unexplained “health score.”

This recipe turns snapshot history and focused investigations into a compact engineering decision
artifact for maintainers, reviewers, and future agents.

:::note Worked-example provenance
The report compares immutable ctx tags v0.3.4 and v0.3.5 as measured on 2026-07-14. The values are
an illustration; later releases and regenerated snapshot histories will differ.
:::

## Quickest version

```bash
git rev-parse <older-tag>^{} <newer-tag>^{}
ctx sql --snapshots=<snapshot-dir> \
  "SELECT commit_sha, committed_at FROM snap.meta ORDER BY committed_at;"
ctx map --focus <materially-changed-path> --budget 2000
```

Compare immutable points with matching provenance, normalize repository-wide totals, and assign an
owner only after inspecting the files and symbols behind each material signal.

## The report contract

Every report should contain:

1. the exact commits or releases compared;
2. snapshot, ctx, and policy coverage;
3. absolute and normalized metric deltas;
4. the files or symbols behind material changes;
5. classifications and uncertainty;
6. actions, monitoring decisions, or accepted intentional cases.

Keep observations separate from interpretations and recommendations. “Total complexity rose 5.7%”
is an observation. “Design quality declined” is an interpretation that needs more evidence.

## 1. Resolve immutable release points

Use tag commit SHAs, not moving branches:

```bash
OLD="$(git rev-list -n 1 v1.4.0)"
NEW="$(git rev-list -n 1 v1.5.0)"
printf 'old=%s\nnew=%s\n' "$OLD" "$NEW"
```

Confirm that both commits have snapshots and inspect measurement provenance:

```bash
ctx sql --snapshots=../ctx-snapshots/snapshots "
  SELECT left(commit_sha, 7) AS commit,
         committed_at,
         captured_at,
         ctx_version,
         snapshot_schema_version,
         capture_mode
  FROM snap.meta
  WHERE commit_sha IN ('$OLD', '$NEW')
  ORDER BY committed_at;"
```

Stop if either release is absent. Do not silently substitute the nearest snapshot. Different ctx
versions or schema versions do not automatically invalidate a comparison, but parser, detector, or
metric changes must be checked in the changelog and noted in the report.

## 2. Build the release comparison table

```bash
ctx sql --snapshots=../ctx-snapshots/snapshots "
  WITH selected(commit_sha, release) AS (
    VALUES ('$OLD', 'v1.4.0'), ('$NEW', 'v1.5.0')
  ), files AS (
    SELECT commit_sha,
           count(*) AS files,
           sum(total_complexity) AS total_complexity,
           max(max_complexity) AS max_complexity,
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
  ), pairs AS (
    SELECT commit_sha, count(*) AS duplicate_pairs
    FROM snap.dup_pairs
    GROUP BY commit_sha
  )
  SELECT x.release,
         m.committed_at,
         f.files,
         s.symbols,
         f.total_complexity,
         round(s.avg_complexity, 2) AS avg_complexity,
         f.max_complexity,
         round(s.avg_fan_out, 2) AS avg_fan_out,
         coalesce(p.duplicate_pairs, 0) AS duplicate_pairs,
         round(1000.0 * coalesce(p.duplicate_pairs, 0) / s.symbols, 1)
           AS pairs_per_1000_symbols,
         f.violations,
         m.ctx_version
  FROM selected x
  JOIN snap.meta m USING (commit_sha)
  JOIN files f USING (commit_sha)
  JOIN symbols s USING (commit_sha)
  LEFT JOIN pairs p USING (commit_sha)
  ORDER BY m.committed_at;"
```

Always show size alongside totals. Prefer averages, rates, and per-symbol or per-file denominators
when the repository grew materially.

## 3. Triage changes instead of scoring them blindly

For each material delta, choose the next investigation:

| Observation | Follow-up |
|---|---|
| Total complexity tracks symbol growth | compare average and maximum complexity |
| Maximum complexity jumps | inspect the responsible symbol and fan-in/fan-out shape |
| Average fan-out rises | locate symbols or files driving coupling growth |
| Duplicate density rises | identify new and persistent pair families |
| Hotspot mass concentrates | inspect churn causes and ownership boundaries |
| Violations change | compare `.ctx/rules.toml` and policy coverage first |
| A metric jumps with a ctx upgrade | review parser, schema, and detector changes |

Use the other cookbook workflows for evidence:

- [intentional complexity](intentional-complexity) for maximum-complexity outliers;
- [chronic hotspots](chronic-hotspots) for churn combined with structural pressure;
- [duplication trajectories](duplication-trajectories) for pair density and persistence;
- [architecture drift](architecture-drift) for coupling and policy movement.

## 4. Find the responsible files and symbols

Rank file-level changes at the two releases:

```bash
ctx sql --snapshots=../ctx-snapshots/snapshots "
  WITH selected(commit_sha, release) AS (
    VALUES ('$OLD', 'old'), ('$NEW', 'new')
  ), ranked AS (
    SELECT x.release,
           f.path,
           f.symbol_count,
           f.total_complexity,
           f.max_complexity,
           f.churn_commits,
           row_number() OVER (
             PARTITION BY x.release
             ORDER BY f.churn_commits * f.total_complexity DESC
           ) AS rank
    FROM selected x
    JOIN snap.files f USING (commit_sha)
  )
  SELECT *
  FROM ranked
  WHERE rank <= 15
  ORDER BY release, rank;"
```

Then inspect the code rather than extrapolating from the table:

```bash
git diff --stat "$OLD..$NEW"
git log --oneline "$OLD..$NEW" -- <path>
ctx map --focus <path> --budget 4000
ctx explain <symbol> --file <path> --json
ctx query callers <symbol>
ctx query deps <symbol>
```

The current index represents the checked-out tree, not both historical releases. For an exact old
graph or source review, use a temporary worktree at the old commit and build its index there.

## 5. Report policy and measurement coverage

Architecture figures require context:

```bash
git diff "$OLD..$NEW" -- .ctx/rules.toml .ctx/config.toml
git log --format='%h %ad %s' --date=short "$OLD..$NEW" -- .ctx/rules.toml
```

State explicitly:

- whether an active rules file existed at both releases;
- which layers and contracts it covered;
- whether exclusions or limits changed;
- which languages and generated paths were indexed;
- whether ctx, schema, parser, or duplication-detector versions changed.

Zero violations with no active rules is a policy-coverage gap, not a clean architecture result.

## 6. Write a decision-oriented report

Use a consistent Markdown shape:

```markdown
# Codebase health: v1.4.0 → v1.5.0

## Scope and provenance
- Commits: `<old>` → `<new>`
- Snapshots: present / missing
- ctx versions: ...
- Architecture-policy coverage: ...

## Executive summary
Three to five sentences describing growth, normalized direction, and the strongest investigated
signal. Do not label the whole release healthy or unhealthy without defining that policy.

## Evidence
| Signal | Old | New | Normalized change | Interpretation | Confidence |
|---|---:|---:|---:|---|---|

## Investigated findings
### <file or symbol>
- Observation:
- Source and history reviewed:
- Classification: intentional / likely accidental / coverage artifact / insufficient evidence
- Decision: act / monitor / accept with rationale

## Policy and measurement changes
Rules, exclusions, ctx versions, schema versions, and known blind spots.

## Follow-ups
Owned, concrete actions with a trigger or due point. Avoid unowned “reduce complexity” goals.
```

Include enough query output to reproduce the conclusion, but keep large JSON or CSV results as CI
artifacts rather than pasting them into the narrative.

## 7. Automate collection, preserve judgment

Run the report on release preparation or after a release tag, and upload:

- the selected commit metadata;
- summary and ranked-file tables as CSV or JSON;
- focused ctx JSON documents;
- the final Markdown report.

Pin the ctx binary and CI actions, use immutable release SHAs, validate JSON before rendering, and
retain the artifacts long enough for later comparisons. Automation should collect and route
evidence. A maintainer or explicitly instructed agent should classify ambiguous complexity,
duplication, and architecture findings.

Do not make release publication depend on every rising metric. Block only explicit release policy,
contract, or operational failures. Track other findings as review items with owners.

## What worked, and what did not in the v0.3.4 → v0.3.5 report

The exact release snapshots produce:

| Signal | v0.3.4 | v0.3.5 | Reading |
|---|---:|---:|---|
| Files | 102 | 110 | repository grew 7.8% |
| Symbols | 2,048 | 2,161 | indexed surface grew 5.5% |
| Total complexity | 28,127 | 29,733 | rose 5.7%, broadly with symbol growth |
| Average complexity | 13.73 | 13.76 | effectively stable |
| Maximum complexity | 208 | 226 | investigate the responsible symbol |
| Average fan-out | 6.16 | 6.16 | stable |
| Duplicate pairs | 138 | 139 | one additional pair |
| Pairs per 1,000 symbols | 67.4 | 64.3 | normalized duplication fell 4.6% |
| Architecture violations | 0 | 0 | no committed active rules; not evidence of conformance |

The maximum-complexity investigation leads to `Metrics::get`, a roughly twelve-line lookup with
fan-in 212. Its score is driven by reuse, not a newly sprawling algorithm. The hotspot investigation
still identifies `src/db/schema.rs` as an active pressure point, while its complexity per symbol is
below an earlier historical peak. Duplication density declined despite one additional absolute
pair.

The defensible conclusion is: ctx grew materially between the releases while average graph
complexity and coupling stayed stable and normalized duplication improved. There is ongoing
maintenance pressure in the database layer, and architecture-policy coverage remains absent. The
report supports monitoring and introducing reviewed architecture rules; it does not support a
blanket claim that complexity regressed.

## Give the workflow to an agent

```text
Produce a release health report between two immutable tags. Verify both snapshot partitions and
record ctx, schema, and policy coverage before comparing metrics. Show repository size beside
totals, normalize complexity and duplication, and investigate the files and symbols behind material
changes using source, graph, and git history. Separate observations, interpretations, confidence,
and actions. Classify intentional design, likely accidental debt, measurement or policy changes,
and insufficient evidence. Do not turn every rising metric into a release blocker or collapse the
report into one unexplained health score.
```

## Next steps

- Keep the underlying [continuous snapshot history](continuous-health) current.
- Use the [pull-request governance recipe](pr-governance) to address explicit contracts before
  release preparation.
- Publish the Markdown summary with machine-readable artifacts so future reports and agents can
  reproduce the analysis.
