---
id: duplication-trajectories
title: Track duplication trajectories without forcing abstractions
sidebar_position: 7
---

# Track duplication trajectories without forcing abstractions

Similar code can mean missed reuse, intentionally parallel implementations, independent tests, or
the natural shape of a protocol. The useful question is not simply “how many duplicate pairs
exist?” but **which pairs are new, persistent, growing relative to the codebase, and costly to keep
in sync?**

This recipe separates current detection, branch review, and longitudinal analysis before deciding
whether two implementations should share an abstraction.

## Know what the detector sees

ctx compares normalized five-token shingles:

- identifiers become `ID`;
- string and number literals become `LIT`;
- comments are removed;
- candidates use MinHash/LSH and are verified with exact Jaccard similarity.

Renaming variables or changing constants therefore does not make structurally parallel code look
different. Conversely, the detector does not prove that two functions have the same semantics,
ownership, failure behavior, or reasons to change.

## 1. Inspect current pairs

Start conservatively, then widen if needed:

```bash
ctx index
ctx duplicates --threshold 0.95 --min-tokens 50
ctx duplicates --threshold 0.85 --min-tokens 100 --json > duplicates.json
```

Read both endpoints and their context:

```bash
jq '.data.pairs[] | {
  a, b, similarity, token_count_a, token_count_b
}' duplicates.json

ctx source <first-symbol> --file <first-file>
ctx source <second-symbol> --file <second-file>
```

Short helpers, builders, enum conversions, CRUD adapters, and test setup commonly share token
shapes. Raising `--min-tokens` removes many small idioms; raising `--threshold` asks for closer
structure. Neither setting turns similarity into a design verdict.

## 2. Ask the correct branch question

These two commands have different semantics:

```bash
ctx duplicates --against <base>
ctx score --against <base> --json
```

| Result | Meaning |
|---|---|
| `duplicates --against` | reports current pairs where at least one endpoint is in a changed file |
| `score.data.metrics.new_duplication` | counts verified pairs that were absent at the base |

The first is useful for reviewing changed code in context, including old similarity. The second is
the stronger signal for “did this branch introduce a pair?” Use `ctx score` for delta governance
and the full duplicates output to understand the actual implementations.

Even a genuinely new pair may be intentional. Keep it as a review signal until the repository has
a documented policy and exception path.

## 3. Normalize the trajectory

Absolute pair counts tend to rise as a repository gains symbols. Query both the numerator and its
denominator:

```bash
ctx sql --snapshots=../ctx-snapshots/snapshots "
  WITH symbols AS (
    SELECT commit_sha, count(*) AS symbols
    FROM snap.symbols
    GROUP BY commit_sha
  ), pairs AS (
    SELECT commit_sha, count(*) AS duplicate_pairs
    FROM snap.dup_pairs
    GROUP BY commit_sha
  )
  SELECT m.committed_at,
         left(m.commit_sha, 7) AS commit,
         s.symbols,
         coalesce(p.duplicate_pairs, 0) AS duplicate_pairs,
         round(1000.0 * coalesce(p.duplicate_pairs, 0) / s.symbols, 1)
           AS pairs_per_1000_symbols
  FROM snap.meta m
  JOIN symbols s USING (commit_sha)
  LEFT JOIN pairs p USING (commit_sha)
  ORDER BY m.committed_at;"
```

Interpretation examples:

| Trajectory | Question to investigate |
|---|---|
| Pair count rises while density falls | Is similarity growing more slowly than the codebase? |
| Count and density rise across releases | Which pair families account for the growth? |
| One commit creates a jump | Did a generator, adapter family, or copied feature land? |
| Density falls after extraction | Did change coupling improve, or was code merely reshaped? |
| A stable pair persists for months | Is it intentional and independently owned? |

## 4. Find persistent pair families

Use distinct commits when measuring persistence:

```bash
ctx sql --snapshots=../ctx-snapshots/snapshots "
  SELECT file_a,
         symbol_a,
         file_b,
         symbol_b,
         count(DISTINCT commit_sha) AS commits_seen,
         count(*) AS pair_occurrences,
         min(committed_at) AS first_seen,
         max(committed_at) AS last_seen,
         round(avg(similarity), 3) AS avg_similarity
  FROM snap.dup_pairs
  GROUP BY file_a, symbol_a, file_b, symbol_b
  ORDER BY commits_seen DESC, pair_occurrences DESC
  LIMIT 100;"
```

Snapshot pair rows contain file and symbol names but not qualified names or line ranges. Several
same-named methods in one file can therefore produce multiple indistinguishable rows for a commit.
`count(*)` measures occurrences, not snapshots. `count(DISTINCT commit_sha)` is the safe persistence
measure, and current source inspection is still required to identify the exact endpoints.

Also account for detector and parser-version changes. Compare `ctx_version` from `snap.meta` before
calling a discontinuity a code trend.

## 5. Test whether reuse would reduce change coupling

For each important pair, inspect history and dependencies:

```bash
git log --date=short --format='%h %ad %s' -- <first-file> <second-file>
ctx query callers <first-symbol>
ctx query callers <second-symbol>
ctx query deps <first-symbol>
ctx query deps <second-symbol>
```

Ask:

1. Do the implementations represent the same domain concept?
2. Must a bug fix or behavior change usually be applied to both?
3. Do they have the same callers, lifecycle, error behavior, and ownership?
4. Would a shared abstraction have a stable name and contract?
5. Would reuse couple components that are supposed to evolve independently?
6. Is explicit repetition making tests, adapters, or protocol cases easier to understand?

Similarity plus synchronized change history is much stronger evidence for reuse than similarity
alone.

## 6. Classify before acting

- **Missed reuse:** the same behavior and ownership are copied; fixes must stay synchronized.
- **Intentional parallel implementation:** backends, platforms, or protocol cases share a shape but
  must evolve independently.
- **Independent test clarity:** explicit fixtures or cases avoid a helper that would obscure intent.
- **Small idiom:** conversion, builder, or glue code is too small to justify an abstraction.
- **Generated or schema-shaped repetition:** the source structure is dictated elsewhere.
- **Measurement collision:** normalization or same-named snapshot rows exaggerate equivalence.
- **Insufficient evidence:** retain the finding without prescribing work.

If reuse is justified, extract the smallest stable behavior and re-run:

```bash
ctx score --against <base>
ctx duplicates --against <base>
ctx query impact <shared-symbol>
```

Judge the result by reduced synchronization cost and a clearer contract, not only by the pair count.

## What this found in ctx itself

Across the release snapshots, ctx's duplicate-pair count moved only from 137 at v0.3.1 to 139 in
the latest recorded 0.3.5 state. Indexed symbols grew from 1,648 to 2,161, so duplicate pairs per
1,000 symbols fell from 83.1 to 64.3—about 23%. “Duplicates increased” is true in absolute terms but
misleading as a health conclusion.

Current high-similarity results include several distinct categories:

- enum `as_str` and `from_str` methods share exhaustive match structure but encode separate type
  contracts;
- TypeScript, Solidity, and formatter tests deliberately repeat scenario setup and assertions;
- `Analytics::call_graph` and `Analytics::impact_analysis` use closely parallel query shapes for
  different graph operations;
- SQLite-facing and DuckDB-facing metric queries mirror one another across backend boundaries;
- repeated `symbol_ref` conversion helpers may be a smaller candidate for shared reuse.

None of those classifications should be accepted from names alone. They are hypotheses for source
and change-history review. The trajectory says duplication density is declining; the pair list says
there are still local opportunities worth examining.

## Give the workflow to an agent

```text
Analyze ctx duplication at three levels: current pairs, pairs introduced by this branch, and the
longitudinal density of pairs per 1,000 symbols. Distinguish duplicates touching changed files from
pairs absent at the base. Group persistent snapshot pairs using distinct commits, account for
same-named identity collisions and ctx-version changes, then inspect source, ownership, callers,
dependencies, and synchronized change history. Classify missed reuse, intentional parallel
implementations, independent test clarity, small idioms, generated repetition, measurement
collisions, and insufficient evidence. Recommend an abstraction only when it creates a stable
contract and reduces future synchronization cost.
```

## Next steps

- Use the [pull-request governance recipe](pr-governance.md) to decide whether new similarity is a
  report, review, or blocking signal.
- Use [ctx duplicates](../commands/duplicates.md) for threshold, token, and exit-code details.
- Feed normalized duplication evidence into the release-health report rather than reporting raw
  pair counts alone.
