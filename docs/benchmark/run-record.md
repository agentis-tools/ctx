# Benchmark run record

Every benchmark run in the longitudinal study produces one *run record*: a
single JSON object appended to a JSON Lines file. Records are validated
against [`run-record.schema.json`](./run-record.schema.json)
(JSON Schema draft 2020-12, `$id`
`https://docs.agentis.tools/schemas/run-record-v1.json`).

## Fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `schema_version` | integer (const `1`) | yes | Version of the record schema. Always `1` for records described by this document. |
| `run_id` | string (UUID) | yes | Globally unique identifier for the run. |
| `task_id` | string | yes | Identifier of the benchmark task the run attempted. |
| `arm` | string | yes | Experiment arm. Conventional values: `control` (no gates), `gates` (advisory gates), `gates_blocking` (blocking gates). Not enum-restricted, so new arms need no schema change. |
| `run_index` | integer >= 0 | yes | Zero-based index of the run among the repeats of `(task_id, arm)`. |
| `started_at` | string (RFC 3339 date-time) | yes | When the run started. |
| `finished_at` | string (RFC 3339 date-time) | yes | When the run finished. |
| `model_version` | string | yes | Exact model identifier used by the agent. |
| `ctx_version` | string | yes | Version of ctx installed during the run. |
| `harness_mode` | `"local"` \| `"plugin"` | yes | How the ctx harness was wired into the agent. |
| `gate_config` | object | yes | Effective gate configuration; see below. |
| `gate_config.fail_on` | string or null | yes | Gate failure condition, or `null` when none is configured. |
| `gate_config.blocking` | boolean | yes | Whether gate failures block the agent (`true`) or are advisory (`false`). |
| `gate_config.gate_log_enabled` | boolean | yes | Whether gate evaluations were logged to `.ctx/gate-log.jsonl`. |
| `transcript_path` | string | yes | Path of the full agent transcript, relative to the study data root. |
| `exit_status` | `"success"` \| `"failure"` \| `"timeout"` \| `"error"` | yes | How the run ended. |
| `metrics` | object | yes | Quantitative outcomes; see below. |
| `metrics.score` | object | yes | Final `ctx score` metrics for the run's diff. Exactly the seven keys `complexity_delta`, `fan_out_delta`, `new_duplication`, `check_violations`, `symbols_added`, `symbols_removed`, `files_changed`, all numbers. |
| `metrics.gate_evaluations` | integer >= 0 | yes | Number of gate evaluations performed during the run. |
| `metrics.wall_clock_seconds` | number >= 0 | yes | Total wall-clock duration of the run in seconds. |
| `notes` | string | no | Free-form operator notes. |

The top-level object rejects unknown keys (`additionalProperties: false`).

## Versioning policy

- **Additive changes** (new *optional* fields, looser descriptions) keep
  `schema_version` at its current value and reuse the same `$id`. Readers
  must ignore optional fields they do not know -- but note that because the
  top level is closed, additive changes still require publishing an updated
  schema document before writers emit the new field.
- **Breaking changes** (removing or renaming a field, changing a type,
  making an optional field required) bump `schema_version` (const `2`, ...)
  and publish under a new `$id`
  (`https://docs.agentis.tools/schemas/run-record-v2.json`, ...). Old records
  are never rewritten; analysis code dispatches on `schema_version`.

## Example record

```json
{
  "schema_version": 1,
  "run_id": "1f0c9a2e-7b4d-4c6a-9e2f-3d8b5a1c4e70",
  "task_id": "refactor-duplicates-01",
  "arm": "gates",
  "run_index": 3,
  "started_at": "2026-07-01T14:03:12Z",
  "finished_at": "2026-07-01T14:41:55Z",
  "model_version": "claude-fable-5",
  "ctx_version": "0.9.2",
  "harness_mode": "local",
  "gate_config": {
    "fail_on": "regression",
    "blocking": false,
    "gate_log_enabled": true
  },
  "transcript_path": "runs/refactor-duplicates-01/gates/run-3/transcript.jsonl",
  "exit_status": "success",
  "metrics": {
    "score": {
      "complexity_delta": -1.5,
      "fan_out_delta": 0.0,
      "new_duplication": 0,
      "check_violations": 0,
      "symbols_added": 4,
      "symbols_removed": 2,
      "files_changed": 3
    },
    "gate_evaluations": 12,
    "wall_clock_seconds": 2323.4
  },
  "notes": "One gate evaluation flagged a transient duplication warning."
}
```

## Joining run records to gate logs

When `gate_config.gate_log_enabled` is true, the harness appends one JSON
line per gate evaluation to `.ctx/gate-log.jsonl` inside the task workspace.
Gate-log lines carry their own timestamps (and, where available, a harness
session id). To attribute gate evaluations to a run:

1. Prefer the session id when both sides record one -- it is exact.
2. Otherwise join by run window: a gate-log line belongs to the run whose
   `[started_at, finished_at]` interval contains its timestamp for the same
   task workspace. Runs of the same task never overlap in time on one
   workspace, so the window join is unambiguous; `metrics.gate_evaluations`
   should equal the number of joined lines and serves as a consistency
   check.
