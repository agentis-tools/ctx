---
id: score
title: ctx score
sidebar_position: 7
---

# ctx score

Score the quality delta of your changes against a git reference.

## Synopsis

```bash
ctx score [--against <REF>] [--fail-on <EXPR>] [--json]
```

## Description

The `score` command compares the working tree (plus commits since the merge base with REF) against REF and prints a compact scorecard. It answers "did this change make the code better or worse?" with numbers:

- **Complexity and fan-out deltas** - per changed file, baseline vs. current
- **New duplication** - near-duplicate function pairs that did not exist at REF
- **Architecture violations** - the [`ctx check`](./check.md) rules, scoped to the same REF
- **Symbols added / removed** - API surface churn

The index is refreshed **incrementally** before scoring (a `note: index refreshed (N files reindexed)` notice goes to stderr). Baseline metrics are computed by parsing each changed file's content at REF **in memory** with the same parser used for indexing - nothing is written to the database, and both sides use the same method so the deltas are honest.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--against <REF>` | Git reference to compare against. The default scores uncommitted changes; use your default branch (`main`/`master`) to score a whole branch or PR | `HEAD` |
| `--fail-on <EXPR>` | Comma-separated conditions `metric OP value` with OP one of `>=`, `<=`, `>`, `<`; exit 1 when any condition holds | none |
| `--json` | Machine-readable output (global flag) | false |

## Metrics

These names are used verbatim in `--fail-on` and under `data.metrics` in JSON:

| Metric | Meaning |
|--------|---------|
| `complexity_delta` | Sum over changed files of per-function `2*fan_out + same-file fan_in`, current minus baseline |
| `fan_out_delta` | Call edges sourced in changed files, current minus baseline |
| `new_duplication` | Verified near-duplicate pairs (Jaccard >= 0.85, >= 50 tokens, at least one endpoint in a changed file) that did not exist at REF |
| `check_violations` | `ctx check --against REF` violations (0 with a note when `.ctx/rules.toml` is missing) |
| `symbols_added` | Symbols present now but not at REF (matched by file, parent, and name) |
| `symbols_removed` | Symbols present at REF but not now |
| `files_changed` | Changed source files that were scored |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Clean (or no `--fail-on` given: the command is informational) |
| 1 | At least one `--fail-on` condition was met (the failed conditions are listed on stderr) |
| 2 | Operational error (not a git repo, bad reference, malformed `--fail-on`, invalid rules file) |

## Examples

### Score Uncommitted Changes

```bash
ctx score
```

Output:
```
Score vs HEAD (1 file changed)

  complexity_delta        3 → 6      ▲ +3
  fan_out_delta           1 → 2      ▲ +1
  new_duplication         0          =
  check_violations        0          =  (no rules file)
  symbols_added           1          ▲
  symbols_removed         0          =

Notes:
  - fan_in approximated as same-file callers for baseline comparability
```

### Score a Branch or PR

```bash
ctx score --against main
```

### CI / Agent Quality Gate

```bash
ctx score --against main --fail-on "check_violations>0,new_duplication>0"
echo $?  # 0 = passed, 1 = a condition was met, 2 = error
```

### JSON Output

```bash
ctx score --against main --json
```

```json
{
  "ctx_version": "0.2.1",
  "command": "score",
  "generated_at": "2026-07-09T12:00:00Z",
  "data": {
    "against": "main",
    "files_changed": 1,
    "metrics": {
      "complexity_delta": 3,
      "fan_out_delta": 1,
      "new_duplication": 0,
      "check_violations": 0,
      "symbols_added": 1,
      "symbols_removed": 0,
      "files_changed": 1
    },
    "check_violations_note": "no rules file",
    "per_file": [
      {
        "path": "src/a.rs",
        "complexity_baseline": 3,
        "complexity_current": 6,
        "fan_out_baseline": 1,
        "fan_out_current": 2,
        "symbols_added": 1,
        "symbols_removed": 0
      }
    ],
    "failed_conditions": [],
    "notes": ["fan_in approximated as same-file callers for baseline comparability"]
  }
}
```

## Caveats

- **Fan-in approximation:** the baseline side is parsed in isolation, so cross-file callers are unknowable there. Fan-in is therefore counted as *same-file* callers on **both** sides, keeping the delta comparable. This is surfaced as a note in every run.
- Symbols are matched across sides by `(file, parent, name)` - never by symbol id, since ids embed line numbers that shift. A renamed function counts as one removal plus one addition.
- `new_duplication` inherits the [`ctx duplicates`](./duplicates.md) caveats: Solidity functions are not fingerprinted, and idiomatic boilerplate can look structurally similar.
- Changed files that are excluded from the index (ignore patterns) are not scored.

## See Also

- [Quality Gates](../integrations/quality-gates.md) - wiring `ctx score` into CI and Claude Code hooks
- [ctx check](./check.md)
- [ctx duplicates](./duplicates.md)
- [JSON Output](../json-output.md)
