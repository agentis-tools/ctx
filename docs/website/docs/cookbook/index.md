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

## Start from your problem

| What you are experiencing | Start here |
|---|---|
| “I do not know where this application starts or how it fits together.” | [Understand an unfamiliar codebase](unfamiliar-codebase) |
| “The agent keeps loading too much—or omitting one critical file.” | [Build the smallest useful context](smallest-useful-context) |
| “I suspect this behavior already exists under another name.” | [Find existing implementations](find-existing-implementations) |
| “A small edit keeps breaking callers, formats, or generated artifacts.” | [Trace the blast radius](blast-radius) |
| “I know the behavior I want, but not the complete working set.” | [Implement with an evidence-backed working set](evidence-backed-implementation) |
| “A test fails and the surrounding subsystem is too large to load.” | [Debug with a focused evidence loop](debug-failing-test) |
| “This branch is too large to review every file with equal attention.” | [Review a large branch](review-large-branch) |
| “The PR gate is charging this change for old debt.” | [Govern a pull request](pr-governance) |
| “ctx blocked a change that we believe is correct—or the index disagrees with source.” | [Recover from a gate or stale evidence](gate-recovery) |
| “We need to analyze fork PRs without exposing write credentials.” | [Run ctx safely on untrusted CI code](untrusted-ci) |
| “Dependencies are crossing boundaries one reasonable-looking change at a time.” | [Detect architectural drift](architecture-drift) |
| “The same files keep becoming difficult to change.” | [Investigate chronic hotspots](chronic-hotspots) |
| “A function scores highly, but its complexity may be intentional.” | [Review intentional complexity](intentional-complexity) |
| “Duplicate code is rising, but forcing abstraction may make things worse.” | [Track duplication trajectories](duplication-trajectories) |
| “We want evidence about whether codebase health is changing over time.” | [Build a health timeline](continuous-health) |
| “We need a release decision, not another dashboard.” | [Produce a release health report](release-health-report) |

If the problem is still ambiguous, read [the concepts shared by every recipe](concepts) and begin
with a point-in-time comparison. Move to longitudinal evidence only when the question is about
direction or persistence.

## Everyday agent-assisted engineering

- **[Understand an unfamiliar codebase before editing it](unfamiliar-codebase)** — refresh the
  index, distinguish central symbols from real entry points, verify a focused execution path, and
  record static-analysis uncertainty before choosing a working set.
- **[Build the smallest useful context for a task](smallest-useful-context)** — rank candidates,
  audit writers, consumers, tests, and contracts, use snippets for investigation, and count the
  explicit implementation set before packaging it.
- **[Find existing implementations before writing new code](find-existing-implementations)** —
  vary semantic, keyword, and signature-like queries; inspect candidates and their callers; verify
  behavioral tests; and choose the correct reuse boundary.
- **[Trace the blast radius before editing a symbol](blast-radius)** — start with direct graph
  evidence, verify transitive branches from source, search persisted and public contracts, and turn
  the classified impact into a validation plan.
- **[Implement a feature with an evidence-backed working set](evidence-backed-implementation)** —
  baseline the owning boundary, exercise the compiled public surface, refresh contracts and the
  index, and interpret the actual diff and metric movement before declaring completion.
- **[Debug a failing test with a focused evidence loop](debug-failing-test)** — reproduce the
  exact symptom, trace the test into production code, compare neighboring behavior, and prove one
  causal correction at widening validation scopes.
- **[Review a large branch without reading every file equally](review-large-branch)** — inventory
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

Every recipe begins with a **Quickest version** for readers who already understand the trade-offs.
The full workflow explains how to verify the result and what the shortcut cannot prove.

:::important Evidence, not verdicts
Do not refactor solely because complexity, fan-out, churn, or duplication is high. Parsers,
orchestrators, platform adapters, tests, and generated structures can be intentionally complex or
similar. Prefer trends and combinations of signals over isolated totals.
:::

## Continuous codebase health

- **[Build a codebase health timeline in CI](continuous-health)** — capture one metrics partition
  per default-branch commit, query trends, and investigate changes without treating every increase
  as a regression. The worked example uses ctx's own 98-partition history.

## Pull-request governance

- **[Govern a pull request without inheriting old debt](pr-governance)** — compare against the
  merge base, separate reporting from blocking policy, investigate the responsible code, and keep
  analysis of fork pull requests isolated from privileged comment publishing.
- **[Run ctx safely in CI on untrusted code](untrusted-ci)** — analyze pull-request code without
  write credentials, validate artifacts on the trusted default branch, and reject stale results
  before publishing them.
- **[Recover when a gate blocks a legitimate change](gate-recovery)** — distinguish findings
  from operational failure, verify the reported relationship, correct source or reviewed policy,
  and rebuild stale evidence without weakening the gate.

## Architecture governance

- **[Detect architectural drift before it becomes a rewrite](architecture-drift)** — define
  reviewed boundaries, stop new drift at the pull request, correlate historical coupling and
  violation signals, and distinguish code movement from policy-coverage changes.

## Refactoring pressure

- **[Investigate chronic hotspots without chasing large files](chronic-hotspots)** — compare
  meaningful churn windows, identify the responsible symbols, confirm persistent pressure in
  snapshot history, and act only when a clearer ownership boundary emerges.

## Intentional complexity

- **[Review and document intentional complexity](intentional-complexity)** — separate fan-in
  from fan-out, apply a responsibility-coherence test, evaluate proposed boundaries, and record why
  a parser, dispatcher, shared primitive, or transaction flow should remain complex.

## Duplication

- **[Track duplication trajectories without forcing abstractions](duplication-trajectories)** —
  distinguish current, changed-file, and newly introduced pairs; normalize history by repository
  size; inspect persistence and ownership; and extract reuse only when synchronization cost falls.

## Release decisions

- **[Produce a release health report that supports decisions](release-health-report)** — compare
  immutable releases with provenance, normalize growth, investigate material signals, document
  policy coverage and uncertainty, and assign actions without inventing one health score.

## Cookbook v1 and v2

The first recipe set covers the complete governance loop: pull-request deltas, default-branch
history, architectural drift, chronic hotspots, intentional complexity, duplication trajectories,
and release decisions. Cookbook v2 applies the same evidence-first method to everyday agent work,
including unfamiliar-codebase orientation, context construction, implementation, debugging,
review, safe CI integration, and recovery when evidence or policy needs correction.

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

:::note About the worked measurements
Numbers in a recipe are illustrations pinned to the commit, release, and date named on that page.
They are not stable ctx output or repository-size specifications. The workflow and interpretation
method should remain valid when those values change.
:::
