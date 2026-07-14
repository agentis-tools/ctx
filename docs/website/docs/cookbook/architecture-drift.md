---
id: architecture-drift
title: Detect architectural drift before it becomes a rewrite
sidebar_position: 4
---

# Detect architectural drift before it becomes a rewrite

Architectural drift is the gradual loss of intended boundaries: a domain package begins calling
infrastructure directly, a supposedly internal module gains dependents, or one coordinator becomes
the route through which everything passes. Each individual change can look reasonable while the
long-term direction becomes expensive.

This recipe combines current dependency evidence, explicit rules, pull-request scoping, and
historical pressure signals. It does not assume that increasing coupling is automatically wrong.

## Quickest version

```bash
ctx index
ctx check --list
BASE="$(git merge-base HEAD origin/main)"
ctx check --against "$BASE"
```

Review each reported edge in source. Use snapshot history only after the point-in-time boundary is
understood; a growing metric alone is not proof of drift.

## What counts as evidence of drift?

Look for combinations rather than one high value:

| Signal | Question it raises |
|---|---|
| Repeatedly rising fan-out | Is a symbol accumulating unrelated responsibilities? |
| Repeatedly rising fan-in | Is an internal implementation becoming a de facto public boundary? |
| New dependency crossing an intended layer | Is the documented architecture still true? |
| New callers of a frozen legacy module | Is migration moving in the wrong direction? |
| Violations concentrated in frequently changed files | Is the boundary causing chronic friction? |
| A policy change followed by a metric jump | Did the code drift, or did measurement coverage improve? |

A stable, highly connected symbol may be an intentional facade. A growing fan-out may reflect a
new orchestration role. The trend tells you where to inspect; the source and ownership model tell
you whether it is drift.

## 1. Describe the intended architecture

Start with a token-budgeted structural view and the current dependency graph:

```bash
ctx index
ctx map --budget 6000
ctx sql "
  SELECT source_file, target_file, count(*) AS edges
  FROM v1.edges
  WHERE target_file IS NOT NULL
    AND source_file <> target_file
  GROUP BY source_file, target_file
  ORDER BY edges DESC, source_file, target_file
  LIMIT 100;"
```

Write down boundaries in engineering language before translating them into rules. Examples:

- domain code does not depend on database adapters;
- only the application layer calls infrastructure;
- new callers must not attach to the legacy subsystem;
- a public facade is the supported entry point for a package.

If the team cannot state the intended boundary, ctx cannot infer whether a dependency is allowed.

## 2. Encode only reviewed contracts

Create `.ctx/rules.toml` gradually:

```toml
version = 1

[layers]
domain = ["src/domain/**"]
application = ["src/application/**"]
infrastructure = ["src/infrastructure/**"]
legacy = ["src/legacy/**"]

[[rules.forbidden]]
from = "domain"
to = "infrastructure"
reason = "Domain code must remain persistence-agnostic"

[[rules.no_new_dependents]]
paths = ["src/legacy/**"]
reason = "The legacy subsystem is being retired"
```

Validate both syntax and coverage:

```bash
ctx check --list
ctx check
```

`--list` matters because a valid glob that matches no indexed files provides no protection. Layers
must not overlap, and unresolved external or dynamically generated relationships cannot be checked
by the static graph.

Do not begin by converting every current metric into a limit rule. A rule should express an
architectural contract, not a desire to make a dashboard number smaller.

## 3. Stop new drift at the pull request

Scope the policy to changed files:

```bash
BASE="$(git merge-base HEAD origin/main)"
ctx check --against "$BASE"
ctx score --against "$BASE" --json > score.json
```

This lets a legacy repository adopt boundaries without requiring one pull request to repair every
existing violation. Remember that touching a file can bring an existing relationship involving
that file into scope. Review the reported edge before claiming the branch created it.

Use [`no_new_dependents`](../commands/check) when the goal is directional: existing callers may
remain temporarily, but migration work must not add another one.

## 4. Capture the long-term pressure

Follow the [continuous health recipe](continuous-health) to retain one snapshot per
default-branch commit. Then inspect symbols whose coupling remains high across multiple captures:

```bash
ctx sql --snapshots=../ctx-snapshots/snapshots "
  WITH ranked AS (
    SELECT committed_at,
           commit_sha,
           file,
           coalesce(qualified_name, name) AS symbol,
           fan_in,
           fan_out,
           row_number() OVER (
             PARTITION BY commit_sha
             ORDER BY fan_out DESC, fan_in DESC
           ) AS rank
    FROM snap.symbols
  )
  SELECT committed_at,
         left(commit_sha, 7) AS commit,
         file,
         symbol,
         fan_in,
         fan_out
  FROM ranked
  WHERE rank <= 10
  ORDER BY committed_at, rank;"
```

Track where architecture findings concentrate:

```bash
ctx sql --snapshots=../ctx-snapshots/snapshots "
  SELECT committed_at,
         left(commit_sha, 7) AS commit,
         path,
         violation_count,
         churn_commits,
         total_complexity
  FROM snap.files
  WHERE violation_count > 0
  ORDER BY committed_at, violation_count DESC;"
```

The snapshot schema records symbol fan-in/fan-out and per-file violation counts, but not the full
historical edge list. Use these values to locate a suspect period. To reconstruct the precise graph,
check out or inspect the responsible commit and rebuild its index.

## 5. Separate code movement from policy movement

A violation trend is only comparable while its policy coverage is comparable. Review changes to
the rules alongside the snapshot series:

```bash
git log --follow --date=short --format='%h %ad %s' -- .ctx/rules.toml
git diff <older-commit>..<newer-commit> -- .ctx/rules.toml
```

Interpret transitions explicitly:

- violations rise with unchanged rules: investigate code movement;
- violations rise when a new layer or rule is introduced: coverage improved;
- violations fall after an exclusion is added: policy weakened or clarified, not necessarily code
  improvement;
- fan-out rises while violations remain flat: the change may be inside an allowed layer but still
  deserve design review.

For durable reports, record the ctx version, snapshot schema version, and rules-file commit or hash
with the analysis.

## 6. Investigate the boundary

Once a symbol, file, or period stands out:

```bash
git show <commit> --stat
ctx map --focus <path> --budget 4000
ctx query callers <symbol>
ctx query deps <symbol>
ctx query impact <symbol>
ctx source <symbol>
```

Classify the finding:

- **Intended boundary:** a facade, dispatcher, or composition root is expected to connect many
  components.
- **Boundary erosion:** business logic is reaching around an intended interface or a module is
  acquiring unrelated consumers.
- **Migration pressure:** temporary coupling is understood and has an owner and removal plan.
- **Coverage artifact:** a rule, parser, language, or index change altered what ctx could observe.
- **Insufficient evidence:** dynamic behavior or unresolved edges prevent a confident conclusion.

The response differs by category. Boundary erosion may require code movement; an intended boundary
may need documentation or an explicit exclusion; insufficient evidence should not trigger an
automatic rewrite.

## What worked, and what did not in ctx itself

ctx's historical snapshots currently show zero architecture violations, but the repository has no
committed active rules. That series therefore measures policy coverage as zero, not architectural
perfection. The same history does expose coupling pressure through symbol fan-in and fan-out, while
the current hotspot view identifies `src/db/schema.rs` as a high-complexity, frequently changed
area. Together those facts justify a boundary investigation; neither proves that the database
schema module should be split.

The first meaningful ctx architecture baseline should therefore begin with reviewed layer and
dependency assertions. Historical violation comparisons should start—or be clearly annotated—when
that policy becomes active.

## Give the workflow to an agent

```text
Analyze architectural drift across the available ctx snapshots. First verify the active rules and
their history. Look for persistent changes in fan-in, fan-out, violations, and churn, then inspect
the exact symbols and dependencies at the responsible commits. Distinguish code drift from changes
in policy or measurement coverage. Classify intentional boundaries, erosion, migration pressure,
coverage artifacts, and insufficient evidence separately. Do not propose a rewrite from a high
coupling metric alone.
```

## Next steps

- Use the [pull-request governance recipe](pr-governance) to enforce reviewed contracts on new
  changes.
- Use [ctx check](../commands/check) for the complete rule grammar and static-analysis caveats.
- Investigate files that combine boundary pressure with churn using the forthcoming chronic
  hotspots recipe.
