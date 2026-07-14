---
id: cookbook
title: Cookbook
sidebar_position: 1
---

# Cookbook

The command reference explains what ctx can measure. This cookbook starts with an engineering
question and builds a repeatable workflow around the answer.

Recipes combine ctx commands, CI configuration, machine-readable queries, and interpretation. They
also call out where judgment is required: a high metric is a signal to investigate, not proof that
the code is wrong.

## Everyday agent-assisted engineering

- **[Understand an unfamiliar codebase before editing it](unfamiliar-codebase.md)** — refresh the
  index, distinguish central symbols from real entry points, verify a focused execution path, and
  record static-analysis uncertainty before choosing a working set.
- **[Build the smallest useful context for a task](smallest-useful-context.md)** — rank candidates,
  audit writers, consumers, tests, and contracts, use snippets for investigation, and count the
  explicit implementation set before packaging it.
- **[Find existing implementations before writing new code](find-existing-implementations.md)** —
  vary semantic, keyword, and signature-like queries; inspect candidates and their callers; verify
  behavioral tests; and choose the correct reuse boundary.
- **[Trace the blast radius before editing a symbol](blast-radius.md)** — start with direct graph
  evidence, verify transitive branches from source, search persisted and public contracts, and turn
  the classified impact into a validation plan.
- **[Implement a feature with an evidence-backed working set](evidence-backed-implementation.md)** —
  baseline the owning boundary, exercise the compiled public surface, refresh contracts and the
  index, and interpret the actual diff and metric movement before declaring completion.
- **[Debug a failing test with a focused evidence loop](debug-failing-test.md)** — reproduce the
  exact symptom, trace the test into production code, compare neighboring behavior, and prove one
  causal correction at widening validation scopes.
- **[Review a large branch without reading every file equally](review-large-branch.md)** — inventory
  the whole change, split it into review streams, route attention with scoped metrics, reject false
  graph expansion, and verify policy claims where intent meets enforcement and CI wiring.

## How to use the recipes

Each recipe follows the same pattern:

1. State the question and choose the appropriate comparison.
2. Collect evidence with deterministic commands.
3. Normalize and correlate the evidence before drawing conclusions.
4. Inspect the files and symbols responsible for the signal.
5. Decide whether to act, monitor, or document an intentional exception.
6. Re-run the measurement after any change.

:::important Evidence, not verdicts
Do not refactor solely because complexity, fan-out, churn, or duplication is high. Parsers,
orchestrators, platform adapters, tests, and generated structures can be intentionally complex or
similar. Prefer trends and combinations of signals over isolated totals.
:::

## Continuous codebase health

- **[Build a codebase health timeline in CI](continuous-health.md)** — capture one metrics partition
  per default-branch commit, query trends, and investigate changes without treating every increase
  as a regression. The worked example uses ctx's own 98-partition history.

## Pull-request governance

- **[Govern a pull request without inheriting old debt](pr-governance.md)** — compare against the
  merge base, separate reporting from blocking policy, investigate the responsible code, and keep
  analysis of fork pull requests isolated from privileged comment publishing.

## Architecture governance

- **[Detect architectural drift before it becomes a rewrite](architecture-drift.md)** — define
  reviewed boundaries, stop new drift at the pull request, correlate historical coupling and
  violation signals, and distinguish code movement from policy-coverage changes.

## Refactoring pressure

- **[Investigate chronic hotspots without chasing large files](chronic-hotspots.md)** — compare
  meaningful churn windows, identify the responsible symbols, confirm persistent pressure in
  snapshot history, and act only when a clearer ownership boundary emerges.

## Intentional complexity

- **[Review and document intentional complexity](intentional-complexity.md)** — separate fan-in
  from fan-out, apply a responsibility-coherence test, evaluate proposed boundaries, and record why
  a parser, dispatcher, shared primitive, or transaction flow should remain complex.

## Duplication

- **[Track duplication trajectories without forcing abstractions](duplication-trajectories.md)** —
  distinguish current, changed-file, and newly introduced pairs; normalize history by repository
  size; inspect persistence and ownership; and extract reuse only when synchronization cost falls.

## Release decisions

- **[Produce a release health report that supports decisions](release-health-report.md)** — compare
  immutable releases with provenance, normalize growth, investigate material signals, document
  policy coverage and uncertainty, and assign actions without inventing one health score.

## Cookbook v1 and v2

The first recipe set now covers the complete governance loop: pull-request deltas, default-branch
history, architectural drift, chronic hotspots, intentional complexity, duplication trajectories,
and release decisions. Cookbook v2 applies the same evidence-first method to everyday agent work,
beginning with unfamiliar-codebase orientation and continuing through context construction,
planning, implementation, debugging, review, search, custom analysis, integrations, and adoption.

## Point-in-time or longitudinal?

| Question | Start with |
|---|---|
| Did this branch introduce a regression? | `ctx score --against <base>` |
| Does this change violate architecture policy? | `ctx check --against <base>` |
| Where are today's refactoring pressure points? | `ctx hotspots` |
| Is repository health improving over time? | `ctx snapshot` + `ctx sql --snapshots` |
| When did a metric change, and what caused it? | Snapshot trend, then git and graph investigation |

Use a point-in-time gate for a merge decision. Use longitudinal analysis to identify sustained
direction, recurring pressure, and questions that need engineering interpretation.
