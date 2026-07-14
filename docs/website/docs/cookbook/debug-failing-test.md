---
id: debug-failing-test
title: Debug a failing test with a focused evidence loop
sidebar_position: 14
---

# Debug a failing test with a focused evidence loop

A failing test is already a compact description of observed behavior. Start there. Reproduce the
exact failure, translate its output into a violated invariant, trace the test into production code,
and compare likely implementations before loading the surrounding subsystem.

This recipe was exercised in an isolated clone of ctx with a controlled one-character regression in
token-budget selection. The experiment changed an inclusive boundary into an exclusive one. The
existing test—not a test written to reveal the injected bug—caught it. The lab change was restored
afterward; the purpose was to verify the diagnostic workflow, not to alter the product.

## Begin with the symptom, not a theory

Run exactly the test and configuration that reported the problem:

```bash
cargo test --locked --all-features \
  tokens::tests::test_select_files_by_tokens
```

The controlled regression produced:

```text
assertion `left == right` failed
  left: 250
 right: 300
at src/tokens.rs:284
```

Record facts before interpreting them:

```text
Test: test_select_files_by_tokens
Configuration: all features
Observed total: 250
Expected total: 300
Assertion location: src/tokens.rs:284
Reproducibility: fails in isolation
```

The output does not prove which production line is wrong. It establishes the violated invariant:
items whose total is exactly equal to the budget must be included.

If the test passes alone but fails in the suite, preserve that evidence. The likely search area then
shifts toward shared state, ordering, time, environment, or concurrency rather than the asserted
function alone.

## 1. Refresh the index for the failing branch

```bash
ctx index
ctx query find test_select_files_by_tokens --json
```

The experiment re-indexed one changed file and located the test at its current lines. Do this before
trusting graph or source results: an index from the passing branch can point at stale symbols and
relationships.

Index freshness is necessary but not sufficient. Macros, dynamic dispatch, reflection, generated
code, and unsupported language constructs may still hide relationships. Keep the test output and
exact repository search available as independent evidence.

## 2. Read only the failing test first

```bash
ctx source test_select_files_by_tokens --file src/tokens.rs
```

The test contained three cases:

| Budget | File sizes | Expected selection | Expected total |
|---:|---|---:|---:|
| 500 | 100, 200, 150 | 3 | 450 |
| 300 | 100, 200, 150 | 2 | 300 |
| 150 | 100, 200, 150 | 1 | 100 |

Only the exact-fit case failed. That distinction narrows the hypothesis from “selection is broken”
to “the budget boundary may be exclusive.” Read assertions as examples of the contract, not merely
obstacles to make green.

## 3. Trace the test into production code

```bash
ctx query deps test_select_files_by_tokens --file src/tokens.rs
ctx query callers select_files_by_tokens --file src/tokens.rs
ctx source select_files_by_tokens --file src/tokens.rs
```

`query deps` returned the three calls from the test to `select_files_by_tokens`, including their
source lines. The reverse query confirmed those same test calls. The production source then exposed
the candidate boundary:

```rust
if total_tokens + file.tokens < max_tokens {
```

This was enough to form a specific hypothesis: equality was being rejected, causing the 200-token
second file to be omitted when the running total was 100 and the budget was 300.

:::note Keep recursive traversal proportional
`ctx query impact select_files_by_tokens --depth 2 --json` returned only the failing test at distance
one. That was useful confirmation, but recursive expansion added nothing. A small direct path is a
reason to stop, not a reason to request a deeper graph.

In ctx 0.3.5, `--depth` on `query deps` and `query callers` is accepted but does not expand beyond
direct relationships. Use impact or graph when recursion is actually needed.
:::

## 4. Compare neighboring behavior

Search for implementations that encode the same concept:

```bash
ctx similar "select files within token budget" --keyword --limit 8 --json
ctx source select_by_token_budget --file src/tokens.rs
ctx source filter_files_by_tokens --file src/commands/context.rs
```

Keyword similarity found both a generic selector and the context command's selector. Each used an
inclusive comparison:

```rust
if total + tokens <= max_tokens {
```

That was independent supporting evidence for the hypothesis: the documented meaning of a maximum
budget and two neighboring implementations all allowed exact fits.

Do not copy a neighbor automatically. Parallel implementations may intentionally differ. Compare:

- input and output types;
- ordering and greedy-selection rules;
- behavior for an oversized first item;
- equality and zero-budget boundaries;
- diagnostics and omission counts;
- caller expectations.

The similarity scores in this experiment were all near `0.94–0.96` and ranked functions, tests, and
loosely related budget code together. They were useful rankings, not confidence probabilities.

## 5. Use branch context without confusing it for causality

```bash
ctx diff HEAD src/tokens.rs \
  --summary --changes-only --no-tree --max-tokens 4000
git diff -- src/tokens.rs
```

Because this was a controlled uncommitted regression, the diff identified one changed symbol and the
one-character comparison change immediately. In a real branch, a changed line is only a suspect:
the failure may expose an older bug, depend on another commit, or arise from changed test setup.

Use diff evidence to answer “what changed near the failing path?” Do not silently upgrade that to
“this change caused the failure.” Causality requires a reproducer that fails with the candidate and
passes when only the candidate is corrected or removed.

The 4,000-token limit was sufficient for this 2,444-token file. It is not a fixed debugging budget.
Measure or increase the budget when the changed path is larger, and inspect omitted-file counts.

## 6. Make the smallest causal correction

The evidence supported one correction:

```diff
- if total_tokens + file.tokens < max_tokens {
+ if total_tokens + file.tokens <= max_tokens {
```

The fix restored the stated maximum-budget behavior without changing ordering, omission handling,
or other token-counting paths.

When the cause is not this localized, keep a hypothesis ledger and change one causal factor at a
time:

```text
Hypothesis:
Evidence for:
Evidence against:
Smallest discriminating test:
Result:
Keep or reject:
```

Avoid bundles of speculative fixes. A green result after three unrelated edits does not reveal which
edit was necessary.

## 7. Prove the fix at widening scopes

First rerun the exact reproducer, then its owner boundary, then relevant configurations:

```bash
cargo test --locked --all-features \
  tokens::tests::test_select_files_by_tokens

cargo test --locked --all-features tokens::tests
cargo test --locked --no-default-features tokens::tests
```

The complete six-test token suite passed with and without default features. The broader tests check
that the correction did not disturb encoding, detailed counts, or estimation behavior in the same
module.

Finally refresh ctx and verify the branch state:

```bash
ctx index
ctx score --against HEAD --json
git diff --check
git status --short
```

After restoring the correct boundary, ctx re-indexed the file and `score --against HEAD` reported
zero changed files and zero metric deltas. That proves the lab was returned to its baseline; it would
not, by itself, prove a nontrivial fix correct. The test progression supplies the behavioral proof.

## Write the debugging handoff

```text
Exact failing command and configuration:
Observed output and assertion location:
Violated behavioral invariant:
Index freshness:
Test-to-production path:
Candidate changed lines:
Neighboring implementations inspected:
Rejected hypotheses:
Causal correction:
Exact reproducer after fix:
Broader suites and configurations:
Static-analysis limitations:
Remaining runtime, platform, or integration uncertainty:
```

This keeps a reviewer from having to reconstruct which facts were observed and which conclusions
were inferred.

## What worked, and what did not

| Technique | Verified use | Limitation observed |
|---|---|---|
| Exact isolated test | Produced a stable numeric symptom | A passing isolated test would not reproduce suite-order failures |
| Refreshed index and test source | Located current assertions without loading the subsystem | Static indexing cannot observe every runtime relationship |
| Test dependencies and reverse callers | Found all three direct production calls and their lines | Direct queries do not explain state or data values |
| Production `source` | Reduced the candidate logic to one function | Reading code alone does not establish that a suspicious line changed |
| Keyword similarity | Found two independent inclusive-budget implementations | High scores also ranked tests and loosely related functions |
| Focused diff | Identified the changed symbol and comparison | Changed code is a suspect, not automatic causal proof |
| Impact depth 2 | Confirmed the test was the only indexed dependent | Deeper traversal added no evidence |
| Widening tests | Proved the exact case and neighboring module behavior in two configurations | Complete CI may still cover additional integrations and platforms |

The reliable loop is **reproduce, state the invariant, refresh, trace the test, inspect the smallest
production path, compare neighbors, make one causal correction, then widen validation**.

## Give the workflow to an agent

```text
Debug this failing test without loading the whole subsystem. Reproduce the exact test and record its
configuration, output, and violated invariant before proposing a cause. Refresh ctx, inspect the
test source, and trace its direct production dependencies and callers. Read the smallest candidate
functions, compare neighboring implementations where they can discriminate behavior, and inspect
branch changes without assuming changed code is causal. Keep explicit hypotheses and reject those
the evidence contradicts. Make one smallest causal correction. Prove it first with the exact
reproducer, then the owning test group and every relevant feature or backend configuration. Report
static-analysis gaps, rejected hypotheses, and remaining integration uncertainty.
```

## Next in Cookbook v2

The next recipe will use diff summaries, graph expansion, and adjustable token budgets to review a
large branch without reading every changed file equally.
