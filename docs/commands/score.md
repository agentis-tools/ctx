# ctx score

Score the quality delta of your changes against a git reference.

## Synopsis

```bash
ctx score [--against <REF>] [--fail-on <EXPR>] [--json]
```

## Description

`ctx score` compares the working tree (plus commits since the merge base with REF) against REF and prints a short scorecard. It answers "did this change make the code better or worse?" with numbers. Score resolves the merge base once and uses that commit for changed paths, baseline file contents, duplication, and architecture checks. Human output shows the resolved commit, and JSON emits it as `data.baseline`. The scorecard reports:

- complexity and fan-out deltas per changed file, baseline against current,
- new near-duplicate function pairs that did not exist at REF,
- [`ctx check`](./check.md) architecture violations, scoped to the same REF,
- symbols added and removed.

Before scoring, ctx refreshes the index incrementally and prints `note: index refreshed (N files reindexed)` to stderr. For the baseline it parses each changed file's content at REF in memory with the indexing parser. Nothing is written to the database, and both sides use the same method, so the deltas compare like with like.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--against <REF>` | Git reference to compare against. The default scores uncommitted changes; use your default branch (`main`/`master`) to score a whole branch or PR | `HEAD` |
| `--fail-on <EXPR>` | Comma-separated conditions `metric OP value` with OP one of `>=`, `<=`, `>`, `<`; exits 1 when any condition holds | none |
| `--json` | Machine-readable output (global flag) | false |

## Metrics

Use these names in `--fail-on`. JSON reports them under `data.metrics`:

| Metric | Meaning |
|--------|---------|
| `complexity_delta` | Sum over changed files of per-function `2*fan_out + same-file fan_in`, current minus baseline |
| `fan_out_delta` | Call edges sourced in changed files, current minus baseline |
| `new_duplication` | Verified near-duplicate pairs (Jaccard >= 0.85, >= 50 tokens, at least one endpoint in a changed file) that did not exist at REF |
| `check_violations` | `ctx check --against REF` violations (0 with a note when `.ctx/rules.toml` is missing) |
| `symbols_added` | Symbols present now but not at REF (matched by file, parent, and name) |
| `symbols_removed` | Symbols present at REF but not now |
| `files_changed` | Changed source files that were scored |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Clean, or no `--fail-on` given (the command is then informational) |
| 1 | At least one `--fail-on` condition was met (the failed conditions are listed on stderr) |
| 2 | Operational error (not a git repo, bad reference, malformed `--fail-on`, invalid rules file) |

## Examples

### Score uncommitted changes

```bash
ctx score
```

Output:
```
Score vs HEAD (baseline 0123456) (1 file changed)

  complexity_delta        3 → 6      ▲ +3
  fan_out_delta           1 → 2      ▲ +1
  new_duplication         0          =
  check_violations        0          =  (no rules file)
  symbols_added           1          ▲
  symbols_removed         0          =

Notes:
  - fan_in approximated as same-file callers for baseline comparability
```

### Score a branch or PR

```bash
ctx score --against main
```

### CI or agent quality gate

```bash
ctx score --against main --fail-on "check_violations>0,new_duplication>0"
echo $?  # 0 = passed, 1 = a condition was met, 2 = error
```

### JSON output

```bash
ctx score --against main --json
```

```json
{
  "ctx_version": "0.4.0",
  "command": "score",
  "generated_at": "2026-07-09T12:00:00Z",
  "data": {
    "against": "main",
    "baseline": "0123456789abcdef0123456789abcdef01234567",
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

## Gate logging

Set the `CTX_GATE_LOG` environment variable and every `ctx score` run appends one record of the gate evaluation to a local log. Use it to keep a history of gate outcomes, for example from the Claude Code Stop hook, without changing what the command does. Logging is opt-in and local. ctx ships no telemetry.

| `CTX_GATE_LOG` value | Effect |
|----------------------|--------|
| unset, empty, or `0` | Logging disabled (the default) |
| `1` or `true` | Append to `.ctx/gate-log.jsonl` under the repo root |
| anything else | The log path (relative paths are joined to the repo root) |

The log is JSON Lines: one JSON object per line, one line per evaluation. It is not the standard `--json` envelope (there is no `command` or `data` wrapper), and ctx writes it whether or not you pass `--json`:

```json
{"schema_version":1,"ts":"2026-07-10T19:25:43.893434Z","ctx_version":"0.4.0","source":"score","against":"HEAD","fail_on":"new_duplication>0","metrics":{"check_violations":0,"complexity_delta":0,"fan_out_delta":0,"files_changed":0,"new_duplication":0,"symbols_added":0,"symbols_removed":0},"failed_conditions":[],"outcome":"pass","blocking":false,"session_id":null}
```

| Field | Description |
|-------|-------------|
| `schema_version` | Version of the line format (currently `1`) |
| `ts` | Evaluation time, RFC 3339 UTC |
| `ctx_version` | The ctx version that evaluated the gate |
| `source` | The command that evaluated the gate (`"score"`) |
| `against` | The git reference the score was computed against |
| `fail_on` | The raw `--fail-on` expression, or `null` when none was given |
| `metrics` | The same seven-key metrics object the `--json` payload emits under `metrics` |
| `failed_conditions` | Rendered `--fail-on` conditions that fired (empty on pass) |
| `outcome` | `"pass"` or `"fail"` |
| `blocking` | Whether blocking mode was requested (`CTX_GATE_BLOCKING=1`; see [ctx harness](./harness.md)) |
| `session_id` | Claude Code session id (`CLAUDE_SESSION_ID`), or `null` |

Logging is best-effort. An IO failure prints a warning to stderr and never changes the exit code. Query the log with standard JSONL tools:

```bash
jq -r 'select(.outcome == "fail") | [.ts, .failed_conditions[]] | @tsv' .ctx/gate-log.jsonl
```

## Caveats

- Fan-in is approximated. ctx parses the baseline side in isolation, so it cannot see cross-file callers there. It therefore counts same-file callers as fan-in on both sides, which keeps the delta comparable, and every run prints a note about it.
- ctx matches symbols across sides by `(file, parent, name)`, never by symbol id, because ids embed line numbers that shift. A renamed function counts as one removal plus one addition.
- `new_duplication` inherits the [`ctx duplicates`](./duplicates.md) caveats: idiomatic boilerplate can look structurally similar. The generated harness Stop hook leaves it out of the blocking gate until you calibrate it.
- ctx does not score changed files that ignore patterns exclude from the index.

## See also

- [Quality gates](../integrations/quality-gates.md): wiring `ctx score` into CI and Claude Code hooks
- [ctx check](./check.md)
- [ctx duplicates](./duplicates.md)
- [JSON output](../json-output.md)
