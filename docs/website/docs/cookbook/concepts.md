---
id: concepts
title: Concepts shared by every recipe
sidebar_position: 2
---

# Concepts shared by every recipe

These rules keep cookbook workflows consistent. Individual recipes apply them to a concrete
problem rather than redefining them.

## Evidence is not a verdict

A metric, ranking, graph edge, or zero count is a reason to ask a better question. It is not proof
that code is healthy or unhealthy. Verify important relationships in source, correlate trends with
repository growth and change history, and state what the available evidence cannot establish.

## Choose the comparison that matches the decision

| Decision | Comparison |
|---|---|
| Inspect only uncommitted work | `--against HEAD` |
| Review a branch or pull request | the merge base with the target branch |
| Compare releases | immutable tag commit SHAs |
| Understand direction over time | multiple snapshot partitions with recorded provenance |

For a local branch:

```bash
git fetch origin main
BASE="$(git merge-base HEAD origin/main)"
ctx score --against "$BASE"
```

In CI, use the forge-provided base SHA and fetch it explicitly. Do not derive trusted metadata from
pull-request code.

## Interpret gate outcomes consistently

Use three policy levels:

| Level | Meaning | Typical response |
|---|---|---|
| Report | Context worth seeing, not a judgment | publish the evidence |
| Review | A new condition needs interpretation | inspect and record a decision |
| Block | An explicit, reviewed contract was violated | fail the gate or correct the configuration |

Complexity, coupling, churn, and duplication usually begin as Report or Review signals. Blocking
is appropriate for operational failure and deliberately adopted contracts, not for every rising
metric.

## Exit codes are part of the integration API

| Code | Meaning |
|---|---|
| `0` | Command completed without findings |
| `1` | Command completed and produced findings |
| `2` | Operational error; the analysis is invalid |
| `3` | `ctx harness compat` version requirement was not met |

Never turn code `2` into a clean report. See the canonical [exit-code reference](../reference/exit-codes)
for command-specific behavior.

## Indexed totals are not repository totals

ctx reports the files, symbols, and relationships represented in its index. Git-ignored,
unsupported, generated, binary, or deliberately excluded files may not be present. A zero result
can mean no finding, no applicable policy, incomplete coverage, or stale evidence. Use `ctx index`,
`ctx check --list`, manifests, exact search, and source inspection to distinguish those cases.

## Worked numbers are illustrations, not specifications

Cookbook measurements are pinned to the named ctx commit or release and the date stated in the
recipe. Your repository and a newer ctx build will produce different totals. The durable part of a
recipe is the evidence loop, command contract, and interpretation method—not the sample number.
