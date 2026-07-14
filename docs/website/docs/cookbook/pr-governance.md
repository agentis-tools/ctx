---
id: pr-governance
title: Govern a pull request without inheriting old debt
sidebar_position: 3
---

# Govern a pull request without inheriting old debt

A useful pull-request check answers **what did this change introduce?** It should not reject a
contributor because unrelated code was already complex, duplicated, or outside the intended
architecture.

This recipe builds a delta-focused review with `ctx score --against`, then separates observations
that deserve investigation from policies that are mature enough to block a merge.

## The operating model

Use three outcomes instead of treating every metric as pass or fail:

| Outcome | Meaning | Typical signals |
|---|---|---|
| Report | Useful context, but not a verdict | complexity, fan-out, symbol churn, hotspots |
| Review | A new condition needs human interpretation | structural duplication, a large or unusual delta |
| Block | The change violates an explicit engineering contract | reviewed architecture rules, operational failure |

Do not block merely because `complexity_delta > 0`. A new parser, coordinator, state machine, or
transaction boundary may legitimately add complexity. Do not block every new near-duplicate pair
until the team has reviewed the detector's results in its own codebase and defined an exception
process.

## 1. Choose the correct base

For local branch review, fetch the default branch and compare from the merge base:

```bash
git fetch origin main
BASE="$(git merge-base HEAD origin/main)"
```

In pull-request CI, use the base SHA supplied by the forge and fetch it explicitly. Do not infer the
base from pull-request code:

```bash
git fetch --no-tags origin "${BASE_SHA}"
```

`ctx score --against <ref>` includes committed branch changes and working-tree changes relative to
the merge base with that ref. The default `--against HEAD` is useful for inspecting only
uncommitted work.

## 2. Verify what policy actually exists

Build the index, then inspect the parsed architecture policy:

```bash
ctx index

if test -f .ctx/rules.toml; then
  ctx check --list
else
  echo "No architecture policy is configured"
fi
```

This step is essential. A violation count of zero can mean any of the following:

- the change conforms to well-scoped rules;
- no rule covers the changed code;
- the rules file exists but contains no active rules;
- no rules file is configured.

Record the configuration state in the report rather than presenting every zero as proof of
conformance.

## 3. Generate the delta scorecard

Start in informational mode:

```bash
ctx score --against "$BASE"
```

The scorecard reports:

| Metric | What it can tell you | What it cannot prove |
|---|---|---|
| `complexity_delta` | changed functions gained or lost structural responsibility | positive means badly designed |
| `fan_out_delta` | changed files call more or fewer symbols | positive means excessive coupling |
| `new_duplication` | a verified similar pair was absent at the base | the two functions should share an abstraction |
| `check_violations` | scoped architecture findings exist | zero means architecture is fully governed |
| `symbols_added` / `symbols_removed` | the change alters the indexed symbol surface | API compatibility changed |
| `files_changed` | how much indexed source participated | total pull-request size |

Use JSON when another program or agent will interpret the result:

```bash
ctx score --against "$BASE" --json > score.json
jq '.data.metrics, .data.per_file, .data.notes' score.json
```

The per-file values matter. A repository-wide delta can hide whether responsibility accumulated in
one central file or was spread across several small additions.

## 4. Add focused evidence

The composite score is the routing layer, not the end of the investigation:

```bash
ctx duplicates --against "$BASE" --json > duplicates.json
ctx hotspots --against "$BASE" --limit 100 --json > hotspots.json

if test -f .ctx/rules.toml; then
  status=0
  ctx check --against "$BASE" --json > check.json || status=$?
  test "$status" -le 1 || exit "$status"
fi
```

These commands deliberately distinguish findings from operational errors:

- exit `0` means the command ran successfully;
- exit `1` means a requested gate or architecture check found something;
- exit `2` means the analysis itself failed and its result must not be trusted.

`ctx check --against` excludes violations that only involve untouched files. If a contributor edits
a legacy file, however, a violation touching that file can enter scope even if the relationship
predates the pull request. That is useful review context, but it is not identical to proving that
the pull request created the relationship.

## 5. Investigate the responsible code

For a surprising per-file delta or architecture finding, inspect ownership and blast radius:

```bash
ctx map --focus <changed-path> --budget 3000
ctx query callers <symbol>
ctx query deps <symbol>
ctx query impact <symbol>
ctx source <symbol>
```

Ask:

1. Is the added responsibility inherent to the operation?
2. Does the symbol sit at an intended orchestration or boundary layer?
3. Is the similar implementation required to evolve independently?
4. Is there an existing abstraction that the change should reuse?
5. Does the architecture rule express a real contract for this part of the repository?

Classify the result as intentional, likely accidental, or insufficient evidence. Only the second
category implies corrective work; insufficient evidence calls for more inspection.

## 6. Introduce blocking gradually

Use a maturity ladder:

### Phase 1: report

Publish the scorecard without `--fail-on`. Learn the repository's normal deltas and false-positive
patterns.

### Phase 2: require review

Highlight new duplication, unusually concentrated complexity, and touched legacy violations in a
pull-request comment. Let a reviewer accept intentional cases with an explanation.

### Phase 3: block explicit contracts

Once `.ctx/rules.toml` has reviewed coverage, enforce the agreed conditions:

```bash
set +e
ctx score \
  --against "$BASE" \
  --fail-on "check_violations>0" \
  --json > score.json
status=$?
set -e

case "$status" in
  0) echo "ctx policy passed" ;;
  1) echo "ctx policy found a blocking condition" >&2; exit 1 ;;
  *) echo "ctx analysis failed" >&2; exit "$status" ;;
esac
```

Add a threshold such as `new_duplication>0` only if the team has deliberately decided that every
new detected pair requires resolution before merge. Avoid universal complexity thresholds unless
they were calibrated against representative changes and have a documented exception path.

## 7. Keep fork pull requests safe

A workflow that analyzes untrusted pull-request code should not also hold permission to comment on
the pull request. Use two workflows:

1. A `pull_request` workflow with `contents: read` checks out the head, runs ctx, and uploads JSON.
2. A trusted `workflow_run` workflow checks out only the default branch, validates the artifact and
   its metadata, and writes or updates the comment without executing pull-request code.

Also pin ctx and every third-party action, disable persisted checkout credentials in the analysis
job, cap artifact sizes, validate command and schema fields, and confirm that the pull request still
points at the analyzed head SHA immediately before publishing.

ctx uses this design in its own
[`ctx-pr-analysis.yml`](https://github.com/agentis-tools/ctx/blob/main/.github/workflows/ctx-pr-analysis.yml)
and
[`ctx-pr-comment.yml`](https://github.com/agentis-tools/ctx/blob/main/.github/workflows/ctx-pr-comment.yml).

## What this recipe found in ctx itself

ctx's PR workflow is intentionally report-first. It captures the audit, map, index statistics,
score, duplicates, hotspots, and architecture check as separate JSON documents. The privileged
publisher validates those documents and produces a sticky comment, but does not execute code from
the pull-request checkout.

Running the scorecard against the working tree while this cookbook was being developed found three
changed indexed files and zero structural metric deltas. `ctx check --list` simultaneously showed
that the local rules file contained no layers or rules. The score was useful evidence that the
documentation and skill work had not altered indexed code structure; it was not evidence that the
repository had complete architecture-policy coverage.

## Give the workflow to an agent

```text
Review this branch against the merge base. Report the full ctx scorecard, then investigate the
files behind any concentrated complexity, coupling, duplication, or architecture signal. Separate
observations from blocking policy. Do not reject the change for unrelated existing debt, and do not
recommend refactoring solely because a metric increased. Treat exit code 2 as an invalid analysis,
not a clean result.
```

## Next steps

- Use [ctx score](../commands/score.md) for metric definitions and gate expressions.
- Use [ctx check](../commands/check.md) to introduce architecture contracts gradually.
- Use [ctx duplicates](../commands/duplicates.md) to inspect similarity before deciding on reuse.
- Follow the [continuous health recipe](continuous-health.md) to see whether accepted pull-request
  deltas become a sustained repository trend.
