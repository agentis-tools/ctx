---
id: gate-recovery
title: Recover when a ctx gate blocks a legitimate change
sidebar_position: 17
---

# Recover when a ctx gate blocks a legitimate change

A blocked gate can mean three very different things: ctx found a real contract violation, the
policy does not describe the intended exception, or the analysis is operationally invalid. Recovery
is the process of distinguishing those cases without deleting the rule, raising a threshold blindly,
or teaching an agent to ignore the gate.

This workflow was exercised against ctx 0.3.5 in its own repository on 2026-07-14. The local
starter policy parsed successfully with zero active rules; a missing rules file and an invalid git
reference both exited `2`; incremental reindexing cleared the stale-index diagnostic while
`ctx harness doctor` continued to report a separate stale-template warning. Those distinct results
are why recovery must classify before acting.

## Quickest version

```bash
ctx check --list
BASE="$(git merge-base HEAD origin/main)"
ctx check --against "$BASE" --json
status=$?
ctx harness doctor --json || true
```

- `status == 1`: inspect a valid finding.
- `status == 2`: repair the analysis before interpreting the output.
- `status == 0`: confirm that active rules actually cover the intended boundary.

## 1. Preserve the evidence

Capture the exact command, ctx version, base SHA, head SHA, exit code, stderr, JSON, and relevant
policy revision. Do not immediately rerun with a weaker threshold: that destroys the best account
of what blocked the change.

```bash
ctx --version
git rev-parse "$BASE" HEAD
ctx check --list --json

set +e
ctx check --against "$BASE" --json > check.json 2> check.stderr
status=$?
set -e
printf 'ctx check exit: %s\n' "$status"
```

Use the [canonical exit-code interpretation](concepts#exit-codes-are-part-of-the-integration-api).
An operational error is neither a clean run nor a policy finding.

## 2. Validate the policy before debating the code

`ctx check --list` proves that the file parses and shows layer membership and active rules. Confirm:

- the expected rules file was loaded;
- path globs match indexed paths rather than repository assumptions;
- layers do not overlap;
- the rule covers the intended boundary;
- the comparison base represents the branch decision;
- a zero count is not merely an inert or missing policy.

The `--against` form scopes findings to changed endpoints. It does not create a permanent baseline
or declare every pre-existing violation acceptable.

## 3. Verify the reported relationship

For a dependency finding, inspect both endpoints and the edge:

```bash
ctx source <from-symbol> --file <from-file>
ctx source <to-symbol> --file <to-file>
ctx query deps <from-symbol> --file <from-file> --depth 1 --json
rg -n '<import-or-symbol-name>' <from-file> <to-file>
```

Reject same-name graph expansion that source does not support. Also look for relationships ctx may
miss—dynamic dispatch, macros, reflection, generated registration, persisted names, or external
consumers. Static evidence can be wrong in either direction.

For a metric limit, inspect the actual symbol, fan-in, fan-out, responsibility, and change pressure.
A coherent parser or dispatcher may deserve a reviewed exception; a new unrelated responsibility
may deserve a code change.

## 4. Correct the right layer

Choose one outcome and record why:

| Finding | Appropriate correction |
|---|---|
| Source violates an agreed boundary | change the dependency or design |
| Rule glob or layer model is wrong | correct the reviewed policy and explain the intent |
| A metric exception is intentional | add the narrowest supported exclusion with ownership and rationale |
| Relationship is a reproducible ctx false positive | minimize a fixture and report it; do not invent a broad policy exception |
| Evidence is incomplete | gather source, tests, runtime, or contract evidence before deciding |

Only limit rules support an `exclude` path list. Forbidden, allowed-dependent, and
no-new-dependent rules do not have a per-edge suppression field. If the architecture has a real
exception, model a more accurate layer boundary or change the code; do not imply that ctx supports
an override it does not provide.

Never edit `.ctx/rules.toml` solely to make one run green. Policy changes are compatibility and
ownership decisions and should receive the same review as the rule's introduction.

## 5. Rebuild stale evidence in increasing order of cost

When source and the index disagree:

```bash
ctx index
ctx harness doctor --json
```

If the same relationship is still inconsistent:

```bash
ctx index --force
ctx harness doctor --json
```

Then repeat the smallest disambiguated query and inspect source. `--force` rebuilds the code index;
it does not repair an invalid rules file, refresh generated harness templates, enable optional
features, fix a bad git reference, or make unsupported dynamic behavior statically visible.

The ctx-on-ctx trial demonstrated that distinction: incremental indexing removed the
`index_stale` warning, but `harness doctor` still exited `1` because older generated hook templates
were a separate warning. Treat each diagnostic independently.

## 6. Recover an agent without creating a bypass habit

When a stop-time agent gate blocks:

1. keep the failed command and evidence in the task record;
2. classify finding versus operational error;
3. inspect the exact source or policy boundary;
4. make one code or reviewed-policy correction;
5. reindex and rerun the identical gate;
6. widen to the repository's required validation.

Do not tell the agent to skip the hook, remove `--fail-on`, edit the policy, or reinterpret exit `2`
as success. If the installed ctx version does not meet the harness floor, update through the
repository's approved installation path and regenerate integrations deliberately.

## What worked, and what did not

| Technique | Verified result | Limitation observed |
|---|---|---|
| `ctx check --list --json` | Parsed the starter policy and showed zero active layers and rules | Successful parsing does not mean meaningful coverage |
| Missing rules path | Exited `2` with an operational diagnostic | No policy judgment is possible from that run |
| Invalid `--against` ref | Exited `2` | A bad base cannot be treated as zero findings |
| `ctx index` | Refreshed two changed indexed files and preserved 110 files / 2,161 symbols | Indexed totals still exclude unsupported and ignored repository files |
| `ctx harness doctor --json` | Distinguished stale index from stale generated templates | Exit `1` aggregates warnings; inspect individual diagnostic codes |
| `ctx index --force` | Provides a full index rebuild path | It cannot correct policy, version, feature, or dynamic-analysis limitations |

The recorded totals are an illustration from ctx commit `0794eb1` on 2026-07-14, not a stable
product specification. Your values will differ.

## Give the workflow to an agent

```text
Recover this ctx gate without weakening it. Preserve the exact command, versions, refs, exit code,
stderr, JSON, and parsed policy. Treat exit 1 as findings and exit 2 as invalid analysis. Verify the
reported relationship in disambiguated source and exact search, classify it as a source violation,
policy-model error, intentional supported exception, reproducible ctx defect, or insufficient
evidence, and correct only that layer. Reindex incrementally, use --force only for a persistent
index disagreement, rerun the identical gate, and then run the repository's wider validation. Do
not delete rules, raise thresholds blindly, or turn operational failure into success.
```

## Next steps

- Review [shared cookbook concepts](concepts) for bases, enforcement levels, and exit codes.
- Use [intentional complexity](intentional-complexity) before excluding a metric limit.
- Use [architecture drift](architecture-drift) before changing a dependency boundary.
- Use [untrusted CI](untrusted-ci) when the blocked result came from fork analysis.
