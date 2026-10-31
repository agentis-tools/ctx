---
id: exit-codes
title: Exit codes
sidebar_position: 2
---

# Exit codes

A shell `&&`, a CI step or an agent hook can enforce a ctx gate from the exit code alone, without
parsing output; `--json` gives the details when a tool needs them. An operational error never exits
0, so a broken gate fails instead of passing.

ctx commands use the same codes, the convention of `grep` and most linters:

| Code | Meaning |
|------|---------|
| `0` | Success, nothing to report |
| `1` | The command ran and produced findings (gating commands only) |
| `2` | Operational error: bad arguments, missing index, missing embeddings, bad git ref, IO error |
| `3` | `ctx harness compat` only: the installed binary is older than a hook's required version |

Versions before 0.3.0 exited `1` on operational errors. If a script treats `1` as an error, update it. The exception is `ctx diff` and `ctx review`, which exit 1 when the directory is not a git repository or the revision is invalid, and exit 2 when there are no changes.

## Which commands gate

| Command | Exit behavior |
|---------|---------------|
| [`ctx check`](../commands/check.md) | `1` when any architecture rule is violated |
| [`ctx score --fail-on`](../commands/score.md) | `1` when any `--fail-on` condition is met |
| [`ctx duplicates --fail-on-found`](../commands/duplicates.md) | `1` when any near-duplicate pair is reported |
| [`ctx sql --fail-on-rows`](../commands/sql.md) | `1` when the query returns at least one row |
| [`ctx audit --min-score`](../commands/audit.md) | `1` when the score is below the threshold |
| `ctx harness doctor` | `1` when it finds problems, `0` when healthy |
| [`ctx lsp doctor`](../commands/lsp.md) | `1` when a configured server fails its checks |
| `ctx harness compat --require <semver>` | `3` when the binary is below the required version (generated hooks then fail open) |
| `ctx hotspots`, `ctx similar`, `ctx map` | Informational: `0` unless an operational error (`2`) |
| [`ctx smart`](../commands/smart.md) | `2` when embeddings are missing, incomplete or from another provider, model or dimension (v0.4.0 and earlier exited `0` with empty output) |

## Composing gates

```bash
# A gate is a command whose exit code you honor
ctx index && ctx check --against origin/main

# One composite gate: fold everything into one scorecard
ctx score --against origin/main --fail-on "check_violations>0,new_duplication>0,complexity_delta>=25"
```

Because operational failures exit `2`, not `1`, CI can tell "the gate found problems" (fail the
build, show the findings) from "the gate itself broke" (fail the build, fix the setup).

## See also

- [JSON output](../json-output.md): the `--json` envelope for tools that need the details
- [Quality gates](../integrations/quality-gates.md): wiring the suite into CI and agents
