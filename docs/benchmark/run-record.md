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
| `metrics.gate_blocks` | integer >= 0 | no | Number of gate evaluations that blocked the agent. |
| `metrics.gate_block_recovered` | boolean | no | Whether the agent recovered (eventually passed the gate) after at least one block. |
| `metrics.wall_clock_seconds` | number >= 0 | yes | Total wall-clock duration of the run in seconds. |
| `metrics.duplication` | object | no | Production near-duplicate accounting for the run's diff. Pair counts consider only production code: a pair is excluded iff both endpoints' files are under `tests/`; `delta` = final − base (signed -- negative rewards dedup). |
| `metrics.duplication.base_pairs` | integer >= 0 | yes (within `duplication`) | Production near-duplicate pairs at the base commit. |
| `metrics.duplication.final_pairs` | integer >= 0 | yes (within `duplication`) | Production near-duplicate pairs in the run's final tree. |
| `metrics.duplication.delta` | integer (signed) | yes (within `duplication`) | `final_pairs - base_pairs`; negative values reward deduplication. |
| `metrics.duplication.new_pairs` | integer >= 0 | yes (within `duplication`) | Pairs present in the final tree but not at base. |
| `metrics.duplication.removed_pairs` | integer >= 0 | yes (within `duplication`) | Pairs present at base but not in the final tree. |
| `metrics.duplication.threshold` | number | yes (within `duplication`) | Jaccard similarity threshold used to count a pair as a near-duplicate. |
| `metrics.duplication.min_tokens` | integer >= 0 | yes (within `duplication`) | Minimum normalized token count for a symbol to participate in pairing. |
| `agent` | object | no | Agent-side accounting reported by the agent harness (e.g. `claude -p`); see below. |
| `agent.session_id` | string | yes (within `agent`) | Agent session identifier, for exact joins against gate logs and transcripts. |
| `agent.total_cost_usd` | number >= 0 | yes (within `agent`) | Total model cost of the run in USD. |
| `agent.total_tokens` | integer >= 0 | yes (within `agent`) | Total tokens (input + output) consumed by the run. |
| `agent.num_turns` | integer >= 0 | no | Number of agent turns; not guaranteed to be reported by `claude -p`. |
| `acceptance` | object | no | Result of the task's acceptance command -- the ctx-independent functional endpoint of the run; see below. |
| `acceptance.command` | string | yes (within `acceptance`) | Acceptance command that was executed. |
| `acceptance.exit_code` | integer | yes (within `acceptance`) | Exit code of the acceptance command. |
| `acceptance.passed` | boolean | yes (within `acceptance`) | Whether the acceptance command succeeded. |
| `acceptance.duration_seconds` | number >= 0 | yes (within `acceptance`) | Wall-clock duration of the acceptance command in seconds. |
| `normalization` | object | no | Diff-size data: the denominator for per-line deltas; see below. |
| `normalization.lines_added` | integer >= 0 | yes (within `normalization`) | Lines added by the run's final diff. |
| `normalization.lines_removed` | integer >= 0 | yes (within `normalization`) | Lines removed by the run's final diff. |
| `normalization.lines_changed` | integer >= 0 | yes (within `normalization`) | Total lines changed (added + removed) by the run's final diff. |
| `study_id` | string | no | Identifier of the study the run belongs to, so multiple studies can share one record store. |
| `task_seed` | integer | no | Seed used by the task generator to produce this run's task instance. |
| `generator_version` | string | no | Version of the task generator that produced the task instance. |
| `scorer_ctx_version` | string | no | Version of the ctx build used by the offline scorer; `ctx_version` remains the harness build. |
| `retry_attempt` | integer >= 0 | no | Zero-based retry attempt number when a run was retried after an infrastructure error. |
| `max_budget_usd` | number >= 0 | no | Effective per-run budget ceiling in USD: the per-task override when one is configured, otherwise the study default. |
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

The optional `agent`, `acceptance`, and `normalization` objects and the
`study_id`, `task_seed`, `generator_version`, `scorer_ctx_version`,
`retry_attempt`, `metrics.gate_blocks`, and `metrics.gate_block_recovered`
fields arrived additively (per the policy above) for the ctx-bench pilot
runner; `schema_version` remains `1`. The optional `metrics.duplication`
object and `max_budget_usd` field arrived additively for ctx-bench Phase 3;
`schema_version` still remains `1`.

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
    "gate_blocks": 1,
    "gate_block_recovered": true,
    "wall_clock_seconds": 2323.4,
    "duplication": {
      "base_pairs": 7,
      "final_pairs": 4,
      "delta": -3,
      "new_pairs": 0,
      "removed_pairs": 3,
      "threshold": 0.85,
      "min_tokens": 50
    }
  },
  "agent": {
    "session_id": "8b2f4a1c-0d3e-4f6a-b7c8-9e0d1f2a3b4c",
    "total_cost_usd": 1.87,
    "total_tokens": 412034,
    "num_turns": 41
  },
  "acceptance": {
    "command": "cargo test --workspace",
    "exit_code": 0,
    "passed": true,
    "duration_seconds": 148.2
  },
  "normalization": {
    "lines_added": 96,
    "lines_removed": 42,
    "lines_changed": 138
  },
  "study_id": "pilot-2026-07",
  "task_seed": 1337,
  "generator_version": "0.1.0",
  "scorer_ctx_version": "0.9.2",
  "retry_attempt": 0,
  "max_budget_usd": 5.0,
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
