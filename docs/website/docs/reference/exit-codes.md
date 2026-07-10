---
id: exit-codes
title: Exit codes
sidebar_position: 2
---

# Exit codes

Exit codes **are** ctx's integration API. A shell `&&`, a CI step, or an agent hook can enforce a
gate without parsing any output — the process exit code carries the verdict, and `--json` is there
when a tool wants the details. Crucially, an operational error never masquerades as a clean run: a
broken gate fails loudly instead of silently "passing".

Every command in the quality suite follows the same convention (the same one `grep` and most linters
use):

| Code | Meaning |
|------|---------|
| `0` | Success — nothing to report |
| `1` | Ran successfully, but produced **findings** (used by the gating commands) |
| `2` | **Operational error** — bad arguments, missing index, bad git ref, IO error |
| `3` | Reserved for `ctx harness compat` — the installed binary is older than a hook's required version floor |

:::note Breaking change
Operational errors previously exited with code `1`. They now exit with code `2`; code `1` is
reserved exclusively for "ran fine, found issues". Update any script that treated `1` as "error".
:::

## Which commands gate

| Command | Exit behavior |
|---------|---------------|
| [`ctx check`](../commands/check.md) | `1` when any architecture rule is violated |
| [`ctx score --fail-on`](../commands/score.md) | `1` when any `--fail-on` condition is met |
| [`ctx duplicates --fail-on-found`](../commands/duplicates.md) | `1` when any near-duplicate pair is reported |
| [`ctx audit --min-score`](../commands/audit.md) | `1` when the score is below the threshold |
| `ctx hotspots`, `ctx similar`, `ctx map` | Informational — `0` unless an operational error (`2`) |
| `ctx harness compat --require <semver>` | `3` when the binary is below the required floor (hooks fail **open**) |
| `ctx harness doctor` | `1` when it finds problems, `0` when healthy |

## Composing gates

```bash
# A gate is just a command whose exit code you honor
ctx index && ctx check --against origin/main

# The composite gate: fold everything into one scorecard
ctx score --against origin/main --fail-on "check_violations>0,new_duplication>0,complexity_delta>=25"
```

Because operational failures are code `2` (not `1`), CI distinguishes "the gate found problems"
(fail the build, show the findings) from "the gate itself broke" (fail the build, fix the setup) —
they are never confused.

## See also

- [JSON output](../json-output.md) — the `--json` envelope for tools that want the details
- [Quality gates](../integrations/quality-gates.md) — wiring the suite into CI and agents
