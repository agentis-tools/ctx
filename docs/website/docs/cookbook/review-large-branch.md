---
id: review-large-branch
title: Review a large branch without reading every file equally
sidebar_position: 15
---

# Review a large branch without reading every file equally

A large review is not a smaller review repeated for every file. First establish the complete change
inventory, separate mechanical artifacts from behavioral streams, then spend attention where intent,
enforcement, contracts, and tests meet. Token-budgeted context can support that work, but the files
that happen to fit are not automatically the files with the greatest review risk.

This recipe was exercised retrospectively against ctx commit `7bb2167`, which introduced release
governance and guardrails. It changed 29 files with 3,712 additions and 941 deletions across policy,
CI, release automation, Python enforcement, tests, lockfiles, and a generated CLI contract. Later
follow-up fixes provide unusually strong evidence about which review questions mattered.

## Choose the comparison that represents the branch

For a feature branch, compare with its merge base rather than an arbitrary local `main`:

```bash
BASE=$(git merge-base HEAD origin/main)
git diff --stat "$BASE"..HEAD
git diff --name-status "$BASE"..HEAD
```

The worked example checked out the historical commit in an isolated clone and used its first parent:

```bash
BASE=7bb2167^
git diff --stat "$BASE"..HEAD
git diff --name-status "$BASE"..HEAD
ctx index
```

Record the base and head SHAs in the review. A moving branch name makes later measurements difficult
to reproduce, especially when the branch is rebased during review.

## 1. Get the complete inventory before packing context

On Unix, retain ctx's stderr summary while discarding the streamed file contents:

```bash
ctx diff "$BASE" --summary --changes-only --no-tree >/dev/null
```

The summary identified all 29 changed paths and the new Python symbols. Keep the independent git
inventory beside it: ctx can add symbol information, while git remains the authority for the full
changed-file set, including YAML, Markdown, lockfiles, and generated JSON.

:::caution Summary and count options do not suppress diff content in ctx 0.3.5
`--summary` adds diagnostics on stderr but still streams context on stdout. The inherited
`--count-only` option is accepted after `ctx diff` but is not applied by the diff command. Redirect
stdout when you want only the diagnostic preview.
:::

## 2. Treat the token budget as capacity, not priority

The default 8,000 tokens are not a required or recommended review budget. Try a budget appropriate
to the receiving model and task:

```bash
ctx diff "$BASE" --summary --changes-only --no-tree --max-tokens 20000
ctx diff "$BASE" --summary --changes-only --no-tree --max-tokens 50000
```

Whole-file packing produced these representative results:

| Budget | Files selected | Files omitted |
|---:|---:|---:|
| 8,000 | varied between repeated runs | varied between repeated runs |
| 20,000 | 16 in one content run | 13 |
| 50,000 | 27 in one content run | 2 |

Three identical 8,000-token diagnostic runs selected 13, 13, and 7 files. Changed files all had the
same priority, and their equal-priority ordering was not stable before greedy packing. Therefore, in
ctx 0.3.5:

- do not infer review priority from which changed files fit;
- do not assume two runs at the same budget produce the same bundle;
- check the selected and omitted counts every time;
- use an explicit review-stream inventory when completeness or reproducibility matters.

Raising the budget improved coverage but did not solve prioritization. Large generated files and
lockfiles can consume capacity while small workflow changes carry greater operational risk.

## 3. Split the branch by review responsibility

The 29 files became four coherent streams:

| Stream | Representative files | Review question |
|---|---|---|
| Intent and ownership | `governance/*.md`, `AGENTS.md`, `CLAUDE.md`, `CODEOWNERS` | Is the promised policy coherent, scoped, and owned? |
| Enforcement | `scripts/version.py`, contract/governance checkers, versioning tests | Does executable policy match the prose, including failure paths? |
| Workflow wiring | policy, CI, snapshot, docs, and release workflows | Are permissions, events, toolchains, dependency order, and secrets correct? |
| Mechanical and generated | both lockfiles, CLI contract, changelog, ignore rules | Is the artifact expected, reproducible, and reviewed through its owner? |

This is more reliable than reviewing by directory or largest line count. Each stream has a different
definition of correctness and a different validator.

The lockfile diff was the largest textual change—1,644 lines—but neither Cargo manifest changed in
the commit. That is a review prompt: establish which dependency update regenerated the locks and
which checks justify it. It is not a reason to read every lockfile line before examining release
permissions or enforcement behavior.

## 4. Use score to route attention, not to cover the branch

```bash
ctx score --against "$BASE" --json
```

For this commit, score reported only the five new Python implementation and test files:

```text
files_changed: 5
complexity_delta: 1107
fan_out_delta: 520
symbols_added: 84
new_duplication: 1
```

The git inventory contained 29 files. Score did not evaluate the workflows, policy prose, lockfiles,
shell scripts, generated contract, or ownership configuration as code metrics. Its per-file results
still routed attention effectively: the 500-line `scripts/version.py` accounted for the largest new
complexity and fan-out.

Because every measured Python file was new, the large deltas mostly described added capability, not
regression. Review the responsibilities, tests, and integration of the new code instead of setting a
goal to make additive metrics zero.

## 5. Investigate every concrete signal

```bash
ctx duplicates --against "$BASE" --json
```

The one new duplicate was a 73-token subprocess wrapper shared by two standalone policy scripts.
Source inspection showed that the scripts deliberately owned separate command-line entry points.
Extracting a common module would add packaging and invocation coupling for very little behavioral
reuse. The pair was worth reviewing, but retaining it was reasonable.

Use the same disposition format for every signal:

```text
Signal:
Responsible symbols/files:
Source evidence:
Behavioral or ownership interpretation:
Action: fix, test, document, accept, or defer
Owner and re-evaluation trigger:
```

## 6. Trace distinctive symbols inside each stream

Avoid expanding a graph from generic names such as `main`, `check`, `parse`, or `current_version`.
Choose symbols distinctive enough to preserve meaning:

```bash
ctx query callers validate_changelog --file scripts/version.py
ctx query deps validate_changelog --file scripts/version.py

ctx query callers pr_policy --file scripts/check-contracts.py
ctx query deps pr_policy --file scripts/check-contracts.py

ctx query callers compare_contracts --file scripts/check-contracts.py
```

These direct queries connected:

- changelog validation to the version check entry point;
- pull-request policy to contract comparison, base versions, labels, and breaking notes;
- contract comparison to both policy execution and its focused unit test.

Read the owner function and its tests together. Then inspect the workflow step that invokes it. This
forms a review triangle: **declared policy → executable enforcement → CI wiring**.

## 7. Be skeptical of automatic graph expansion

The experiment also ran diff context without `--changes-only`:

```bash
ctx diff "$BASE" --summary --depth 1 --no-tree --max-tokens 200000 >/dev/null
ctx diff "$BASE" --summary --depth 3 --no-tree --max-tokens 200000 >/dev/null
```

Depth 1 expanded 29 changed files to 30 context files by adding `perf/src/main.rs`. Depth 3 expanded
to 40 files, including unrelated Rust indexing, configuration, schema, harness, scoring, and error
modules. Generic same-named symbols in the new Python scripts caused cross-language false-positive
paths.

Automatic expansion was less useful than `--changes-only` for this branch. Use it as a hypothesis
source, and retain an added file only after its source contains the relationship that led to it.
Direct queries from distinctive symbols were much cleaner.

:::caution Positional file patterns do not scope `ctx diff` in ctx 0.3.5
The global help displays positional patterns after the revision, but a verified run with
`scripts/version.py` still analyzed all 29 files. Use `git diff -- <paths>` for an exact stream and
use ctx source/query commands to build symbol context within it.
:::

## 8. Review claims against their enforcement

For every policy statement containing words such as “enforces,” “requires,” “never,” or “only,” find
the executable mechanism and its negative test.

For this branch, focused local validation passed:

```bash
python3 scripts/check-governance.py check
python3 -m unittest discover -s tests/versioning -p 'test_*.py'
python3 scripts/version.py show
python3 scripts/version.py check --skip-binary
```

The governance boundary check passed, all 12 focused tests passed, and version invariants were
consistent. Those results proved the tested script behavior; they did not prove GitHub permissions,
branch protection, environments, action compatibility, or every workflow matrix path.

Review external claims separately. This commit correctly documented that repository files could not
prove branch protection or release-environment configuration. A reviewer should preserve that
uncertainty rather than treating policy prose as external state.

## 9. Use follow-up history to calibrate the review

Later commits fixed several issues in or around the original change:

- normal CI jobs explicitly selected Rust 1.91 and used `--locked`;
- Clippy expanded to all targets and all features;
- checkout credentials were disabled and permissions narrowed;
- the cargo-deny action was updated for CVSS v4 advisories;
- the isolated performance harness gained license metadata and an explicit wildcard-path exception.

These follow-ups validate the review-stream model. They live at the intersections that deserve the
most attention: stated reproducibility versus actual CI commands, root policy versus the isolated
`perf/` package, and pinned workflow syntax versus the installed action version.

Retrospective evidence is not available during an ordinary review, so turn each lesson into a
question:

```text
Does every job use the declared toolchain and lockfile?
Does lint cover the feature/target matrix claimed by policy?
Are checkout credentials and job permissions minimized?
Does dependency policy cover every manifest, including isolated harnesses?
Is the pinned action version compatible with the configured policy format?
```

## Produce a risk-ranked briefing

```text
Base and head SHAs:
Complete changed-file inventory:
Review streams and owners:
High-risk behavior or permission changes:
Generated/mechanical artifacts and generators:
Score coverage and uncovered file types:
Concrete metric or duplication signals, with disposition:
Verified symbol-to-test-to-workflow paths:
Rejected graph expansions:
Commands and configurations validated:
External settings not proven from the repository:
Blocking findings:
Follow-up questions and optional improvements:
```

Rank findings by consequence and evidence, not file size. A four-line permission change can outrank
a thousand-line lockfile update, while an unexplained lockfile regeneration can still block until
its provenance is established.

## What worked, and what did not

| Technique | Verified use | Limitation observed |
|---|---|---|
| Git inventory plus ctx summary | Covered all 29 paths and added symbol detail | `--summary` still streamed content |
| Adjustable budgets | Increased whole-file coverage up to 27 of 29 files | Equal-priority packing was nondeterministic |
| Review streams | Matched intent, enforcement, wiring, and generated ownership | Requires human classification |
| `ctx score` | Ranked five new Python code/test files | Did not cover 24 workflow, policy, shell, lock, or generated files |
| New-duplication inspection | Found and dispositioned one exact helper pair | A duplicate finding did not imply extraction was beneficial |
| Distinctive direct queries | Connected policy functions, callers, dependencies, and tests | Generic names remained ambiguous |
| Automatic depth expansion | Exposed possible related context | Added unrelated files at depth 1 and worsened at depth 3 |
| Focused repository checks | Verified 12 tests and local policy/version invariants | Could not prove external GitHub settings or hosted matrix behavior |
| Follow-up commit analysis | Confirmed which review intersections were fragile | Available only retrospectively |

The reliable loop is **inventory everything, classify review streams, route with metrics, trace
distinctive symbols, verify claims against enforcement, reject false expansion, and report external
uncertainty**.

## Give the workflow to an agent

```text
Review this large branch without reading every changed file equally. Record immutable base and head
SHAs, then build a complete git inventory before generating context. Split the branch into coherent
review streams such as intent/ownership, executable enforcement, workflow wiring, product behavior,
tests, and generated or mechanical artifacts. Use ctx score only for the file types it reports and
investigate every concrete signal from source. Trace distinctive symbols into callers, dependencies,
tests, and invoking workflows; reject automatic graph expansions whose source does not verify the
relationship. Choose token budgets for capacity, report selected and omitted files, and do not infer
priority from the packed subset. Verify policy claims with negative tests and workflow wiring, and
separate repository evidence from external settings that cannot be proven locally. Produce a
risk-ranked briefing with blocking findings, accepted signals, validation, and uncertainty.
```

## Next in Cookbook v2

The next recipe will compare keyword, natural-language, semantic, and structural retrieval against
the same engineering questions so users can choose the right search mode deliberately.
