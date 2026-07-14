---
id: evidence-backed-implementation
title: Implement a feature with an evidence-backed working set
sidebar_position: 13
---

# Implement a feature with an evidence-backed working set

A good implementation loop does not freeze the working set chosen during planning. It starts with
the smallest evidence-backed set, establishes a baseline, makes one coherent change, then uses the
compiler, tests, contracts, and a refreshed ctx index to discover what the edit actually affected.

This recipe was exercised in an isolated clone of ctx by adding file-based target disambiguation to
one query command. The experiment changed two Rust files, but validation expanded the real working
set to documentation, the changelog, and a generated CLI contract. The experimental feature is not
the point of the recipe and is not a claim about the released CLI; the verified edit-and-reindex
loop is the reusable result.

## Define behavior before files

Write a small implementation brief:

```text
Behavior: let a caller identify one same-named query target by file
Unchanged behavior: traversal may still include callers from every file
Contracts: CLI help, JSON output, exit behavior, no-default-feature build
Acceptance examples: two same-named symbols resolve to different targets by file
Non-goal: redesign graph resolution or the analytics API
```

This makes file selection a consequence of behavior and contracts. Starting with “edit the query
module” would have hidden the public surfaces that later had to change.

## 1. Isolate an experiment when the repository is already in use

The cookbook worktree contained unrelated documentation changes, so the implementation trial used
a separate local clone. A branch or worktree is equally suitable. The requirement is that baseline
and post-edit evidence refer to one clean comparison point.

Before editing, record:

```bash
git status --short
git rev-parse HEAD
ctx index
```

Do not delete, reset, or silently include an existing user's changes. If isolation is not possible,
state exactly which dirty files belong to the experiment and exclude the rest from conclusions.

## 2. Build the initial working set from multiple kinds of evidence

For the trial, repository search found the command definition, dispatch, analytics implementations,
docs, and contract snapshot:

```bash
rg -n "Impact|impact_analysis|query impact" \
  src tests docs governance/contracts

ctx query find impact --json
ctx query deps impact_analysis --file src/analytics/duckdb.rs
ctx source impact_analysis --file src/analytics/duckdb.rs
ctx source impact_analysis --file src/analytics/stub.rs
```

Source inspection established an important boundary: the analytics API already accepted a full
symbol ID. Target disambiguation could therefore happen in the command layer without changing
analytics or its public API.

The initial edit set became:

- `src/cli.rs` for the argument contract;
- `src/commands/query.rs` for resolution, output, and focused tests;
- command and JSON documentation;
- the changelog;
- the generated CLI contract, updated only through its canonical capture script.

The analytics files remained validation-only. Evidence should remove files from the edit set as
well as add them.

:::caution A focused map can still be global
`ctx map --focus query_impact` found no matching symbol in this experiment and returned a broadly
ranked map. The dispatch logic was inline rather than named `query_impact`. Confirm that focused
output contains the requested concept before using it as a working set.
:::

## 3. Establish a narrow baseline

Run the least expensive tests that prove the owner boundary works before changing it:

```bash
cargo test --locked --all-features commands::query
```

The baseline passed eight query tests. This matters: a post-edit failure can now be attributed to
the experiment rather than assumed to have existed beforehand.

Reuse the repository's configured build cache when its contributor workflow permits it. The first
isolated test invocation used a fresh target directory and began rebuilding DuckDB; pointing Cargo
at the established target directory reduced the feedback loop substantially. Treat cache reuse as
an optimization, not a reason to skip a clean CI build later.

## 4. Make one coherent change and test through the public surface

The trial added the argument, resolved a filtered target from the index, passed its full ID into the
existing analytics layer, documented the JSON addition, and added a resolver test. Then it ran:

```bash
cargo fmt --all
cargo test --locked --all-features commands::query
cargo build --locked --all-features
```

Do not stop at unit tests. Exercise the compiled interface with acceptance examples that distinguish
the new behavior from an implementation that merely parses the option:

```text
query the name "run" with the main source file -> resolves the main-program target
query the name "run" with the performance source file -> resolves the harness target
query with a missing file -> empty machine result and successful no-result exit
```

The compiled trial passed all three. Help output also exposed the new option. These checks caught the
behavior users and scripts see, including stdout, stderr, JSON fields, and exits.

## 5. Let contracts expand the working set

A CLI change is not complete when Rust compiles. Search the repository's contract surfaces and use
their owning tools:

```bash
rg -n "query\.impact|query impact" docs governance tests src
python3 scripts/check-contracts.py capture --binary target/debug/ctx
python3 scripts/check-contracts.py check --binary target/debug/ctx
python3 scripts/check-governance.py check
```

The capture changed the CLI snapshot, and the comparison then passed. The snapshot was generated,
not hand-edited. Documentation covered both human usage and JSON shape, and the changelog recorded
the public behavior.

The first changelog edit accidentally landed under the latest released version. Reviewing the diff
caught it and moved it to `Unreleased`. Automation can keep a generated contract synchronized; it
cannot decide that a release note is in the correct historical section.

## 6. Re-index after editing

The pre-edit index cannot describe symbols that did not exist yet. Refresh it and query the new
structure:

```bash
ctx index
ctx query find resolve_impact_target --json
ctx query deps resolve_impact_target --file src/commands/query.rs
```

The refreshed index found the new resolver and its test, and showed the resolver's direct dependency
on the filtered symbol lookup. That confirmed the intended command-layer boundary from a second
view, while source inspection remained the authority for semantics.

Re-indexing reported zero files processed during the final run because an earlier acceptance command
had already refreshed the index. Use the resulting index contents and symbol counts to establish
freshness; do not equate “zero processed” with “the edit was not indexed.”

## 7. Compare the actual change with the planned change

```bash
ctx diff HEAD --summary --changes-only --no-tree >/dev/null
ctx diff HEAD --summary --changes-only --no-tree --max-tokens 20000
ctx score --against HEAD --json
git diff --check
git diff --stat
git diff
```

The 8,000-token budget is only the `ctx diff` default, not a recommended ceiling. On Unix, redirect
stdout to preview the summary and token count without retaining the streamed context, then set
`--max-tokens` for the task and receiving model. In ctx 0.3.5, the global `--count-only` option is
accepted after `ctx diff` but does not suppress its context output, and `--summary` adds a summary on
stderr rather than replacing the content.

In the trial, the default-budget run found eight changed files and seven affected symbols, but its
context pack included only one documentation file and omitted the other seven. The summary was
useful; that particular pack was not complete enough for review.

Raising the budget is the first response when the complete change fits the available context window.
Because ctx packs whole files, the required increase can be substantial when one changed file is
large. If the full change still does not fit, split it into explicit, reviewable groups rather than
silently accepting omissions. Always check changed-file and omitted-file counts before handing
generated context to an agent.

`ctx score` scoped code metrics to the two changed Rust files and reported:

```text
complexity_delta: 70
fan_out_delta: 32
symbols_added: 2
new_duplication: 0
```

Those increases triggered a structural review of the already-large query module. They did not prove
the feature was bad: much of the measured delta came from explicit output and no-result branches,
and the new resolver stayed at the intended boundary. A future refactor may still be worthwhile,
but it should be justified by responsibility and ownership, not by making the delta green.

## 8. Validate the configurations the feature can affect

The query command exists with and without the optional analytics engine, so the trial checked both
configurations as well as repository contracts:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked --all-features commands::query
cargo test --locked --no-default-features commands::query
python3 scripts/check-contracts.py check --binary target/debug/ctx
python3 scripts/check-governance.py check
```

All checks passed, with nine focused query tests after the new case was added. This is proportional
validation for the experiment, not a substitute for the repository's complete CI suite before a
real merge.

## Keep an evidence ledger

```text
Requested behavior and non-goals:
Clean comparison point:
Initial edit files, with reason:
Validation-only files, with reason:
Baseline command and result:
Acceptance examples and observed exits/output:
Contracts regenerated through canonical tools:
Post-edit symbols and relationships:
Diff omissions or stale-index risks:
Metric deltas and interpretation:
Feature configurations checked:
Uncertainty and full-CI work remaining:
```

The ledger makes it possible to distinguish verified behavior from inference when the work is handed
to another agent or reviewer.

## What worked, and what did not

| Technique | Verified use | Limitation observed |
|---|---|---|
| Source plus dependency queries | Kept the analytics API out of the edit set | Inline dispatch was not discoverable under the expected name |
| Focused baseline tests | Established an eight-test clean starting point | A fresh target directory caused an expensive DuckDB rebuild |
| Compiled CLI examples | Proved target selection, JSON, help, and no-result behavior | Manual examples should become integration tests before shipping |
| Canonical contract capture | Kept help output and the CLI snapshot synchronized | It could not detect the changelog entry in the wrong release section |
| Re-index and query | Confirmed the new resolver and its direct dependency | “Zero files indexed” can mean an earlier command already refreshed it |
| `ctx diff` | Summarized the candidate set before choosing a task-appropriate budget | The 8k default omitted seven of eight changed files; `--count-only` did not suppress diff content |
| `ctx score` | Quantified structural movement in changed code | Rising deltas required interpretation, not automatic rejection |
| All/default-free compilation paths | Verified the feature boundary in both configurations | Focused tests are still narrower than complete CI |

The reliable loop is **define behavior, select from evidence, baseline, edit, exercise the public
surface, refresh contracts and the index, compare the actual change, then validate every affected
configuration**.

## Give the workflow to an agent

```text
Implement this feature with an evidence-backed working set. Begin by stating behavior, non-goals,
contracts, and acceptance examples. Inspect source, symbol relationships, tests, docs, generated
contracts, and feature-gated variants to select the smallest initial edit set; keep a separate list
of validation-only files. Run a focused baseline before editing. Make one coherent change and test
it through the compiled public interface, including JSON, stderr, and exit behavior where relevant.
Use canonical generators for owned artifacts. Re-index after editing, query the new or changed
symbols, and compare against the clean base with ctx diff, ctx score, and git diff. Report omitted
context, metric interpretation, configurations checked, and remaining uncertainty. Do not declare
completion from a green unit test or a single metric.
```

## Next in Cookbook v2

The next recipe will apply the same before-and-after discipline to debugging a regression: reproduce
the symptom, localize the responsible path, and prove the fix without confusing correlation for
cause.
